#include "vitriol-cuda-integration.h"
#include "vitriol-buffer.h"
#include "ggml-cuda.h"
#include "ggml-backend.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unordered_map>
#include <list>
#include <mutex>
#include <cuda.h>

vitriol_config_t g_vitriol_config;

static struct vitriol_config_init {
    vitriol_config_init() {
        memset(&g_vitriol_config, 0, sizeof(g_vitriol_config));
        g_vitriol_config.mode = VITRIOL_MODE_DISABLED;
        g_vitriol_config.prefetch_ahead = 1;
        g_vitriol_config.static_layers = 15;
        g_vitriol_config.window_size_mb = 256;
        g_vitriol_config.use_double_buffer = true;
        g_vitriol_config.buffer_count = 2;
    }
} s_vitriol_config_init;

/* ── LRU Cache ──────────────────────────────────────────────────── */

#define VITRIOL_LRU_POOL_SIZE  (512ULL * 1024 * 1024)  // 512 MB VRAM pool
#define VITRIOL_LRU_MAX_SLOTS  128

static CUdeviceptr g_lru_pool = 0;
static size_t      g_lru_pool_size = 0;
static size_t      g_lru_slot_size = 0;
static int         g_lru_num_slots = 0;

/* Composite key: (tensor_base_address, expert_idx) prevents
 * cross-layer collisions where expert 0 of layer 1 != expert 0 of layer 2. */
struct LRUKey {
    uintptr_t tensor_base;
    int       expert_idx;
    bool operator==(const LRUKey &o) const {
        return tensor_base == o.tensor_base && expert_idx == o.expert_idx;
    }
};

struct LRUKeyHash {
    size_t operator()(const LRUKey &k) const {
        return (size_t)(k.tensor_base * 2654435761U) ^ (size_t)k.expert_idx;
    }
};

static std::unordered_map<LRUKey, int, LRUKeyHash> g_lru_map;
static std::list<LRUKey> g_lru_order;
static std::mutex        g_lru_mtx;

/* Dedicated stream + event for async DMA */
static CUstream  g_lru_stream = 0;
static CUevent   g_lru_event  = 0;

static struct LRUStats {
    unsigned long long hits;
    unsigned long long misses;
    unsigned long long evictions;
} g_lru_stats;

static bool lru_init_pool(size_t min_expert_size);
static bool lru_ensure_stream(void);

/* ── Initialization ──────────────────────────────────────────────── */

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

    if (g_vitriol_config.mode == VITRIOL_MODE_STREAM) {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: stream mode — page-locked host RAM + LRU VRAM cache\n");
    }

    memset(&g_lru_stats, 0, sizeof(g_lru_stats));
}

static bool lru_ensure_stream(void) {
    if (g_lru_stream != 0)
        return true;
    CUresult r;
    r = cuStreamCreate(&g_lru_stream, CU_STREAM_NON_BLOCKING);
    if (r != CUDA_SUCCESS) return false;
    r = cuEventCreate(&g_lru_event, CU_EVENT_DISABLE_TIMING);
    if (r != CUDA_SUCCESS) return false;
    return true;
}

/* (Re-)initialize VRAM pool.  If already allocated and the new
 * min_expert_size exceeds the current slot, free and reallocate. */
static bool lru_init_pool(size_t min_expert_size) {
    const char* pool_env = getenv("VITRIOL_LRU_MB");
    size_t pool_size = VITRIOL_LRU_POOL_SIZE;
    if (pool_env) {
        unsigned long mb = strtoul(pool_env, NULL, 10);
        if (mb >= 64) pool_size = mb * 1024ULL * 1024;
    }

    size_t needed_slot = (min_expert_size + 255) & ~(size_t)255;
    int    needed_slots = (int)(pool_size / needed_slot);
    if (needed_slots > VITRIOL_LRU_MAX_SLOTS) needed_slots = VITRIOL_LRU_MAX_SLOTS;
    if (needed_slots < 1) needed_slots = 1;

    /* Already allocated with at least the needed slot size? */
    if (g_lru_pool != 0 && needed_slot <= g_lru_slot_size)
        return true;

    /* Reallocate */
    if (g_lru_pool != 0) {
        cuMemFree(g_lru_pool);
        g_lru_pool = 0;
        g_lru_map.clear();
        g_lru_order.clear();
    }

    CUresult err = cuMemAlloc(&g_lru_pool, pool_size);
    if (err != CUDA_SUCCESS) {
        fprintf(stderr, "VITRIOL: LRU pool alloc %zu MB failed (%d)\n",
                pool_size / 1024 / 1024, (int)err);
        g_lru_pool = 0;
        return false;
    }

    g_lru_pool_size  = pool_size;
    g_lru_slot_size  = needed_slot;
    g_lru_num_slots  = needed_slots;

    if (g_vitriol_config.verbose)
        printf("VITRIOL: LRU pool %zu MB, %d slots x %zu bytes\n",
               pool_size / 1024 / 1024, g_lru_num_slots, g_lru_slot_size);

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
    if (!expert_data || expert_size == 0 || !tensor_base)
        return 0;

    if (!lru_ensure_stream())
        return 0;
    if (!lru_init_pool(expert_size))
        return 0;

    /* If expert doesn't fit even after realloc, skip cache. */
    if (expert_size > g_lru_slot_size)
        return 0;

    LRUKey key = { (uintptr_t)tensor_base, expert_idx };

    /* Check cache */
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        auto it = g_lru_map.find(key);
        if (it != g_lru_map.end()) {
            g_lru_order.remove(key);
            g_lru_order.push_front(key);
            g_lru_stats.hits++;
            return g_lru_pool + (size_t)it->second * g_lru_slot_size;
        }
    }

    /* Cache miss */
    g_lru_stats.misses++;

    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if ((int)g_lru_map.size() < g_lru_num_slots) {
            slot = (int)g_lru_map.size();
        } else {
            LRUKey evict = g_lru_order.back();
            g_lru_order.pop_back();
            auto eit = g_lru_map.find(evict);
            slot = (eit != g_lru_map.end()) ? eit->second : 0;
            if (eit != g_lru_map.end()) g_lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        g_lru_map[key] = slot;
        g_lru_order.push_front(key);
    }

    CUdeviceptr dst = g_lru_pool + (size_t)slot * g_lru_slot_size;

    /* Async DMA on dedicated stream, then make compute stream wait */
    CUresult r;
    r = cuMemcpyHtoDAsync(dst, expert_data, expert_size, g_lru_stream);
    if (r != CUDA_SUCCESS) return 0;

    r = cuEventRecord(g_lru_event, g_lru_stream);
    if (r != CUDA_SUCCESS) return 0;

    r = cuStreamWaitEvent(compute_stream, g_lru_event, 0);
    if (r != CUDA_SUCCESS) return 0;

    return dst;
}

void vitriol_lru_prefetch(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size,
    CUstream       compute_stream)
{
    /* Fire-and-forget: load into LRU but don't wait on compute_stream.
     * If already cached this is a no-op (the lookup/promotion still happens). */
    vitriol_lru_ensure(tensor_base, expert_idx, expert_data, expert_size, compute_stream);
}

ggml_backend_buffer_type_t vitriol_get_expert_buffer_type(void) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM)
        return NULL;
    return vitriol_get_buffer_type(0);
}

void vitriol_cuda_print_stats(void) {
    printf("=== VITRIOL Statistics ===\n");
    printf("Mode: %d\n", g_vitriol_config.mode);
    printf("LRU Cache: pool=%llu MB, slots=%d, slot_size=%zu\n",
           (unsigned long long)(g_lru_pool_size / 1024 / 1024),
           g_lru_num_slots, g_lru_slot_size);
    printf("LRU Hits: %llu\n", g_lru_stats.hits);
    printf("LRU Misses: %llu\n", g_lru_stats.misses);
    printf("LRU Evictions: %llu\n", g_lru_stats.evictions);
    printf("Strategy: RAM Shot + LRU VRAM cache\n");
    printf("===============================\n");
}
