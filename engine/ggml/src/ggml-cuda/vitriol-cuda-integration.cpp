#include "vitriol-cuda-integration.h"
#include "vitriol-buffer.h"
#include "ggml-cuda.h"
#include "ggml-backend.h"
#include <stdio.h>
#include <stdlib.h>
#include <cerrno>
#include <string.h>

/* ── Rust streaming backend (optional, VITRIOL_RUST=1) ─────────────
 * When built with VITRIOL_RUST_LIB and enabled via env, the LRU/store hot
 * path is implemented by the vitriol-stream crate (safe Rust). The C++
 * implementation below remains the default path. See
 * .opencode/plans/vitriol-stream-rust-hybrid-2026-09-25.md. */
/* CUdeviceptr-sized unsigned int; matches Rust usize on this ABI. */
typedef unsigned long long usize_t_rs;

extern "C" {
usize_t_rs vs_lru_ensure(const void * tensor_base, int expert_idx,
                         const void * expert_data, size_t expert_size,
                         void * compute_stream);
void vs_lru_mark_compute_done(usize_t_rs vram_ptr, void * stream);
int  vs_bind_tensor_named(void * tensor_base, const char * name,
                          size_t tensor_bytes, size_t row_bytes);
int  vs_open_store(const char * path);
int  vs_init(void);
void vs_print_stats(void);
}

bool vitriol_rust_enabled(void) {
    static const bool enabled = getenv("VITRIOL_RUST") != nullptr;
    return enabled;
}

#include <cstring>
#include <unordered_map>
#include <set>
#include <map>
#include <array>
#include <list>
#include <mutex>
#include <algorithm>
#include <atomic>
#include <ctime>
#include <vector>
#include <cuda.h>
#include <cuda_runtime.h>
#include <sys/mman.h>

#include "vitriol-store.h"

vitriol_config_t g_vitriol_config;

enum class store_source {
    gguf,
    device,
};
static store_source g_store_source = store_source::device;

/* Store backing: host-only (this file is g++-compiled; header stays nvcc-safe).
 * Path lives here, not in vitriol_config_t (C++20 ceiling on nvcc headers). */
static char g_store_path[256] = {0};

const char * vitriol_store_path(void) { return g_store_path[0] ? g_store_path : nullptr; }

static struct vitriol_config_init {
    vitriol_config_init() {
        memset(&g_vitriol_config, 0, sizeof(g_vitriol_config));
        g_vitriol_config.mode = VITRIOL_MODE_DISABLED;
        g_vitriol_config.prefetch_ahead = 2;
        g_vitriol_config.static_layers = 15;
        g_vitriol_config.window_size_mb = 2048;
        g_vitriol_config.use_double_buffer = true;
        g_vitriol_config.buffer_count = 2;
        g_vitriol_config.pin_tensors_per_layer = 2;
    }
} s_vitriol_config_init;

/* ── LRU Cache ──────────────────────────────────────────────────── */

#define VITRIOL_LRU_POOL_SIZE  (2048ULL * 1024 * 1024)  // default 2 GB VRAM pool (env VITRIOL_LRU_MB overrides)
#define VITRIOL_LRU_MAX_SLOTS  65536

/* Everything below is per-CUDA-device. With a layer split across two GPUs
 * (e.g. GTX 1070 Ti + RTX 3060), each device owns the expert tensors of its
 * layers, so each device needs its own LRU pool, DMA stream and slot events.
 * A single global pool/stream would route GPU1's expert DMA into GPU0's VRAM
 * and hand GPU1 kernels a device-0 pointer → invalid memory access. */
static CUdeviceptr g_lru_pool[GGML_CUDA_MAX_DEVICES];
static size_t      g_lru_pool_size[GGML_CUDA_MAX_DEVICES];
static size_t      g_lru_slot_size[GGML_CUDA_MAX_DEVICES];
static int         g_lru_num_slots[GGML_CUDA_MAX_DEVICES];

/* Composite key: (device, tensor_base_address, expert_idx) prevents
 * cross-layer collisions where expert 0 of layer 1 != expert 0 of layer 2,
 * and keeps cache entries from different GPUs fully disjoint. */
struct LRUKey {
    int       device;
    uintptr_t tensor_base;
    int       expert_idx;
    bool operator==(const LRUKey &o) const {
        return device == o.device && tensor_base == o.tensor_base && expert_idx == o.expert_idx;
    }
};

struct LRUKeyHash {
    size_t operator()(const LRUKey &k) const {
        return (size_t)(k.tensor_base * 2654435761U) ^ (size_t)k.expert_idx ^ (size_t)k.device;
    }
};

/* Per-device LRU bookkeeping. These were file-scope globals while pools,
 * slot sizes and slot counts are per-device arrays; any future second pool
 * (GPU 1) would then mix devices in map.size() and victim scans. The slots
 * a compute stream is still reading are tracked in g_lru_compute_pending
 * and excluded from victim selection. */
static std::unordered_map<LRUKey, int, LRUKeyHash> g_lru_map[GGML_CUDA_MAX_DEVICES];
static std::list<LRUKey> g_lru_order[GGML_CUDA_MAX_DEVICES];
/* Next never-allocated slot index per device. Slots are only retired by
 * eviction (whose slot is immediately reused by the evicting key), so a
 * fresh slot must come from this monotonic counter - NOT from
 * lru_map.size(), which drops below num_slots after the first eviction
 * and would alias a new key onto a still-occupied slot. */
static int g_lru_next_fresh_slot[GGML_CUDA_MAX_DEVICES];
/* Stage 4.5 hot-set protection: last-use ms timestamp per slot. Slots
 * touched within VITRIOL_LRU_HOT_MS are skipped by victim selection while
 * colder candidates exist, so a cold-expert burst cannot flush the hot set. */
static std::vector<int64_t> g_lru_last_use[GGML_CUDA_MAX_DEVICES];
static std::vector<bool> g_lru_compute_pending[GGML_CUDA_MAX_DEVICES];
static std::mutex        g_lru_mtx;

/* Dedicated stream for async DMA; per-slot events for sync.
 * One stream + event array per device. */
static CUstream  g_lru_stream[GGML_CUDA_MAX_DEVICES];
static CUevent *g_lru_slot_events[GGML_CUDA_MAX_DEVICES];
static std::mutex g_lru_init_mtx;

static struct LRUStats {
    unsigned long long hits;
    unsigned long long misses;
    unsigned long long evictions;
} g_lru_stats;

/* ── Expert Pinning Table ──────────────────────────────────────────
 * Maps tensor_base address → deduplicated pinned VRAM buffer.
 * Each entry covers ALL experts of that tensor (contiguous, entire tensor). */
static std::unordered_map<uintptr_t, CUdeviceptr> g_pin_map;
static std::mutex          g_pin_mtx;
static size_t              g_pinned_bytes = 0;

/* Monolithic VRAM pool per device for all pinned tensors.
 * Allocated once on first pin for a given device; subdivided per tensor. */
static CUdeviceptr g_pin_pool[GGML_CUDA_MAX_DEVICES];
static size_t      g_pin_pool_offset[GGML_CUDA_MAX_DEVICES];
static size_t      g_pin_pool_total[GGML_CUDA_MAX_DEVICES];
static bool        g_pin_pool_failed[GGML_CUDA_MAX_DEVICES];

/* ── Expert Output Cache (approximate) ───────────────────────────── */

#define VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER 256
#define VITRIOL_MAX_CACHE_LAYERS            128

static struct {
    uintptr_t tensor_base;
    int       expert_id;
    float    *data_dev;   // GPU buffer, n_embd floats
    bool      valid;
} g_output_cache[VITRIOL_MAX_CACHE_LAYERS][VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER];

static bool   g_output_cache_initialized = false;
static size_t g_output_cache_n_embd      = 0;
static int    g_output_cache_n_layers    = 0;
static CUdeviceptr g_output_cache_pool[GGML_CUDA_MAX_DEVICES];
static size_t g_output_cache_pool_size[GGML_CUDA_MAX_DEVICES];

static struct {
    unsigned long long hits;
    unsigned long long misses;
} g_output_cache_stats;

static int vitriol_current_device(void);

void vitriol_output_cache_init(int n_layers, size_t n_embd) {
    if (g_output_cache_initialized) return;

    if (n_layers > VITRIOL_MAX_CACHE_LAYERS) n_layers = VITRIOL_MAX_CACHE_LAYERS;

    // Allocate a per-device VRAM pool: n_layers * n_experts * n_embd * sizeof(float)
    size_t entry_size = n_embd * sizeof(float);
    size_t pool_size  = (size_t)n_layers * VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER * entry_size;

    int dev = vitriol_current_device();
    cudaSetDevice(dev);
    CUresult err = cuMemAlloc(&g_output_cache_pool[dev], pool_size);
    if (err != CUDA_SUCCESS) {
        fprintf(stderr, "VITRIOL: output cache pool alloc %zu MB failed (%d)\n",
                pool_size / 1024 / 1024, (int)err);
        return;
    }

    // Initialize all entries
    memset(g_output_cache, 0, sizeof(g_output_cache));

    g_output_cache_pool_size[dev] = pool_size;
    g_output_cache_n_embd    = n_embd;
    g_output_cache_n_layers  = n_layers;
    g_output_cache_initialized = true;

    // Device pointers are assigned per entry at store time (the pool is
    // per-device, so a fixed bulk assignment can only describe one GPU).

    if (g_vitriol_config.verbose)
        printf("VITRIOL: output cache initialized: %d layers x %d experts x %zu bytes = %zu MB (GPU %d)\n",
               n_layers, VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER, entry_size,
               pool_size / 1024 / 1024, dev);
}

// Forward declarations
static int get_layer_index(uintptr_t tensor_base);
static void detect_token_boundary(int layer_idx);

const float * vitriol_output_cache_lookup(
    const void *tensor_base,
    int         expert_id)
{
    if (!g_output_cache_initialized) return NULL;

    // Find layer index using existing predictor infrastructure
    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0 || layer_idx >= g_output_cache_n_layers) return NULL;

    auto & entry = g_output_cache[layer_idx][expert_id];
    if (entry.valid &&
        entry.tensor_base == (uintptr_t)tensor_base &&
        entry.expert_id == expert_id) {
        g_output_cache_stats.hits++;
        return entry.data_dev;
    }

    g_output_cache_stats.misses++;
    return NULL;
}

void vitriol_output_cache_store(
    const void    *tensor_base,
    int            expert_id,
    const float   *output_data,
    size_t         n_embd,
    CUstream       stream)
{
    if (!g_output_cache_initialized) {
        // Lazy init: assume up to 128 layers, infer n_embd from data
        vitriol_output_cache_init(128, n_embd);
        if (!g_output_cache_initialized) return;
    }

    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0 || layer_idx >= g_output_cache_n_layers) return;

    if (n_embd != g_output_cache_n_embd) return;

    int dev = vitriol_current_device();
    if (g_output_cache_pool[dev] == 0) {
        vitriol_output_cache_init(g_output_cache_n_layers, n_embd);
        if (g_output_cache_pool[dev] == 0) return;
    }

    auto & entry = g_output_cache[layer_idx][expert_id];

    size_t entry_size = n_embd * sizeof(float);
    entry.data_dev = (float *)(g_output_cache_pool[dev] +
        ((size_t)layer_idx * VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER + (size_t)expert_id) * entry_size);
    entry.tensor_base = (uintptr_t)tensor_base;
    entry.expert_id   = expert_id;
    entry.valid       = true;

    // D2D copy from compute buffer to cache slot (same device)
    size_t copy_size = n_embd * sizeof(float);
    CUresult r = cuMemcpyDtoDAsync(
        (CUdeviceptr)entry.data_dev,
        (CUdeviceptr)output_data,
        copy_size,
        stream);
    if (r != CUDA_SUCCESS) {
        entry.valid = false;
    }
}

void vitriol_output_cache_advance_token(void) {
    if (!g_output_cache_initialized) return;

    // On token boundary, invalidate ALL entries from the previous token
    // because outputs are token-dependent
    for (int l = 0; l < g_output_cache_n_layers; l++) {
        for (int e = 0; e < VITRIOL_MAX_CACHE_EXPERTS_PER_LAYER; e++) {
            g_output_cache[l][e].valid = false;
        }
    }
}

void vitriol_output_cache_print_stats(void) {
    uint64_t total = g_output_cache_stats.hits + g_output_cache_stats.misses;
    float hr = (total > 0) ? 100.0f * (float)g_output_cache_stats.hits / (float)total : 0.0f;
    printf("=== VITRIOL Output Cache ===\n");
    printf("Enabled: %s\n", g_vitriol_config.output_cache ? "yes" : "no");
    printf("Initialized: %s\n", g_output_cache_initialized ? "yes" : "no");
    printf("Layers: %d, n_embd: %zu\n", g_output_cache_n_layers, g_output_cache_n_embd);
    printf("Hits: %llu\n", g_output_cache_stats.hits);
    printf("Misses: %llu\n", g_output_cache_stats.misses);
    printf("Hit Rate: %.2f%%\n", hr);
    printf("==============================\n");
}

static bool lru_init_pool(size_t min_expert_size, int device);
static bool lru_ensure_stream(int device);
/* Forward decl so the output cache (early in the file) can resolve the
 * current device before the helper's definition below. */
static int vitriol_current_device(void);

/* Current CUDA device index for this thread, clamped to a sane range.
 * The VITRIOL paths are entered from ggml-cuda kernels running on whatever
 * device owns the layer being computed — exactly the device whose LRU pool /
 * streams / events must be used. Self-contained (cudaGetDevice), no dependency
 * on ggml internals. */
static int vitriol_current_device(void) {
    int dev = 0;
    cudaGetDevice(&dev);
    if (dev < 0) dev = 0;
    if (dev >= GGML_CUDA_MAX_DEVICES) dev = GGML_CUDA_MAX_DEVICES - 1;
    return dev;
}

/* VITRIOL_LRU_STRICT=1: block the CPU until each expert H2D has completed
 * before returning from the LRU. Diagnostic toggle for ordering bugs in the
 * streaming path; leaves the normal async path untouched. */
static bool vitriol_lru_strict(void) {
    static const bool strict = getenv("VITRIOL_LRU_STRICT") != nullptr;
    return strict;
}

static unsigned long long vitriol_fnv1a(const void * p, size_t n) {
    const unsigned char * b = (const unsigned char *) p;
    /* FNV-1a 64-bit offset basis. NOTE: this constant was truncated
     * (1469598103934665603) until 2026-09-25; hashes recorded in session
     * logs/plans dated before that use the old basis and are internally
     * consistent but are NOT standard FNV-1a. */
    unsigned long long h = 14695981039346656037ULL;
    for (size_t i = 0; i < n; i++) {
        h ^= b[i];
        h *= 1099511628211ULL;
    }
    return h;
}

/* ── Initialization ──────────────────────────────────────────────── */

static void vitriol_cuda_cleanup_vram(void);

void vitriol_cuda_init(void) {
    const char* mode_env = getenv("VITRIOL_MODE");
    if (mode_env) {
        if (strcmp(mode_env, "off") == 0 || strcmp(mode_env, "disabled") == 0)
            g_vitriol_config.mode = VITRIOL_MODE_DISABLED;
        else if (strcmp(mode_env, "sync") == 0)
            g_vitriol_config.mode = VITRIOL_MODE_SYNC;
        else if (strcmp(mode_env, "async") == 0)
            g_vitriol_config.mode = VITRIOL_MODE_ASYNC;
        else if (strcmp(mode_env, "stream") == 0)
            g_vitriol_config.mode = VITRIOL_MODE_STREAM;
    }

    const char* verbose_env = getenv("VITRIOL_VERBOSE");
    if (verbose_env && strcmp(verbose_env, "1") == 0)
        g_vitriol_config.verbose = true;

    const char* pf_env = getenv("VITRIOL_PREDICTIVE_PREFETCH");
    if (pf_env && strcmp(pf_env, "1") == 0)
        g_vitriol_config.async_prefetch = true;

    const char* disk_env = getenv("VITRIOL_DISK_OFFLOAD");
    if (disk_env && strcmp(disk_env, "1") == 0)
        g_vitriol_config.disk_offload = true;

    /* Store backing: VITRIOL_STORE=1 + VITRIOL_STORE_PATH=/dev/... selects the
     * raw-zone store as the expert cold source (O_DIRECT, no page cache).
     * Non-destructive: mmap path remains default when unset. */
    const char* store_env = getenv("VITRIOL_STORE");
    if (store_env && strcmp(store_env, "1") == 0) {
        const char* sp = getenv("VITRIOL_STORE_PATH");
        if (sp && *sp) {
            snprintf(g_store_path, sizeof(g_store_path), "%s", sp);
            g_vitriol_config.store_backed = true;
        } else {
            fprintf(stderr, "VITRIOL: VITRIOL_STORE=1 requires VITRIOL_STORE_PATH\n");
        }
        const char * source_env = getenv("VITRIOL_STORE_SOURCE");
        if (source_env) {
            if (strcmp(source_env, "store") == 0) {
                g_store_source = store_source::device;
            } else if (strcmp(source_env, "gguf") == 0) {
                g_store_source = store_source::gguf;
            } else {
                fprintf(stderr, "VITRIOL: invalid VITRIOL_STORE_SOURCE=%s\n", source_env);
            }
        }
        if (g_vitriol_config.verbose) {
            printf("VITRIOL: store source=%s\n",
                   g_store_source == store_source::device ? "store" : "gguf");
        }
    }

    const char* oc_env = getenv("VITRIOL_OUTPUT_CACHE");
    if (oc_env && strcmp(oc_env, "1") == 0)
        g_vitriol_config.output_cache = true;

    const char* pin_env = getenv("VITRIOL_PIN_FIRST_N_LAYERS");
    if (pin_env) {
        int val = atoi(pin_env);
        if (val > 0) g_vitriol_config.pin_first_n_layers = val;
    }

    const char* prune_env = getenv("VITRIOL_PRUNE_EXPERTS");
    if (prune_env) {
        int val = atoi(prune_env);
        if (val > 0 && val <= 7) g_vitriol_config.prune_experts = val;
    }

    const char* locked_env = getenv("VITRIOL_MAX_LOCKED_MB");
    if (locked_env) {
        int val = atoi(locked_env);
        if (val > 0) g_vitriol_config.max_locked_mb = val;
    }

    if (g_vitriol_config.mode == VITRIOL_MODE_STREAM) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: stream mode — page-locked host RAM + LRU VRAM cache\n");
        if (g_vitriol_config.max_locked_mb > 0) {
            if (g_vitriol_config.verbose)
                printf("VITRIOL: lazy page locking — max %d MB, per-expert on demand\n",
                       g_vitriol_config.max_locked_mb);
        }
    }

    if (g_vitriol_config.async_prefetch) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: predictive prefetching enabled (cross-layer + temporal)\n");
    }

    if (g_vitriol_config.pin_first_n_layers > 0) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: expert pinning enabled — first %d layers will be loaded into VRAM\n",
                   g_vitriol_config.pin_first_n_layers);
        /* Pinning targets the fast path; disable output cache to avoid confusion */
        if (g_vitriol_config.output_cache) {
            g_vitriol_config.output_cache = false;
            if (g_vitriol_config.verbose)
                printf("VITRIOL: output cache disabled (pinning + output cache target different decode paths)\n");
        }
    }

    if (g_vitriol_config.disk_offload) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: disk offload mode — file-backed mmap, no page-lock\n");
    }

    static bool first_init = true;
    if (first_init) {
        memset(&g_lru_stats, 0, sizeof(g_lru_stats));
        first_init = false;

        /* Register atexit handlers */
        atexit(vitriol_cuda_cleanup_vram);
        atexit(vitriol_cuda_print_stats);
    }
}

/* ── Store-backed fill ─────────────────────────────────────────── */

static std::mutex g_store_mtx;

struct store_buffer_reg {
    ggml_backend_buffer_t buffer;
    vitriol::store * store;
    const void * base;
    size_t size;
};

struct store_tensor_reg {
    ggml_backend_buffer_t buffer;
    vitriol::store * store;
    uint32_t tid;
};

struct store_pointer_hash {
    size_t operator()(const void * p) const noexcept {
        size_t x = std::hash<const void *>{}(p);
        x ^= x >> 30;
        x *= 0xbf58476d1ce4e5b9ULL;
        x ^= x >> 27;
        x *= 0x94d049bb133111ebULL;
        return x ^ (x >> 31);
    }
};

static std::vector<store_buffer_reg> g_store_buffers;
static std::unordered_map<const void *, store_tensor_reg, store_pointer_hash> g_store_tensors;
static std::atomic<bool> g_store_binding_failed{false};
static size_t g_store_max_expert_bytes = 0;

static int store_kind_from_name(const char * name) {
    if (!name) return -1;
    if (strstr(name, "ffn_up_exps"))    return 0;
    if (strstr(name, "ffn_gate_exps"))  return 1;
    if (strstr(name, "ffn_down_exps"))  return 2;
    return -1;
}

static int store_layer_from_name(const char * name) {
    if (!name) return -1;
    const char * b = strstr(name, "blk.");
    return b ? atoi(b + 4) : -1;
}

static void release_locked_range(const void * base, size_t size);

static bool store_buffer_for(
        ggml_backend_buffer_t buffer,
        store_buffer_reg & out) {
    std::lock_guard<std::mutex> lock(g_store_mtx);
    for (const auto & reg : g_store_buffers) {
        if (reg.buffer == buffer) {
            out = reg;
            return true;
        }
    }
    return false;
}

bool vitriol_store_register_buffer(
        ggml_backend_buffer_t buffer,
        void * store_handle,
        const void * base,
        size_t size) {
    if (!buffer || !store_handle || !base || size == 0) return false;
    auto * store = static_cast<vitriol::store *>(store_handle);
    std::lock_guard<std::mutex> lock(g_store_mtx);
    for (auto & reg : g_store_buffers) {
        if (reg.buffer == buffer) {
            reg.store = store;
            reg.base = base;
            reg.size = size;
            return true;
        }
    }
    g_store_buffers.push_back({buffer, store, base, size});
    return true;
}

bool vitriol_store_bind_tensor(
        ggml_backend_buffer_t buffer,
        const void * tensor_base,
        const char * name,
        size_t tensor_bytes,
        size_t row_bytes) {
    if (!g_vitriol_config.store_backed || !tensor_base || !name) return false;
    if (vitriol_rust_enabled()) {
        /* Rust mode: the store and binding table live in the crate; the
         * buffer handle is irrelevant there. */
        return vs_bind_tensor_named((void *) tensor_base, name,
                                    tensor_bytes, row_bytes) == 1;
    }
    if (!buffer) return false;

    store_buffer_reg buffer_reg;
    if (!store_buffer_for(buffer, buffer_reg)) return false;

    const int role = store_kind_from_name(name);
    const int layer = store_layer_from_name(name);
    if (role < 0 || layer < 0) return false;

    const uint32_t count = vitriol::store_tensor_count(buffer_reg.store);
    for (uint32_t tid = 0; tid < count; ++tid) {
        const auto & desc = vitriol::store_tensor_desc_at(buffer_reg.store, tid);
        if (desc.layer != static_cast<uint32_t>(layer) || desc.kind != static_cast<uint32_t>(role)) {
            continue;
        }
        if (desc.n_experts == 0 || desc.data_bytes == 0 || desc.slice_bytes < desc.data_bytes ||
            row_bytes != desc.data_bytes ||
            static_cast<uint64_t>(desc.n_experts) * desc.data_bytes != tensor_bytes) {
            return false;
        }
        std::lock_guard<std::mutex> lock(g_store_mtx);
        g_store_max_expert_bytes = std::max(g_store_max_expert_bytes,
            static_cast<size_t>(desc.data_bytes));
        const auto found = g_store_tensors.find(tensor_base);
        if (found != g_store_tensors.end()) {
            return found->second.buffer == buffer && found->second.store == buffer_reg.store &&
                found->second.tid == tid;
        }
        g_store_tensors.emplace(tensor_base, store_tensor_reg{buffer, buffer_reg.store, tid});
        if (g_vitriol_config.verbose) {
            printf("VITRIOL: store reg %s -> tid %u (base %p, %zu B)\n",
                   name, tid, tensor_base, tensor_bytes);
        }
        return true;
    }
    return false;
}

static bool store_binding_for(
        const void * tensor_base,
        vitriol::store *& store,
        uint32_t & tid) {
    std::lock_guard<std::mutex> lock(g_store_mtx);
    const auto found = g_store_tensors.find(tensor_base);
    if (found == g_store_tensors.end()) return false;
    store = found->second.store;
    tid = found->second.tid;
    return true;
}

void vitriol_store_release_buffer(
        ggml_backend_buffer_t buffer,
        const void * base,
        size_t size) {
    if (!vitriol_store_source_device()) release_locked_range(base, size);
    std::lock_guard<std::mutex> lock(g_store_mtx);
    for (auto it = g_store_tensors.begin(); it != g_store_tensors.end();) {
        if (it->second.buffer == buffer) {
            it = g_store_tensors.erase(it);
        } else {
            ++it;
        }
    }
    g_store_buffers.erase(
        std::remove_if(g_store_buffers.begin(), g_store_buffers.end(),
            [buffer](const store_buffer_reg & reg) { return reg.buffer == buffer; }),
        g_store_buffers.end());
}

bool vitriol_store_source_device(void) {
    /* store fills only exist when a store is actually bound; the source
     * flag alone defaults to "device" and must not route non-store runs
     * into store_read_bounce (latent abort once the hooks engage) */
    return g_vitriol_config.store_backed && g_store_source == store_source::device;
}

bool vitriol_loader_stream_mode() {
    return g_vitriol_config.mode == VITRIOL_MODE_STREAM;
}

bool vitriol_skip_host_read(struct ggml_tensor * cur) {
    if (!g_vitriol_config.store_backed || !cur || !cur->buffer) return false;
    if (!vitriol_store_bind_tensor(cur->buffer, cur->data, cur->name,
                                   ggml_nbytes(cur), cur->nb[2])) {
        g_store_binding_failed.store(true, std::memory_order_relaxed);
        return false;
    }
    return vitriol_store_source_device();
}

struct store_stats {
    uint64_t fills = 0;
    uint64_t bytes = 0;
    uint64_t failures = 0;
    uint64_t read_ns = 0;
};
static store_stats g_store_stats;


/* ── Locked-Slice LRU (Lazy/Chunked Page Locking) ──────────────
 * Tracks which (tensor_base, expert_idx) pairs are currently
 * page-locked via mlock + cudaHostRegister. Keeps total locked
 * bytes below g_vitriol_config.max_locked_mb. Evicts by LRU.
 * ───────────────────────────────────────────────────────────────*/

struct locked_key {
    const void * tensor_base;
    uint32_t     expert_idx;

    bool operator==(const locked_key & other) const noexcept {
        return tensor_base == other.tensor_base && expert_idx == other.expert_idx;
    }
};

struct locked_key_hash {
    size_t operator()(const locked_key & key) const noexcept {
        size_t value = std::hash<const void *>{}(key.tensor_base);
        value ^= std::hash<uint32_t>{}(key.expert_idx) + 0x9e3779b9U + (value << 6) + (value >> 2);
        value ^= value >> 30;
        value *= 0xbf58476d1ce4e5b9ULL;
        value ^= value >> 27;
        value *= 0x94d049bb133111ebULL;
        return value ^ (value >> 31);
    }
};

using locked_order = std::list<locked_key>;

struct locked_value {
    size_t              stride;
    int                 device;
    locked_order::iterator order_it;
};

static constexpr size_t VITRIOL_MAX_LOCKED_SLICES = 1U << 20;

static std::mutex g_locked_mtx;
static locked_order g_locked_order;
static std::unordered_map<locked_key, locked_value, locked_key_hash> g_locked_slices;
static uint64_t g_locked_bytes = 0;

struct host_registration {
    const void * addr;
    size_t size;
    int device;
};
static std::vector<host_registration> g_orphan_registrations;

static void * locked_address(const locked_key & key, size_t stride) {
    return const_cast<std::byte *>(
        static_cast<const std::byte *>(key.tensor_base) +
        static_cast<size_t>(key.expert_idx) * stride);
}

static bool unregister_host_range(void * addr, int device, bool synchronize) {
    if (synchronize) {
        cudaSetDevice(device);
        if (cudaDeviceSynchronize() != cudaSuccess) return false;
    } else {
        cudaSetDevice(device);
    }
    const cudaError_t err = cudaHostUnregister(addr);
    return err == cudaSuccess;
}

static void retire_locked_value(const locked_key & key, const locked_value & value) {
    void * addr = locked_address(key, value.stride);
    if (!unregister_host_range(addr, value.device, true)) {
        g_orphan_registrations.push_back({addr, value.stride, value.device});
    }
    munlock(addr, value.stride);
    g_locked_bytes -= value.stride;
}

static void release_locked_range(const void * base, size_t size) {
    if (!base || size == 0) return;
    for (int device = 0; device < GGML_CUDA_MAX_DEVICES; ++device) {
        cudaSetDevice(device);
        cudaDeviceSynchronize();
    }

    const auto * begin = static_cast<const std::byte *>(base);
    const auto * end = begin + size;
    std::lock_guard<std::mutex> lock(g_locked_mtx);
    for (auto it = g_locked_order.begin(); it != g_locked_order.end();) {
        const locked_key key = *it;
        const auto found = g_locked_slices.find(key);
        if (found == g_locked_slices.end()) {
            it = g_locked_order.erase(it);
            continue;
        }
        const locked_value value = found->second;
        const auto * addr = static_cast<const std::byte *>(locked_address(key, value.stride));
        if (addr >= begin && addr < end) {
            if (!unregister_host_range(const_cast<std::byte *>(addr), value.device, false)) {
                g_orphan_registrations.push_back({addr, value.stride, value.device});
            }
            munlock(addr, value.stride);
            g_locked_bytes -= value.stride;
            g_locked_slices.erase(found);
            it = g_locked_order.erase(it);
        } else {
            ++it;
        }
    }
    for (auto it = g_orphan_registrations.begin(); it != g_orphan_registrations.end();) {
        const auto * addr = static_cast<const std::byte *>(it->addr);
        if (addr >= begin && addr < end) {
            unregister_host_range(const_cast<std::byte *>(addr), it->device, false);
            it = g_orphan_registrations.erase(it);
        } else {
            ++it;
        }
    }
}

struct store_bounce_slot {
    void * allocation = nullptr;
    void * aligned = nullptr;
    size_t capacity = 0;
    CUevent event = nullptr;
};

static std::vector<store_bounce_slot> g_store_bounce[GGML_CUDA_MAX_DEVICES];
static size_t g_store_bounce_next[GGML_CUDA_MAX_DEVICES] = {};
static std::mutex g_store_bounce_mtx[GGML_CUDA_MAX_DEVICES];

static constexpr int VITRIOL_STORE_BOUNCE_SLOTS = 16;
static constexpr size_t VITRIOL_STORE_BOUNCE_ALIGN = 512;

static size_t aligned_size(size_t size, size_t alignment) {
    return (size + alignment - 1) / alignment * alignment;
}

static store_bounce_slot * store_bounce_acquire(int device, size_t size) {
    std::lock_guard<std::mutex> lock(g_store_bounce_mtx[device]);
    if (g_store_bounce[device].empty()) {
        fprintf(stderr, "VITRIOL: store bounce ring not initialized (GPU %d)\n", device);
        return nullptr;
    }
    const size_t index = g_store_bounce_next[device] % g_store_bounce[device].size();
    ++g_store_bounce_next[device];
    store_bounce_slot & slot = g_store_bounce[device][index];
    if (slot.capacity < size || !slot.aligned || !slot.event) {
        fprintf(stderr, "VITRIOL: store bounce slot %d too small (%zu < %zu)\n",
                static_cast<int>(index), slot.capacity, size);
        return nullptr;
    }
    if (cuEventSynchronize(slot.event) != CUDA_SUCCESS) {
        fprintf(stderr, "VITRIOL: store bounce event sync failed (GPU %d)\n", device);
        return nullptr;
    }
    return &slot;
}

static bool store_bounce_init(int device, size_t expert_size) {
    std::lock_guard<std::mutex> lock(g_store_bounce_mtx[device]);
    if (!g_store_bounce[device].empty()) return true;

    const size_t capacity = aligned_size(expert_size, VITRIOL_STORE_BOUNCE_ALIGN);
    cudaSetDevice(device);
    for (int i = 0; i < VITRIOL_STORE_BOUNCE_SLOTS; ++i) {
        store_bounce_slot slot;
        const cudaError_t alloc = cudaHostAlloc(&slot.allocation,
            capacity + VITRIOL_STORE_BOUNCE_ALIGN, cudaHostAllocPortable);
        if (alloc != cudaSuccess) {
            fprintf(stderr, "VITRIOL: cudaHostAlloc bounce failed (GPU %d): %s\n",
                    device, cudaGetErrorString(alloc));
            return false;
        }
        slot.aligned = static_cast<std::byte *>(slot.allocation) +
            (VITRIOL_STORE_BOUNCE_ALIGN -
             reinterpret_cast<uintptr_t>(slot.allocation) % VITRIOL_STORE_BOUNCE_ALIGN);
        slot.capacity = capacity;
        if (cuEventCreate(&slot.event, CU_EVENT_DISABLE_TIMING) != CUDA_SUCCESS) {
            fprintf(stderr, "VITRIOL: cuEventCreate bounce failed (GPU %d)\n", device);
            cudaFreeHost(slot.allocation);
            return false;
        }
        g_store_bounce[device].push_back(slot);
    }
    return true;
}

static bool store_read_bounce(
        const void * tensor_base,
        int expert_idx,
        size_t expert_size,
        store_bounce_slot & bounce) {
    vitriol::store * store = nullptr;
    uint32_t tid = 0;
    if (!store_binding_for(tensor_base, store, tid)) {
        fprintf(stderr, "VITRIOL: store H2D has no exact tensor binding\n");
        return false;
    }
    if (!store || tid >= vitriol::store_tensor_count(store) ||
        static_cast<uint32_t>(expert_idx) >= vitriol::store_tensor_desc_at(store, tid).n_experts ||
        vitriol::store_tensor_desc_at(store, tid).data_bytes != expert_size) {
        fprintf(stderr, "VITRIOL: store H2D geometry mismatch (tid=%u e=%d)\n", tid, expert_idx);
        return false;
    }

    timespec start{};
    timespec finish{};
    clock_gettime(CLOCK_MONOTONIC, &start);
    const ssize_t result = vitriol::store_read_expert(
        store, tid, static_cast<uint32_t>(expert_idx), bounce.aligned, expert_size);
    clock_gettime(CLOCK_MONOTONIC, &finish);
    g_store_stats.read_ns +=
        static_cast<uint64_t>(finish.tv_sec - start.tv_sec) * 1000000000ULL +
        static_cast<uint64_t>(finish.tv_nsec - start.tv_nsec);
    if (result != static_cast<ssize_t>(expert_size)) {
        g_store_stats.failures++;
        fprintf(stderr, "VITRIOL: store H2D read FAIL tid=%u e=%d (%zd)\n",
                tid, expert_idx, result);
        return false;
    }
    g_store_stats.fills++;
    g_store_stats.bytes += expert_size;
    return true;
}

void vitriol_ensure_expert_locked(const void * tensor_base, int expert_idx, size_t expert_size) {
    if (!tensor_base || expert_idx < 0 || expert_size == 0) return;
    if (!vitriol_lazy_lock_active() || vitriol_store_source_device()) return;

    std::lock_guard<std::mutex> lock(g_locked_mtx);
    const locked_key key{tensor_base, static_cast<uint32_t>(expert_idx)};
    const auto found = g_locked_slices.find(key);
    if (found != g_locked_slices.end()) {
        if (found->second.stride == expert_size) {
            g_locked_order.splice(g_locked_order.begin(), g_locked_order, found->second.order_it);
            return;
        }
        const locked_key old_key = key;
        const locked_value old_value = found->second;
        g_locked_order.erase(old_value.order_it);
        g_locked_slices.erase(found);
        retire_locked_value(old_key, old_value);
    }

    const uint64_t max_bytes = static_cast<uint64_t>(g_vitriol_config.max_locked_mb) * 1024 * 1024;
    if (max_bytes == 0 || expert_size > max_bytes) return;
    while (!g_locked_order.empty() &&
           (g_locked_bytes > max_bytes || expert_size > max_bytes - g_locked_bytes ||
            g_locked_slices.size() >= VITRIOL_MAX_LOCKED_SLICES)) {
        const locked_key victim_key = g_locked_order.back();
        const auto victim = g_locked_slices.find(victim_key);
        if (victim == g_locked_slices.end()) {
            g_locked_order.pop_back();
            continue;
        }
        const locked_value victim_value = victim->second;
        g_locked_order.pop_back();
        g_locked_slices.erase(victim);
        retire_locked_value(victim_key, victim_value);
    }

    void * addr = locked_address(key, expert_size);
    if (mlock(addr, expert_size) != 0) {
        static bool warned = false;
        if (!warned) {
            fprintf(stderr, "VITRIOL: mlock slice(%zu) failed: %s\n", expert_size, strerror(errno));
            warned = true;
        }
        return;
    }

    const cudaError_t err = cudaHostRegister(addr, expert_size, 0);
    if (err != cudaSuccess) {
        static bool warned = false;
        if (!warned) {
            fprintf(stderr, "VITRIOL: cudaHostRegister slice(%zu) failed: %s\n",
                    expert_size, cudaGetErrorString(err));
            warned = true;
        }
        /* the failed runtime call poisons CUDA's per-thread sticky last
         * error; the next ggml launch check would read it as its own and
         * abort ("invalid argument" at quantize_row_q8_1). Clear it. */
        cudaGetLastError();
        munlock(addr, expert_size);
        return;
    }

    const auto order_it = g_locked_order.insert(g_locked_order.begin(), key);
    int device = vitriol_current_device();
    g_locked_slices.emplace(key, locked_value{expert_size, device, order_it});
    g_locked_bytes += expert_size;

    if (g_vitriol_config.verbose) {
        printf("VITRIOL: locked slice (tensor=%p, expert=%d, size=%zu, total_locked=%lu MiB)\n",
               tensor_base, expert_idx, expert_size,
               static_cast<unsigned long>(g_locked_bytes / 1024 / 1024));
    }
}

/* Fallback predictor state (used when per-layer tracking hasn't stabilized) */
static std::mutex            g_pred_mtx;
static int                   g_pred_experts[256];
static int                    g_pred_count;

/* ── Temporal + Cross-Layer Tracking ─────────────────────────────────
 * Per-layer expert selection history across tokens.
 * Combined predictor: prefetch union of
 *   (a) cross-layer:  cur[this_layer - 1]  (experts from previous layer, same token)
 *   (b) temporal:     prev[this_layer]      (experts from same layer, previous token)
 *
 * Token boundary is detected when a previously-seen tensor_base reappears
 * at an index lower than the highest index seen so far (i.e., sequential
 * layer processing wraps back to layer 0).
 */
#define VITRIOL_MAX_LAYERS 128

/* Map tensor_base address → sequential layer index (0, 1, 2, ...) */
static uintptr_t g_layer_bases[VITRIOL_MAX_LAYERS];
static int       g_n_layers = 0;   // total distinct layers seen
static int       g_last_layer = -1; // last layer index seen

/* Current token */
static int  g_cur_exp[VITRIOL_MAX_LAYERS][256];
static int  g_cur_cnt[VITRIOL_MAX_LAYERS];

/* Previous token (boundary: wrap detected) */
static int  g_prev_exp[VITRIOL_MAX_LAYERS][256];
static int  g_prev_cnt[VITRIOL_MAX_LAYERS];

/* ── Layer Index Map ──────────────────────────────────────────────── */

/* Resolve tensor_base → sequential layer index (or -1 if at capacity).
 * This is NOT a direct hardware-layer mapping — it's an ordinal index
 * in order of first appearance (layer 0 → 0, layer 1 → 1, ...), which
 * matches sequential layer execution in autoregressive decode. */
static int get_layer_index(uintptr_t tensor_base) {
    for (int i = 0; i < g_n_layers && i < VITRIOL_MAX_LAYERS; i++) {
        if (g_layer_bases[i] == tensor_base) return i;
    }
    if (g_n_layers >= VITRIOL_MAX_LAYERS) return -1;
    g_layer_bases[g_n_layers] = tensor_base;
    return g_n_layers++;
}

/* Check for token boundary: when layer index wrapped back to an earlier
 * value (e.g., 0 after 39), the current token's per-layer data becomes
 * the "previous token" for the next call. */
static void detect_token_boundary(int layer_idx) {
    if (g_last_layer >= 0 && layer_idx <= g_last_layer) {
        // Swap cur → prev for all layers
        for (int i = 0; i < VITRIOL_MAX_LAYERS; i++) {
            g_prev_cnt[i] = g_cur_cnt[i];
            memcpy(g_prev_exp[i], g_cur_exp[i], g_cur_cnt[i] * sizeof(int));
            g_cur_cnt[i] = 0;
        }
    }
    g_last_layer = layer_idx;
}

void vitriol_predictor_prefetch(
    const void    *tensor_base,
    size_t         expert_size,
    CUstream       compute_stream)
{
    if (!g_vitriol_config.async_prefetch)
        return;

    if (!lru_ensure_stream(vitriol_current_device()))
        return;

    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0) return;

    detect_token_boundary(layer_idx);

    /* Collect predicted expert set (union of cross-layer + temporal) */
    int predicted[256];
    int n_predicted = 0;

    auto add_prediction = [&](int e) {
        if (e < 0) return;
        for (int i = 0; i < n_predicted; i++)
            if (predicted[i] == e) return;
        predicted[n_predicted++] = e;
    };

    /* (a) Cross-layer: experts from the previous layer of current token */
    if (layer_idx > 0) {
        for (int i = 0; i < g_cur_cnt[layer_idx - 1]; i++)
            add_prediction(g_cur_exp[layer_idx - 1][i]);
    }

    /* (b) Temporal: experts from the same layer of previous token */
    for (int i = 0; i < g_prev_cnt[layer_idx]; i++)
        add_prediction(g_prev_exp[layer_idx][i]);

    if (n_predicted == 0)
        return;

    /* Also fall through to the old g_pred_experts from the last call
     * (for pre-existing behavior on the very first few layers before
     *  per-layer tracking builds up). */
    {
        std::lock_guard<std::mutex> lock(g_pred_mtx);
        for (int i = 0; i < g_pred_count && n_predicted < 256; i++)
            add_prediction(g_pred_experts[i]);
    }

    /* Submit async prefetches for the combined prediction set */
    for (int i = 0; i < n_predicted; i++) {
        int e = predicted[i];
        const void *expert_data = (const char *)tensor_base + (size_t)e * expert_size;
        vitriol_lru_prefetch(tensor_base, e, expert_data, expert_size, compute_stream);
    }
}

void vitriol_predictor_update(
    const void    *tensor_base,
    size_t         expert_size,
    const int32_t *expert_ids,
    int            n_experts)
{
    (void)tensor_base;
    (void)expert_size;

    if (!g_vitriol_config.async_prefetch)
        return;

    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0) {
        /* Fallback: store in old flat buffer */
        std::lock_guard<std::mutex> lock(g_pred_mtx);
        g_pred_count = 0;
        for (int i = 0; i < n_experts; i++) {
            int e = expert_ids[i];
            if (e < 0) continue;
            bool dup = false;
            for (int j = 0; j < g_pred_count; j++)
                if (g_pred_experts[j] == e) { dup = true; break; }
            if (dup) continue;
            g_pred_experts[g_pred_count++] = e;
        }
        return;
    }

    detect_token_boundary(layer_idx);

    /* Store current layer's expert indices (deduplicated) */
    int count = 0;
    for (int i = 0; i < n_experts && count < 256; i++) {
        int e = expert_ids[i];
        if (e < 0) continue;
        bool dup = false;
        for (int j = 0; j < count; j++)
            if (g_cur_exp[layer_idx][j] == e) { dup = true; break; }
        if (dup) continue;
        g_cur_exp[layer_idx][count++] = e;
    }
    g_cur_cnt[layer_idx] = count;

    /* Also update the old flat buffer as a fallback predictor
     * (used during the first pass before per-layer tracking stabilizes) */
    {
        std::lock_guard<std::mutex> lock(g_pred_mtx);
        g_pred_count = count;
        memcpy(g_pred_experts, g_cur_exp[layer_idx], count * sizeof(int));
    }
}

/* ── Expert Pinning Operations ────────────────────────────────────── */

/* Per-device pin range (model-layer count of pinned layers for this GPU).
 * Env VITRIOL_PIN_FIRST_N_LAYERS_GPU<d> overrides the global
 * VITRIOL_PIN_FIRST_N_LAYERS for that device — e.g. pin 16 layers on the
 * 12 GB card, only 6 on the 8 GB card. */
static int vitriol_pin_range(int device) {
    char env_buf[64];
    snprintf(env_buf, sizeof(env_buf), "VITRIOL_PIN_FIRST_N_LAYERS_GPU%d", device);
    const char * dev_env = getenv(env_buf);
    if (dev_env && dev_env[0]) {
        int v = atoi(dev_env);
        return v < 0 ? 0 : v;
    }
    return g_vitriol_config.pin_first_n_layers;
}

bool vitriol_pin_enabled(void) {
    if (g_vitriol_config.pin_first_n_layers > 0)
        return true;
    for (int d = 0; d < GGML_CUDA_MAX_DEVICES; d++) {
        if (vitriol_pin_range(d) > 0)
            return true;
    }
    return false;
}

CUdeviceptr vitriol_pin_ensure(
    const void    *tensor_base,
    size_t         tensor_nb02,
    int64_t        n_experts,
    CUstream       stream)
{
    if (vitriol_store_source_device()) return 0;
    if (!tensor_base || tensor_nb02 == 0 || n_experts <= 0)
        return 0;

    /* Only pin layers within the configured range.
     * Each model layer has multiple tensor ops (e.g., gate_up + down for fused MoE).
     * Divide by pin_tensors_per_layer to count in model-layer units. */
    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0)
        return 0;

    int tensors_per_layer = g_vitriol_config.pin_tensors_per_layer;
    if (tensors_per_layer <= 0) tensors_per_layer = 2;  // safe default

    int model_layer = layer_idx / tensors_per_layer;

    int dev = vitriol_current_device();
    if (model_layer >= vitriol_pin_range(dev))
        return 0;

    /* Check if already pinned */
    {
        std::lock_guard<std::mutex> lock(g_pin_mtx);
        auto it = g_pin_map.find((uintptr_t)tensor_base);
        if (it != g_pin_map.end()) {
            return it->second;
        }
    }

    /* Get total size for this tensor */
    size_t total_size = tensor_nb02 * (size_t)n_experts;

    if (g_pin_pool_failed[dev])
        return 0;

    /* Per-device monolithic pool: allocate once on first pin for this GPU.
     * (Layer split → each pinned tensor lives on exactly one GPU.) */
    if (g_pin_pool[dev] == 0) {
        int expected_tensors = vitriol_pin_range(dev) *
                               g_vitriol_config.pin_tensors_per_layer;
        if (expected_tensors < 1) expected_tensors = 1;
        /* Use 1.5× the first tensor's size as per-slot estimate.
         * Different tensor types vary (e.g., gate_up ~66 MB, down ~98 MB),
         * so we leave margin. If a tensor exceeds remaining pool space,
         * it falls through to host RAM gracefully. */
        size_t pool_size = total_size * expected_tensors * 3 / 2;
        cudaSetDevice(dev);
        CUresult err = cuMemAlloc(&g_pin_pool[dev], pool_size);
        if (err != CUDA_SUCCESS) {
            fprintf(stderr, "VITRIOL: pin pool alloc %zu MB on GPU %d failed (%d) — pinning disabled for this device\n",
                    pool_size / 1024 / 1024, dev, (int)err);
            g_pin_pool_failed[dev] = true;
            return 0;
        }
        g_pin_pool_total[dev] = pool_size;
        g_pin_pool_offset[dev] = 0;
        if (g_vitriol_config.verbose)
            printf("VITRIOL: pin pool allocated: %zu MB (%d slots x ~%zu MB each) on GPU %d\n",
                   pool_size / 1024 / 1024, expected_tensors,
                   total_size / 1024 / 1024, dev);
    }

    /* Check if this tensor fits in remaining pool */
    if (g_pin_pool_offset[dev] + total_size > g_pin_pool_total[dev]) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: pin pool exhausted for layer %d (%zu MB needed, %zu MB left) — skipping\n",
                   layer_idx, total_size / 1024 / 1024,
                   (g_pin_pool_total[dev] - g_pin_pool_offset[dev]) / 1024 / 1024);
        return 0;
    }

    CUdeviceptr vram_buf = g_pin_pool[dev] + g_pin_pool_offset[dev];
    g_pin_pool_offset[dev] += total_size;

    /* Async H2D copy of the full tensor to VRAM.
     * In lazy mode, ensure all expert pages are locked first.
     * Store mode has no host expert bytes; pinning is not supported. */
    if (vitriol_lazy_lock_active() && !vitriol_store_source_device()) {
        for (int64_t i = 0; i < n_experts; i++) {
            vitriol_ensure_expert_locked(tensor_base, (int)i, tensor_nb02);
        }
    }
    cudaSetDevice(dev);
    CUresult err = cuMemcpyHtoDAsync(vram_buf, tensor_base, total_size, stream);
    if (err != CUDA_SUCCESS) {
        fprintf(stderr, "VITRIOL: pin H2D copy failed (%d)\n", (int)err);
        return 0;
    }

    /* Synchronize so matmul can safely read from VRAM */
    cuStreamSynchronize(stream);

    /* Store in pin map */
    {
        std::lock_guard<std::mutex> lock(g_pin_mtx);
        g_pin_map[(uintptr_t)tensor_base] = vram_buf;
        g_pinned_bytes += total_size;
        g_vitriol_config.pin_active = true;
    }

    if (g_vitriol_config.verbose)
        printf("VITRIOL: pinned layer %d tensor %p -> VRAM %llx (%zu MB, %d experts x %zu bytes)\n",
               layer_idx, tensor_base, (unsigned long long)vram_buf,
               total_size / 1024 / 1024, (int)n_experts, tensor_nb02);

    return vram_buf;
}

CUdeviceptr vitriol_pin_lookup(const void *tensor_base) {
    if (!tensor_base) return 0;
    std::lock_guard<std::mutex> lock(g_pin_mtx);
    auto it = g_pin_map.find((uintptr_t)tensor_base);
    return (it != g_pin_map.end()) ? it->second : 0;
}

static bool lru_ensure_stream(int device) {
    if (g_lru_stream[device] != 0)
        return true;
    std::lock_guard<std::mutex> lock(g_lru_init_mtx);
    if (g_lru_stream[device] != 0)
        return true;
    CUresult r;
    cudaSetDevice(device);
    r = cuStreamCreate(&g_lru_stream[device], CU_STREAM_NON_BLOCKING);
    if (r != CUDA_SUCCESS) return false;
    return true;
}

/* Initialize VRAM pool for one device.  Slot size is fixed at first
 * allocation; if a later tensor has larger experts they bypass the cache
 * (return 0 → host RAM read).  This prevents pool thrashing from resizing.
 * VITRIOL_LRU_MB applies per device (each GPU gets its own up-to-LRU_MB pool). */
static bool lru_init_pool(size_t min_expert_size, int device) {
    if (g_lru_pool[device] != 0)
        return true;
    std::lock_guard<std::mutex> lock(g_lru_init_mtx);
    if (g_lru_pool[device] != 0)
        return true;

    const char* pool_env = getenv("VITRIOL_LRU_MB");
    /* Per-device override: VITRIOL_LRU_MB_<dev> wins over the global. Lets the
     * 12 GB card keep a fat pool while the 8 GB card runs a small one. */
    char dev_env_name[32];
    snprintf(dev_env_name, sizeof(dev_env_name), "VITRIOL_LRU_MB_%d", device);
    const char* dev_env = getenv(dev_env_name);
    if (dev_env && dev_env[0]) {
        pool_env = dev_env;
    }
    /* Stage 4.5: default pool = free VRAM at init - reserve. The reserve
     * (VITRIOL_POOL_RESERVE_MB, default 2300/device) keeps headroom for KV,
     * dense weights and compute-buffer growth. An explicit VITRIOL_LRU_MB
     * still pins an exact size; 0 or unset now means auto. */
    size_t pool_size = 0;
    if (pool_env) {
        unsigned long mb = strtoul(pool_env, NULL, 10);
        if (mb >= 64) pool_size = mb * 1024ULL * 1024;
    }
    if (pool_size == 0) {
        size_t free_b = 0, total_b = 0;
        cudaSetDevice(device);
        if (cudaMemGetInfo(&free_b, &total_b) == cudaSuccess) {
            unsigned long reserve_mb = 2300;
            const char* rsv = getenv("VITRIOL_POOL_RESERVE_MB");
            if (rsv && rsv[0]) reserve_mb = strtoul(rsv, NULL, 10);
            const size_t reserve_b = reserve_mb * 1024ULL * 1024;
            pool_size = free_b > reserve_b ? free_b - reserve_b : 0;
        }
        else {
            cudaGetLastError(); // failed runtime probe: clear sticky error
        }
    }
    if (pool_size == 0) {
        pool_size = VITRIOL_LRU_POOL_SIZE; // fallback: legacy default
    }

    size_t needed_slot = (min_expert_size + 255) & ~(size_t)255;
    if (g_store_max_expert_bytes > 0) {
        needed_slot = std::max(needed_slot,
            aligned_size(g_store_max_expert_bytes, static_cast<size_t>(256)));
    }
    int    needed_slots = (int)(pool_size / needed_slot);
    if (needed_slots > VITRIOL_LRU_MAX_SLOTS) needed_slots = VITRIOL_LRU_MAX_SLOTS;
    if (needed_slots < 1) needed_slots = 1;
    if (getenv("VITRIOL_LRU_DBG"))
        fprintf(stderr, "VITRIOL_LRU_DBG pool dev=%d min_expert=%zu slot=%zu slots=%d pool_mb=%zu\n",
                device, min_expert_size, needed_slot, needed_slots, pool_size/1024/1024);

    /* Stream must exist before slots events are created, and both must be
     * bound to this device's context. */
    if (!lru_ensure_stream(device))
        return false;

    cudaSetDevice(device);
    cudaFree(0); /* force primary-context init: cuMemAlloc on a thread whose
                    context is not yet created returns spurious OOM (2) */
    CUresult err = cuMemAlloc(&g_lru_pool[device], pool_size);
    if (err != CUDA_SUCCESS) {
        fprintf(stderr, "VITRIOL: LRU pool alloc %zu MB on GPU %d failed (%d)\n",
                pool_size / 1024 / 1024, device, (int)err);
        g_lru_pool[device] = 0;
        return false;
    }

    g_lru_pool_size[device]  = pool_size;
    g_lru_slot_size[device]  = needed_slot;
    g_lru_num_slots[device]  = needed_slots;
    g_lru_last_use[device].assign(needed_slots, 0);

    if (g_vitriol_config.verbose)
        printf("VITRIOL: LRU pool %zu MB, %d slots x %zu bytes on GPU %d\n",
               pool_size / 1024 / 1024, g_lru_num_slots[device], g_lru_slot_size[device], device);

    if (!g_lru_slot_events[device]) {
        g_lru_slot_events[device] = new CUevent[g_lru_num_slots[device]];
        for (int i = 0; i < g_lru_num_slots[device]; i++)
            cuEventCreate(&g_lru_slot_events[device][i], CU_EVENT_DISABLE_TIMING);
    }
    g_lru_compute_pending[device].assign(g_lru_num_slots[device], false);
    if (g_vitriol_config.store_backed &&
        g_store_source == store_source::device &&
        !store_bounce_init(device, needed_slot)) {
        return false;
    }

    return true;
}

/* ── LRU Cache Operations ────────────────────────────────────────── */

CUdeviceptr vitriol_lru_ensure(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size,
    CUstream       compute_stream)
{
    if (vitriol_rust_enabled()) {
        /* Mirror the C++ sequence: the runtime must have the device's
         * primary context current before any driver call (cuMemAlloc
         * returns CUDA_ERROR_INVALID_CONTEXT otherwise). */
        int rs_dev = 0;
        cudaGetDevice(&rs_dev);
        cudaSetDevice(rs_dev);
        const usize_t_rs r = vs_lru_ensure(tensor_base, expert_idx,
            expert_data, expert_size, (void *) compute_stream);
        if (r == (usize_t_rs) -1) {
            GGML_ABORT("VITRIOL(rust) LRU protocol violation");
        }
        return (CUdeviceptr) r;
    }
    const bool store_device = g_vitriol_config.store_backed && g_store_source == store_source::device;
    if (!store_device && (!expert_data || expert_size == 0) && !tensor_base)
        return 0;
    if (store_device && (!tensor_base || expert_size == 0))
        GGML_ABORT("VITRIOL store H2D called with invalid geometry");

    /* VITRIOL_NO_LRU=1: diagnostic kill-switch - bypass the VRAM pool and
     * let every slice read straight from the host page. Isolates pool
     * (hit/miss/evict) bugs from sync-loop bugs. */
    static const bool no_lru = getenv("VITRIOL_NO_LRU") != nullptr;
    if (no_lru)
        return 0;

    int dev = vitriol_current_device();

    /* VITRIOL_LRU_TRACE=1 with VITRIOL_LRU_TRACE_KEY=tensor:expert[,expert...]:
     * log branch/slot/fill for one key. Diagnostic for zero-slot reads. */
    static const bool trace = getenv("VITRIOL_LRU_TRACE") != nullptr;
    static const unsigned long long trace_base = [] {
        const char* e = getenv("VITRIOL_LRU_TRACE_BASE");
        return e ? strtoull(e, nullptr, 0) : 0ull;
    }();
    auto trace_log = [&](const char* what, int slot, CUdeviceptr dst, unsigned got) {
        fprintf(stderr, "VITRIOL_TRACE %s dev=%d expert=%d slot=%d dst=%llx first=%08x\n",
                what, dev, expert_idx, slot, (unsigned long long) dst, got);
    };
    if (trace && (!trace_base || (uintptr_t) tensor_base == trace_base)) {
        static const char* te = getenv("VITRIOL_LRU_TRACE_EXPERTS");
        static std::set<int> experts;
        if (te && experts.empty()) {
            for (char* s = strdup(te); ; ) {
                char* p = strchr(s, ','); if (p) *p = 0;
                experts.insert(atoi(s));
                if (!p) break; s = p + 1;
            }
        }
        if (experts.empty() || experts.count(expert_idx)) {
            /* VITRIOL_SENTINEL: pure-CPU ground truth. Capture the host
             * bytes once (load time / first request - known good), then
             * flag any divergence on later visits. Catches host-buffer
             * zeroing that CUDA-side compares (src vs vram) cannot see
             * because both sides go zero together. */
            static std::map<std::pair<uintptr_t,int>, std::array<unsigned char,12>> g_sent;
            auto sk = std::make_pair((uintptr_t) tensor_base, expert_idx);
            auto sit = g_sent.find(sk);
            if (sit == g_sent.end()) {
                std::array<unsigned char,12> b{};
                memcpy(b.data(), expert_data, b.size());
                g_sent.emplace(sk, b);
            } else if (memcmp(sit->second.data(), expert_data, sit->second.size()) != 0) {
                fprintf(stderr, "VITRIOL_SENTINEL DIVERGED tensor=%p expert=%d first=%02x%02x%02x want=%02x%02x%02x\n",
                        tensor_base, expert_idx,
                        ((const unsigned char*)expert_data)[0], ((const unsigned char*)expert_data)[1], ((const unsigned char*)expert_data)[2],
                        sit->second[0], sit->second[1], sit->second[2]);
            }
            char hb[4] = {0};
            cudaMemcpy(hb, expert_data, 4, cudaMemcpyHostToHost);
            cudaGetLastError(); // trace-only probe: do not poison sticky error
            unsigned first; memcpy(&first, hb, 4);
            trace_log("ENSURE-ENTER", -1, 0, first);
        }
    }

    if (!lru_ensure_stream(dev))
        GGML_ABORT("VITRIOL failed to create LRU stream");
    if (!lru_init_pool(expert_size, dev)) {
        /* graceful: bypass (0 = host fallback) instead of killing the run;
         * loud so the silent-degradation class stays visible */
        static bool warned = false;
        if (!warned) {
            fprintf(stderr, "VITRIOL: LRU pool unavailable on GPU %d - experts bypass the cache\n", dev);
            warned = true;
        }
        return 0;
    }

    if (expert_size > g_lru_slot_size[dev]) {
        if (store_device)
            GGML_ABORT("VITRIOL store expert exceeds LRU slot");
        return 0;
    }

    LRUKey key = { dev, (uintptr_t)tensor_base, expert_idx };

    CUstream cstream = compute_stream;

    /* All DMA below must execute in this device's context, because both the
     * LRU pool and the enter/transition streams are per-GPU (layer split). */
    cudaSetDevice(dev);

    /* Check cache */
    auto & lru_map   = g_lru_map[dev];
    auto & lru_order = g_lru_order[dev];
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        auto it = lru_map.find(key);
        if (it != lru_map.end()) {
            lru_order.remove(key);
            lru_order.push_front(key);
            g_lru_stats.hits++;
            if ((int) g_lru_last_use[dev].size() > it->second) {
                g_lru_last_use[dev][it->second] = ggml_time_ms(); // Stage 4.5 hot-set stamp
            }
            /* Wait for any in-flight prefetch DMA on this slot */
            cuStreamWaitEvent(cstream, g_lru_slot_events[dev][it->second], 0);
            if ((int)g_lru_compute_pending[dev].size() > it->second) {
                g_lru_compute_pending[dev][it->second] = true;
            }
            CUdeviceptr hit_dst = g_lru_pool[dev] + (size_t)it->second * g_lru_slot_size[dev];
            if (trace && (!trace_base || (uintptr_t) tensor_base == trace_base)) {
                static const char* te = getenv("VITRIOL_LRU_TRACE_EXPERTS");
                static std::set<int> experts;
                if (te && experts.empty()) {
                    for (char* s = strdup(te); ; ) {
                        char* p = strchr(s, ','); if (p) *p = 0;
                        experts.insert(atoi(s));
                        if (!p) break; s = p + 1;
                    }
                }
                if (experts.empty() || experts.count(expert_idx)) {
                    unsigned first = 0;
                    cuMemcpyDtoH(&first, hit_dst, 4);
                    trace_log("HIT", it->second, hit_dst, first);
                }
            }
            return hit_dst;
        }
    }

    /* Cache miss */
    g_lru_stats.misses++;

    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if (g_lru_next_fresh_slot[dev] < g_lru_num_slots[dev]) {
            slot = g_lru_next_fresh_slot[dev]++;
        } else {
            // PROVENANCE: inspiration — kimi-k3-in-c (Apache-2.0; re-derived, not copied).
            // Three-state cache principle: skip INFLIGHT slots in victim selection. Pick
            // the LRU candidate whose last DMA has COMPLETED (cuEventQuery == SUCCESS)
            // instead of blindly evicting the LRU and stalling on its in-flight fill.
            LRUKey evict = lru_order.back();
            bool found = false;
            int lru_fallback = -1;
            unsigned long hot_ms = 5000;
            if (const char* hm = getenv("VITRIOL_LRU_HOT_MS")) hot_ms = strtoul(hm, NULL, 10);
            const int64_t hot_cutoff = ggml_time_ms() - (int64_t) hot_ms;
            for (auto it = std::prev(lru_order.end()); it != lru_order.begin(); --it) {
                auto mit = lru_map.find(*it);
                int s = (mit != lru_map.end()) ? mit->second : -1;
                bool pending = s >= 0 && s < (int)g_lru_compute_pending[dev].size() &&
                    g_lru_compute_pending[dev][s];
                if (s >= 0 && s < g_lru_num_slots[dev] && !pending &&
                    cuEventQuery(g_lru_slot_events[dev][s]) == CUDA_SUCCESS) {
                    /* Stage 4.5 hot protection: prefer candidates idle
                     * longer than VITRIOL_LRU_HOT_MS; remember the plain
                     * LRU fallback in case every candidate is hot. */
                    if (lru_fallback < 0) lru_fallback = s;
                    int64_t lu = (s < (int) g_lru_last_use[dev].size()) ?
                        g_lru_last_use[dev][s] : 0;
                    if (lu <= hot_cutoff) {
                        evict = *it;
                        found = true;
                        break;
                    }
                }
            }
            if (!found && lru_fallback >= 0) {
                for (auto it2 = lru_order.begin(); it2 != lru_order.end(); ++it2) {
                    auto mit2 = lru_map.find(*it2);
                    if (mit2 != lru_map.end() && mit2->second == lru_fallback) {
                        evict = *it2;
                        found = true;
                        break;
                    }
                }
            }
            if (!found) {
                /* Every slot's last-read event is still unfired: the compute
                 * stream lags the host by more than the whole pool. Falling
                 * back to lru_order.back() would overwrite a slot whose
                 * reader kernels have not executed yet and feed torn weights
                 * to the rest of the run. Drain the compute stream instead -
                 * every pending reader completes, then the true LRU is safe
                 * to refill. */
                cuStreamSynchronize(cstream);
                evict = lru_order.back();
            }
            lru_order.remove(evict);
            auto eit = lru_map.find(evict);
            /* A missing entry means map/order desync. Falling back to slot 0
             * would alias two keys onto one slot: silent weight corruption. */
            if (eit == lru_map.end())
                GGML_ABORT("VITRIOL LRU map/order desync");
            slot = eit->second;
            lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        lru_map[key] = slot;
        lru_order.push_front(key);
        if ((int) g_lru_last_use[dev].size() > slot) {
            g_lru_last_use[dev][slot] = ggml_time_ms(); // Stage 4.5 hot-set stamp
        }
        if ((int)g_lru_compute_pending[dev].size() > slot) {
            g_lru_compute_pending[dev][slot] = true;
        }
    }

    CUdeviceptr dst = g_lru_pool[dev] + (size_t)slot * g_lru_slot_size[dev];

    store_bounce_slot * bounce = nullptr;
    if (store_device) {
        bounce = store_bounce_acquire(dev, expert_size);
        if (!bounce)
            GGML_ABORT("VITRIOL failed to acquire store bounce slot");
        if (!store_read_bounce(tensor_base, expert_idx, expert_size, *bounce))
            GGML_ABORT("VITRIOL store H2D read failed");
    }

    /* Wait for compute to finish reading this slot before overwriting it */
    cuStreamWaitEvent(g_lru_stream[dev], g_lru_slot_events[dev][slot], 0);

    /* Async DMA on dedicated stream, then make compute stream wait */
    const void * copy_source = store_device ? bounce->aligned : expert_data;
    static const bool hash_dbg = getenv("VITRIOL_LRU_HASH") != nullptr;
    if (hash_dbg)
        fprintf(stderr, "VITRIOL_LRU_HASH dev=%d tensor=%p expert=%d size=%zu src=%016llx\n",
                dev, tensor_base, expert_idx, expert_size,
                vitriol_fnv1a(copy_source, expert_size));
    CUresult r = cuMemcpyHtoDAsync(dst, copy_source, expert_size, g_lru_stream[dev]);
    if (r != CUDA_SUCCESS)
        GGML_ABORT("VITRIOL LRU H2D failed");

    if (bounce) {
        r = cuEventRecord(bounce->event, g_lru_stream[dev]);
        if (r != CUDA_SUCCESS)
            GGML_ABORT("VITRIOL bounce event record failed");
    }

    r = cuEventRecord(g_lru_slot_events[dev][slot], g_lru_stream[dev]);
    if (r != CUDA_SUCCESS)
        GGML_ABORT("VITRIOL LRU event record failed");

    static const bool vram_hash_dbg = getenv("VITRIOL_LRU_VRAM_HASH") != nullptr;
    if (vram_hash_dbg) {
        /* Verify the DMA landed the exact bytes: block until the H2D is
         * done, read the slot back over D2H, hash all three sides:
         * expected (host expert_data), copy_source (what the DMA read),
         * and the VRAM slot. A bounce/overwrite race shows up as
         * source != expected; a DMA failure as vram != source. */
        if (cuStreamSynchronize(g_lru_stream[dev]) != CUDA_SUCCESS)
            GGML_ABORT("VITRIOL vram-hash sync failed");
        std::vector<char> vbuf(expert_size);
        if (cudaMemcpy(vbuf.data(), (const void *) dst, expert_size,
                       cudaMemcpyDeviceToHost) != cudaSuccess)
            GGML_ABORT("VITRIOL vram-hash D2H failed");
        unsigned long long vh = vitriol_fnv1a(vbuf.data(), expert_size);
        unsigned long long sh = vitriol_fnv1a(copy_source, expert_size);
        unsigned long long eh = vitriol_fnv1a(expert_data, expert_size);
        const char * verdict = (sh == eh) ?
            (vh == eh ? "OK" : "VRAM_BAD") : "SRC_BAD";
        if (strcmp(verdict, "OK") != 0)
            fprintf(stderr, "VITRIOL_VHASH %s dev=%d tensor=%p expert=%d size=%zu exp=%016llx src=%016llx vram=%016llx\n",
                    verdict, dev, tensor_base, expert_idx, expert_size, eh, sh, vh);
    }

    r = cuStreamWaitEvent(cstream, g_lru_slot_events[dev][slot], 0);
    if (r != CUDA_SUCCESS)
        GGML_ABORT("VITRIOL LRU compute wait failed");

    if (trace && (!trace_base || (uintptr_t) tensor_base == trace_base)) {
        static const char* te = getenv("VITRIOL_LRU_TRACE_EXPERTS");
        static std::set<int> experts;
        if (te && experts.empty()) {
            for (char* s = strdup(te); ; ) {
                char* p = strchr(s, ','); if (p) *p = 0;
                experts.insert(atoi(s));
                if (!p) break; s = p + 1;
            }
        }
        if (experts.empty() || experts.count(expert_idx)) {
            unsigned first = 0;
            cuMemcpyDtoH(&first, dst, 4);
            trace_log("MISS-FILLED", slot, dst, first);
        }
    }

    if (vitriol_lru_strict()) {
        r = cuStreamSynchronize(g_lru_stream[dev]);
        if (r != CUDA_SUCCESS)
            GGML_ABORT("VITRIOL LRU strict sync failed");
    }

    return dst;
}

static void vitriol_lru_prefetch_async(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size)
{
    /* Fire-and-forget: submit DMA on LRU stream but do NOT block
     * any compute stream.  The per-expert loop's vitriol_lru_ensure
     * will wait on the event when it actually needs the data. */
    if (!expert_data || expert_size == 0 || !tensor_base)
        return;
    int dev = vitriol_current_device();
    if (!lru_ensure_stream(dev))
        return;
    if (!lru_init_pool(expert_size, dev))
        return;
    if (expert_size > g_lru_slot_size[dev])
        return;

    LRUKey key = { dev, (uintptr_t)tensor_base, expert_idx };

    /* Check cache — if already present, DMA was already submitted */
    auto & lru_map   = g_lru_map[dev];
    auto & lru_order = g_lru_order[dev];
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if (lru_map.find(key) != lru_map.end())
            return;
    }

    cudaSetDevice(dev);

    /* Allocate or evict slot */
    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if (g_lru_next_fresh_slot[dev] < g_lru_num_slots[dev]) {
            slot = g_lru_next_fresh_slot[dev]++;
        } else {
            LRUKey evict = lru_order.back();
            lru_order.pop_back();
            auto eit = lru_map.find(evict);
            if (eit == lru_map.end())
                GGML_ABORT("VITRIOL prefetch LRU map/order desync");
            slot = eit->second;
            lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        lru_map[key] = slot;
        lru_order.push_front(key);
        if ((int)g_lru_compute_pending[dev].size() > slot) {
            g_lru_compute_pending[dev][slot] = true;
        }
    }

    CUdeviceptr dst = g_lru_pool[dev] + (size_t)slot * g_lru_slot_size[dev];

    /* Wait for compute to finish with this slot before overwriting */
    cuStreamWaitEvent(g_lru_stream[dev], g_lru_slot_events[dev][slot], 0);

    /* Async DMA — no cuStreamWaitEvent, compute stream NOT blocked */
    CUresult r = cuMemcpyHtoDAsync(dst, expert_data, expert_size, g_lru_stream[dev]);
    if (r != CUDA_SUCCESS) return;
    cuEventRecord(g_lru_slot_events[dev][slot], g_lru_stream[dev]);
}

void vitriol_lru_prefetch(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size,
    CUstream       compute_stream)
{
    (void)compute_stream;
    if (vitriol_store_source_device()) return;
    if (vitriol_lazy_lock_active()) {
        vitriol_ensure_expert_locked(tensor_base, expert_idx, expert_size);
    }
    vitriol_lru_prefetch_async(tensor_base, expert_idx, expert_data, expert_size);
}

ggml_backend_buffer_type_t vitriol_get_expert_buffer_type(void) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM)
        return NULL;
    if (g_vitriol_config.max_locked_mb > 0) {
        return vitriol_get_buffer_type_lazy(0);
    }
    return vitriol_get_buffer_type(0);
}

/* Device-aware variant for multi-GPU layer splits. The loader resolves which
 * GPU owns the current layer's expert tensor and asks for that device's
 * buffer type, instead of always landing on device 0. */
ggml_backend_buffer_type_t vitriol_get_expert_buffer_type_dev(int device) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM)
        return NULL;
    if (g_vitriol_config.max_locked_mb > 0) {
        return vitriol_get_buffer_type_lazy(device);
    }
    return vitriol_get_buffer_type(device);
}

void vitriol_lru_mark_compute_done(CUdeviceptr vram_ptr, CUstream stream) {
    if (vitriol_rust_enabled()) {
        vs_lru_mark_compute_done((usize_t_rs) vram_ptr, (void *) stream);
        return;
    }
    if (!vram_ptr)
        return;
    /* Find which device's pool this pointer belongs to, then record the
     * completion event on that device's slot-events array. */
    for (int dev = 0; dev < GGML_CUDA_MAX_DEVICES; dev++) {
        if (!g_lru_slot_events[dev] || g_lru_pool[dev] == 0)
            continue;
        long long offset = (long long)(vram_ptr - g_lru_pool[dev]);
        int slot = (int)(offset / g_lru_slot_size[dev]);
        if (offset >= 0 && slot >= 0 && slot < g_lru_num_slots[dev] &&
            offset % (long long)g_lru_slot_size[dev] == 0) {
            cudaSetDevice(dev);
            /* stream is the compute stream on the same device (layer split
             * keeps a tensor's compute and cache on one GPU). */
            cuEventRecord(g_lru_slot_events[dev][slot], stream);
            if ((int)g_lru_compute_pending[dev].size() > slot) {
                g_lru_compute_pending[dev][slot] = false;
            }
            return;
        }
    }
}

void vitriol_cuda_cleanup_vram(void) {
    for (int dev = 0; dev < GGML_CUDA_MAX_DEVICES; dev++) {
        {
            std::lock_guard<std::mutex> lock(g_store_bounce_mtx[dev]);
            for (auto & slot : g_store_bounce[dev]) {
                if (slot.event) {
                    cudaSetDevice(dev);
                    cuEventDestroy(slot.event);
                }
                if (slot.allocation) {
                    cudaSetDevice(dev);
                    cudaFreeHost(slot.allocation);
                }
            }
            g_store_bounce[dev].clear();
            g_store_bounce_next[dev] = 0;
        }
        if (g_lru_slot_events[dev]) {
            cudaSetDevice(dev);
            for (int i = 0; i < g_lru_num_slots[dev]; i++)
                cuEventDestroy(g_lru_slot_events[dev][i]);
            delete[] g_lru_slot_events[dev];
            g_lru_slot_events[dev] = nullptr;
        }
        if (g_lru_stream[dev] != 0) {
            cudaSetDevice(dev);
            cuStreamDestroy(g_lru_stream[dev]);
            g_lru_stream[dev] = 0;
        }
        if (g_lru_pool[dev] != 0) {
            cudaSetDevice(dev);
            CUresult r = cuMemFree(g_lru_pool[dev]);
            if (r != CUDA_SUCCESS) {
                fprintf(stderr, "VITRIOL: cuMemFree(LRU pool, GPU %d) failed: %d\n", dev, (int)r);
            }
            g_lru_pool[dev] = 0;
        }
        if (g_output_cache_pool[dev] != 0) {
            cudaSetDevice(dev);
            CUresult r = cuMemFree(g_output_cache_pool[dev]);
            if (r != CUDA_SUCCESS) {
                fprintf(stderr, "VITRIOL: cuMemFree(output cache pool, GPU %d) failed: %d\n", dev, (int)r);
            }
            g_output_cache_pool[dev] = 0;
        }
        /* Free per-device pin pool (single allocation — all tensors) */
        if (g_pin_pool[dev] != 0) {
            cudaSetDevice(dev);
            CUresult r = cuMemFree(g_pin_pool[dev]);
            if (r != CUDA_SUCCESS) {
                fprintf(stderr, "VITRIOL: cuMemFree(pin pool, GPU %d) failed: %d\n", dev, (int)r);
            }
            g_pin_pool[dev] = 0;
            g_pin_pool_offset[dev] = 0;
            g_pin_pool_total[dev] = 0;
        }
    }
    {
        std::lock_guard<std::mutex> lock(g_pin_mtx);
        g_pin_map.clear();
        g_pinned_bytes = 0;
        g_vitriol_config.pin_active = false;
    }
}

void vitriol_cuda_print_stats(void) {
    if (vitriol_rust_enabled()) {
        vs_print_stats();
        /* Fall through: C++ counters are idle in Rust mode but cheap to
         * print for provenance. */
    }
    uint64_t total = g_lru_stats.hits + g_lru_stats.misses;
    float hr = (total > 0) ? 100.0f * (float)g_lru_stats.hits / (float)total : 0.0f;
    printf("=== VITRIOL Statistics ===\n");
    printf("Mode: %d\n", g_vitriol_config.mode);
    printf("Store Source: %s\n", vitriol_store_source_device() ? "store" : "gguf");
    for (int has_pool = 0; has_pool < GGML_CUDA_MAX_DEVICES; has_pool++) {
        if (g_lru_pool[has_pool] != 0) {
            printf("LRU Cache (GPU %d): pool=%llu MB, slots=%d, slot_size=%zu\n",
                   has_pool,
                   (unsigned long long)(g_lru_pool_size[has_pool] / 1024 / 1024),
                   g_lru_num_slots[has_pool], g_lru_slot_size[has_pool]);
        }
    }
    printf("LRU Hits: %llu\n", g_lru_stats.hits);
    printf("LRU Misses: %llu\n", g_lru_stats.misses);
    printf("LRU Hit Rate: %.2f%%\n", hr);
    printf("LRU Evictions: %llu\n", g_lru_stats.evictions);
    printf("Predictor: %s\n", g_vitriol_config.async_prefetch ? "cross-layer + temporal" : "none");
    printf("Output Cache: %s\n", g_vitriol_config.output_cache ? "enabled (approximate)" : "none");
    printf("Expert Pinning: %s\n", g_vitriol_config.pin_active ? "active" :
           g_vitriol_config.pin_first_n_layers > 0 ? "configured but no layers pinned yet" : "none");
    if (g_vitriol_config.pin_active) {
        printf("Pinned Tensors: %zu\n", g_pin_map.size());
        printf("Pinned Memory: %zu MB (%.1f GiB)\n",
               g_pinned_bytes / 1024 / 1024,
               (double)g_pinned_bytes / (1024.0 * 1024.0 * 1024.0));
    }
    if (g_vitriol_config.output_cache) {
        uint64_t oc_total = g_output_cache_stats.hits + g_output_cache_stats.misses;
        float oc_hr = (oc_total > 0) ? 100.0f * (float)g_output_cache_stats.hits / (float)oc_total : 0.0f;
        printf("Output Cache Hits: %llu\n", g_output_cache_stats.hits);
        printf("Output Cache Misses: %llu\n", g_output_cache_stats.misses);
        printf("Output Cache Hit Rate: %.2f%%\n", oc_hr);
    }
    {
        std::lock_guard<std::mutex> lock(g_locked_mtx);
        printf("Locked Slices: %zu\n", g_locked_slices.size());
        printf("Locked MB: %llu\n", static_cast<unsigned long long>(g_locked_bytes / 1024 / 1024));
        printf("Orphan Registrations: %zu\n", g_orphan_registrations.size());
    }
    printf("Store Fills: %llu\n", static_cast<unsigned long long>(g_store_stats.fills));
    printf("Store Bytes: %llu\n", static_cast<unsigned long long>(g_store_stats.bytes));
    printf("Store Failures: %llu\n", static_cast<unsigned long long>(g_store_stats.failures));
    printf("Store Read ms: %llu\n", static_cast<unsigned long long>(g_store_stats.read_ns / 1000000ULL));
    printf("Strategy: %s\n", vitriol_store_source_device() ?
           "Optane store -> pinned bounce -> LRU VRAM" : "RAM Shot + LRU VRAM cache");
    printf("===============================\n");
}
