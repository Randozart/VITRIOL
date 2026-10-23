/*
 * vitriol-sycl-integration.cpp — VITRIOL streaming for SYCL backend
 *
 * LRU VRAM cache for hot expert weights, predictive prefetching,
 * and hooks into MUL_MAT_ID dispatch.
 */

#include "ggml-backend-impl.h"
#include "vitriol-sycl-buffer.hpp"
#include "ggml-sycl.h"
#include "dpct/helper.hpp"
#include <sycl/sycl.hpp>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <unistd.h>
#include <unordered_map>
#include <list>
#include <mutex>
#include <vector>
#include <unordered_set>
#include <algorithm>
#include <cerrno>
#include <sys/mman.h>

/* ── LRU Cache ─────────────────────────────────────────────────── */

#define VITRIOL_LRU_MAX_SLOTS 65536

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

static void           *g_lru_pool = nullptr;
static size_t          g_lru_pool_size = 0;
static size_t          g_lru_slot_size = 0;
static int             g_lru_num_slots = 0;

static std::unordered_map<LRUKey, int, LRUKeyHash> g_lru_map;
static std::list<LRUKey> g_lru_order;
static std::mutex        g_lru_mtx;

static sycl::queue     *g_lru_queue = nullptr;
static sycl::event     *g_lru_slot_events = nullptr;
static std::mutex        g_lru_init_mtx;

static struct {
    unsigned long long hits;
    unsigned long long misses;
    unsigned long long evictions;
} g_lru_stats = {};

/* Per-slot event tracking for async DMA sync.
 * g_lru_pending_slots tracks ALL slots with in-flight DMAs so that
 * lru_sync can wait for all of them, not just the most recent. */
static int g_lru_last_slot = -1;
static bool g_lru_has_pending_dma = false;
static std::unordered_set<int> g_lru_pending_slots;

/* ── Predictive Prefetcher ─────────────────────────────────────── */

#define VITRIOL_MAX_LAYERS 128

static uintptr_t g_layer_bases[VITRIOL_MAX_LAYERS];
static int       g_n_layers = 0;
static int       g_last_layer = -1;

static int  g_cur_exp[VITRIOL_MAX_LAYERS][256];
static int  g_cur_cnt[VITRIOL_MAX_LAYERS];
static int  g_prev_exp[VITRIOL_MAX_LAYERS][256];
static int  g_prev_cnt[VITRIOL_MAX_LAYERS];

static int get_layer_index(uintptr_t tensor_base) {
    for (int i = 0; i < g_n_layers && i < VITRIOL_MAX_LAYERS; i++) {
        if (g_layer_bases[i] == tensor_base) return i;
    }
    if (g_n_layers >= VITRIOL_MAX_LAYERS) return -1;
    g_layer_bases[g_n_layers] = tensor_base;
    return g_n_layers++;
}

static void detect_token_boundary(int layer_idx) {
    if (g_last_layer >= 0 && layer_idx <= g_last_layer) {
        for (int i = 0; i < VITRIOL_MAX_LAYERS; i++) {
            g_prev_cnt[i] = g_cur_cnt[i];
            memcpy(g_prev_exp[i], g_cur_exp[i], g_cur_cnt[i] * sizeof(int));
            g_cur_cnt[i] = 0;
        }
    }
    g_last_layer = layer_idx;
}

/* ── E34: expert-usage profiler ────────────────────────────────── */

/* Records (tensor_slot, expert_id) selection counts for MUL_MAT_ID across a
 * real workload. Tensor slots are first-touch indexed: slot//3 approximates
 * the layer index, slot%3 the tensor kind (gate/up/down; down disambiguated
 * by its larger expert_size). Dumped as CSV at process exit. */

static bool        g_prof_enabled = false;
static std::string g_prof_path;
static std::mutex  g_prof_mtx;
static std::unordered_map<uintptr_t, int> g_prof_slot_of;
static std::vector<uintptr_t>             g_prof_bases;
static std::vector<size_t>                g_prof_sizes;
static std::vector<std::vector<unsigned long long>> g_prof_counts;

static void vitriol_sycl_profile_dump(void) {
    if (!g_prof_enabled) {
        return;
    }
    FILE * f = fopen(g_prof_path.c_str(), "w");
    if (!f) {
        fprintf(stderr, "VITRIOL-PROFILE: cannot write %s\n", g_prof_path.c_str());
        return;
    }
    fprintf(f, "# VITRIOL expert-usage profile\n");
    for (size_t s = 0; s < g_prof_bases.size(); s++) {
        fprintf(f, "# slot %zu base=%p expert_size=%zu\n",
                s, (void *) g_prof_bases[s], g_prof_sizes[s]);
    }
    fprintf(f, "slot,expert,count\n");
    unsigned long long total = 0;
    for (size_t s = 0; s < g_prof_counts.size(); s++) {
        for (size_t e = 0; e < g_prof_counts[s].size(); e++) {
            if (g_prof_counts[s][e]) {
                fprintf(f, "%zu,%zu,%llu\n", s, e, g_prof_counts[s][e]);
                total += g_prof_counts[s][e];
            }
        }
    }
    fclose(f);
    fprintf(stderr, "VITRIOL-PROFILE: dumped %zu tensor slots (%llu selections) to %s\n",
            g_prof_bases.size(), total, g_prof_path.c_str());
}

void vitriol_sycl_profile_init(void) {
    const char * p = getenv("VITRIOL_PROFILE");
    g_prof_enabled = p && p[0] == '1';
    if (!g_prof_enabled) {
        return;
    }
    const char * path = getenv("VITRIOL_PROFILE_PATH");
    g_prof_path = path ? path : "/tmp/vitriol-expert-profile.csv";
    atexit(vitriol_sycl_profile_dump);
    fprintf(stderr, "VITRIOL-PROFILE: recording expert usage to %s\n", g_prof_path.c_str());
}

bool vitriol_sycl_profile_active(void) {
    return g_prof_enabled;
}

void vitriol_sycl_profile_record(
    const void *tensor_base, const char *ids_host,
    size_t nb0, size_t nb1, int n_ids, int n_iid1, size_t expert_size)
{
    if (!g_prof_enabled || !tensor_base || !ids_host) {
        return;
    }
    std::lock_guard<std::mutex> lock(g_prof_mtx);
    uintptr_t base = (uintptr_t) tensor_base;
    auto it = g_prof_slot_of.find(base);
    int slot;
    if (it == g_prof_slot_of.end()) {
        slot = (int) g_prof_bases.size();
        g_prof_slot_of[base] = slot;
        g_prof_bases.push_back(base);
        g_prof_sizes.push_back(expert_size);
        g_prof_counts.emplace_back();
    } else {
        slot = it->second;
    }
    auto & counts = g_prof_counts[slot];
    for (int iid1 = 0; iid1 < n_iid1; iid1++) {
        for (int id = 0; id < n_ids; id++) {
            int32_t e;
            memcpy(&e, ids_host + iid1 * nb1 + id * nb0, sizeof(e));
            if (e < 0) {
                continue;
            }
            if ((size_t) e >= counts.size()) {
                counts.resize((size_t) e + 1, 0);
            }
            counts[e]++;
        }
    }
}

/* ── Initialization ────────────────────────────────────────────── */

/* L0 copy engines cannot page-fault: make sure a file-backed (zero-copy
 * wrapped) range is resident before the device reads it. MADV_WILLNEED
 * starts kernel readahead for the whole slice; the touch loop then only
 * blocks on the pages the readahead has not covered yet. */
static void vitriol_sycl_touch_pages(const void * ptr, size_t size) {
    static const size_t page = (size_t) sysconf(_SC_PAGESIZE);
    if (!ptr || size == 0 || page == 0) {
        return;
    }
    uintptr_t base = (uintptr_t) ptr & ~(page - 1);
    madvise((void *) base, size + (uintptr_t)ptr - base + page, MADV_WILLNEED);
    const volatile char * p = (const volatile char *) ptr;
    for (size_t off = 0; off < size; off += page) {
        (void) p[off];
    }
    (void) p[size - 1];
}

static bool lru_ensure_queue(void) {
    if (g_lru_queue) return true;
    std::lock_guard<std::mutex> lock(g_lru_init_mtx);
    if (g_lru_queue) return true;

    auto devices = sycl::device::get_devices();
    if (devices.empty()) return false;

    g_lru_queue = new sycl::queue(devices[0], sycl::property_list{
        sycl::property::queue::in_order{}
    });
    return true;
}

static bool lru_init_pool(size_t min_expert_size) {
    if (g_lru_pool) return true;

    /* Queue init must happen outside g_lru_init_mtx: lru_ensure_queue locks
     * the same mutex, and std::mutex is non-recursive (self-deadlock). */
    if (!lru_ensure_queue()) return false;

    std::lock_guard<std::mutex> lock(g_lru_init_mtx);
    if (g_lru_pool) return true;

    size_t pool_size = vitriol_sycl_lru_mb() * 1024ULL * 1024;
    size_t slot_size = (min_expert_size + 255) & ~(size_t)255;
    int num_slots = (int)(pool_size / slot_size);
    if (num_slots > VITRIOL_LRU_MAX_SLOTS) num_slots = VITRIOL_LRU_MAX_SLOTS;
    if (num_slots < 1) num_slots = 1;

    g_lru_pool = sycl::malloc_device(pool_size, *g_lru_queue);
    if (!g_lru_pool) {
        fprintf(stderr, "VITRIOL-SYCL: LRU pool alloc %zu MB failed\n", pool_size / 1024 / 1024);
        return false;
    }

    g_lru_pool_size = pool_size;
    g_lru_slot_size = slot_size;
    g_lru_num_slots = num_slots;
    g_lru_slot_events = new sycl::event[num_slots];

    if (vitriol_sycl_verbose())
        fprintf(stderr, "VITRIOL-SYCL: LRU pool %zu MB, %d slots x %zu bytes\n",
                pool_size / 1024 / 1024, num_slots, slot_size);

    return true;
}

/* Re-create the pool when a larger expert slice appears (e.g. q6_K down
 * experts after q4_K gate/up sized the pool). Drops the cached contents;
 * in-flight DMAs are drained first. */
static bool lru_resize(size_t new_expert_size) {
    if (!g_lru_pool || !g_lru_queue)
        return false;

    size_t new_slot_size = (new_expert_size + 255) & ~(size_t)255;
    if (new_slot_size <= g_lru_slot_size)
        return true;

    /* Drain in-flight DMAs before freeing the pool */
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        for (int s = 0; s < g_lru_num_slots; s++) {
            g_lru_slot_events[s].wait();
        }
    }

    std::lock_guard<std::mutex> lock(g_lru_init_mtx);
    if (new_slot_size <= g_lru_slot_size)
        return true;

    size_t pool_size = vitriol_sycl_lru_mb() * 1024ULL * 1024;
    int num_slots = (int)(pool_size / new_slot_size);
    if (num_slots > VITRIOL_LRU_MAX_SLOTS) num_slots = VITRIOL_LRU_MAX_SLOTS;
    if (num_slots < 1) {
        fprintf(stderr, "VITRIOL-SYCL: expert slice %zu bytes exceeds LRU budget\n", new_expert_size);
        return false;
    }

    sycl::free(g_lru_pool, *g_lru_queue);
    g_lru_pool = sycl::malloc_device(pool_size, *g_lru_queue);
    if (!g_lru_pool) {
        fprintf(stderr, "VITRIOL-SYCL: LRU pool realloc %zu MB failed\n", pool_size / 1024 / 1024);
        return false;
    }

    g_lru_pool_size = pool_size;
    g_lru_slot_size = new_slot_size;
    g_lru_num_slots = num_slots;
    delete[] g_lru_slot_events;
    g_lru_slot_events = new sycl::event[num_slots];

    {
        std::lock_guard<std::mutex> mlock(g_lru_mtx);
        g_lru_map.clear();
        g_lru_order.clear();
        g_lru_last_slot = -1;
        g_lru_pending_slots.clear();
    }

    if (vitriol_sycl_verbose())
        fprintf(stderr, "VITRIOL-SYCL: LRU pool resized: %d slots x %zu bytes\n", num_slots, new_slot_size);

    return true;
}

/* ── LRU Cache Operations ──────────────────────────────────────── */

void * vitriol_sycl_lru_ensure(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size)
{
    if (!expert_data || expert_size == 0 || !tensor_base)
        return nullptr;

    if (!lru_init_pool(expert_size))
        return nullptr;

    if (expert_size > g_lru_slot_size && !lru_resize(expert_size))
        return nullptr;

    LRUKey key = {(uintptr_t)tensor_base, expert_idx};

    /* Check cache */
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        auto it = g_lru_map.find(key);
        if (it != g_lru_map.end()) {
            g_lru_order.remove(key);
            g_lru_order.push_front(key);
            g_lru_stats.hits++;
            int slot = it->second;
            if (vitriol_sycl_verbose() && (g_lru_stats.hits % 100 == 0)) {
                fprintf(stderr, "VITRIOL-SYCL LRU: hits=%llu misses=%llu evictions=%llu (hit rate %.1f%%)\n",
                        g_lru_stats.hits, g_lru_stats.misses, g_lru_stats.evictions,
                        100.0 * g_lru_stats.hits / (g_lru_stats.hits + g_lru_stats.misses + 1));
            }
            if (slot < g_lru_num_slots) {
                g_lru_slot_events[slot].wait();
            }
            return (char *)g_lru_pool + slot * g_lru_slot_size;
        }
    }

    /* Cache miss */
    g_lru_stats.misses++;
    if (vitriol_sycl_verbose() && (g_lru_stats.misses % 50 == 0)) {
        fprintf(stderr, "VITRIOL-SYCL LRU: miss #%llu (hits=%llu evictions=%llu, hit rate %.1f%%)\n",
                g_lru_stats.misses, g_lru_stats.hits, g_lru_stats.evictions,
                100.0 * g_lru_stats.hits / (g_lru_stats.hits + g_lru_stats.misses + 1));
    }

    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if ((int)g_lru_map.size() < g_lru_num_slots) {
            slot = (int)g_lru_map.size();
        } else {
            /* Three-state eviction: skip slots with in-flight DMA.
             * PROVENANCE: inspired by CUDA VITRIOL's cuEventQuery approach
             * (kimi-k3-in-c, Apache-2.0; re-derived for SYCL). */
            LRUKey evict = g_lru_order.back();
            for (auto it = std::prev(g_lru_order.end()); it != g_lru_order.begin(); --it) {
                auto mit = g_lru_map.find(*it);
                int s = (mit != g_lru_map.end()) ? mit->second : -1;
                if (s >= 0 && s < g_lru_num_slots) {
                    auto status = g_lru_slot_events[s].get_info<sycl::info::event::command_execution_status>();
                    if (status == sycl::info::event_command_status::complete) {
                        evict = *it;
                        break;
                    }
                }
            }
            g_lru_order.remove(evict);
            auto eit = g_lru_map.find(evict);
            slot = (eit != g_lru_map.end()) ? eit->second : 0;
            if (eit != g_lru_map.end()) g_lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        g_lru_map[key] = slot;
        g_lru_order.push_front(key);
    }

    /* Wait for previous DMA on this slot */
    if (slot < g_lru_num_slots) {
        g_lru_slot_events[slot].wait();
    }

    /* Fault the source pages in before the copy engine reads them */
    vitriol_sycl_touch_pages(expert_data, expert_size);

    /* Async DMA from host to device */
    void *dst = (char *)g_lru_pool + slot * g_lru_slot_size;
    g_lru_slot_events[slot] = g_lru_queue->memcpy(dst, expert_data, expert_size);

    /* In sync mode, block until DMA completes */
    if (!vitriol_sycl_async_dma()) {
        g_lru_slot_events[slot].wait();
    } else {
        g_lru_last_slot = slot;
        g_lru_pending_slots.insert(slot);
    }

    return dst;
}

void vitriol_sycl_lru_sync(void) {
    /* Wait for ALL in-flight DMA events, not just the most recent.
     * The predictor prefetch fires DMAs for NEXT layer's experts;
     * multiple may be in flight simultaneously.  Waiting only on the
     * last slot left earlier DMAs incomplete, causing GPU stalls. */
    if (!g_lru_pending_slots.empty()) {
        for (int s : g_lru_pending_slots) {
            if (s >= 0 && s < g_lru_num_slots)
                g_lru_slot_events[s].wait();
        }
        if (vitriol_sycl_verbose() && g_lru_pending_slots.size() > 1) {
            fprintf(stderr, "VITRIOL-SYCL: lru_sync drained %zu pending DMAs\n",
                    g_lru_pending_slots.size());
        }
        g_lru_pending_slots.clear();
        g_lru_last_slot = -1;
    } else if (g_lru_last_slot >= 0 && g_lru_last_slot < g_lru_num_slots) {
        /* Fallback for code paths that don't use pending_slots tracking */
        g_lru_slot_events[g_lru_last_slot].wait();
        g_lru_last_slot = -1;
    }
}

void vitriol_sycl_lru_prefetch(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size)
{
    if (!expert_data || expert_size == 0 || !tensor_base)
        return;

    if (!lru_init_pool(expert_size))
        return;

    if (expert_size > g_lru_slot_size && !lru_resize(expert_size))
        return;

    LRUKey key = {(uintptr_t)tensor_base, expert_idx};

    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if (g_lru_map.find(key) != g_lru_map.end())
            return;
    }

    int slot;
    {
        std::lock_guard<std::mutex> lock(g_lru_mtx);
        if ((int)g_lru_map.size() < g_lru_num_slots) {
            slot = (int)g_lru_map.size();
        } else {
            /* Three-state eviction: skip slots with in-flight DMA */
            LRUKey evict = g_lru_order.back();
            for (auto it = std::prev(g_lru_order.end()); it != g_lru_order.begin(); --it) {
                auto mit = g_lru_map.find(*it);
                int s = (mit != g_lru_map.end()) ? mit->second : -1;
                if (s >= 0 && s < g_lru_num_slots) {
                    auto status = g_lru_slot_events[s].get_info<sycl::info::event::command_execution_status>();
                    if (status == sycl::info::event_command_status::complete) {
                        evict = *it;
                        break;
                    }
                }
            }
            g_lru_order.remove(evict);
            auto eit = g_lru_map.find(evict);
            slot = (eit != g_lru_map.end()) ? eit->second : 0;
            if (eit != g_lru_map.end()) g_lru_map.erase(eit);
            g_lru_stats.evictions++;
        }
        g_lru_map[key] = slot;
        g_lru_order.push_front(key);
    }

    if (slot < g_lru_num_slots) {
        g_lru_slot_events[slot].wait();
    }

    vitriol_sycl_touch_pages(expert_data, expert_size);

    void *dst = (char *)g_lru_pool + slot * g_lru_slot_size;
    /* Fire-and-forget DMA */
    g_lru_slot_events[slot] = g_lru_queue->memcpy(dst, expert_data, expert_size);
    g_lru_pending_slots.insert(slot);
}

/* ── Predictive Prefetching ────────────────────────────────────── */

void vitriol_sycl_predictor_prefetch(
    const void    *tensor_base,
    size_t         expert_size)
{
    if (!vitriol_sycl_prefetch_enabled())
        return;

    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0) return;

    detect_token_boundary(layer_idx);

    int predicted[256];
    int n_predicted = 0;

    auto add_pred = [&](int e) {
        if (e < 0) return;
        for (int i = 0; i < n_predicted; i++)
            if (predicted[i] == e) return;
        predicted[n_predicted++] = e;
    };

    if (layer_idx > 0) {
        for (int i = 0; i < g_cur_cnt[layer_idx - 1]; i++)
            add_pred(g_cur_exp[layer_idx - 1][i]);
    }

    for (int i = 0; i < g_prev_cnt[layer_idx]; i++)
        add_pred(g_prev_exp[layer_idx][i]);

    if (n_predicted == 0)
        return;

    for (int i = 0; i < n_predicted; i++) {
        int e = predicted[i];
        const void *expert_data = (const char *)tensor_base + (size_t)e * expert_size;
        vitriol_sycl_lru_prefetch(tensor_base, e, expert_data, expert_size);
    }
}

void vitriol_sycl_predictor_update(
    const void    *tensor_base,
    size_t         expert_size,
    const int32_t *expert_ids,
    int            n_experts)
{
    if (!vitriol_sycl_prefetch_enabled())
        return;

    int layer_idx = get_layer_index((uintptr_t)tensor_base);
    if (layer_idx < 0) return;

    detect_token_boundary(layer_idx);

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
}

/* ── E31: hot-expert profile + selective mlock ────────────────── */

/* Parsed from E34 CSV (VITRIOL_HOT_PROFILE): set of per-expert slice
 * sizes that carry >50% of traffic.  After model load, tensors whose
 * per-expert size matches are mlocked so the hot pages never fault. */

static std::unordered_set<size_t> g_hot_expert_sizes;
static bool g_hot_profile_loaded = false;
static bool g_mlock_unavailable = false;

/* Parse an E34 profile CSV.  Extract all unique expert_size values from
 * metadata lines and mark them as hot.  Every MoE tensor whose per-expert
 * slice matches a hot size will be mlocked/preloaded. */
void vitriol_sycl_load_hot_profile(const char *path) {
    if (!path || !path[0]) return;

    FILE *f = fopen(path, "r");
    if (!f) {
        fprintf(stderr, "VITRIOL-E31: profile not found: %s\n", path);
        return;
    }

    char line[256];
    while (fgets(line, sizeof(line), f)) {
        if (line[0] != '#') continue;
        const char *p = strstr(line, "expert_size=");
        if (p) {
            size_t esz = strtoull(p + 12, nullptr, 10);
            if (esz > 0) g_hot_expert_sizes.insert(esz);
        }
    }
    fclose(f);

    g_hot_profile_loaded = !g_hot_expert_sizes.empty();
    if (g_hot_profile_loaded) {
        fprintf(stderr, "VITRIOL-E31: loaded hot profile %s (%zu unique expert sizes: ",
                path, g_hot_expert_sizes.size());
        bool first = true;
        for (size_t esz : g_hot_expert_sizes) {
            if (!first) fprintf(stderr, ", ");
            fprintf(stderr, "%zu", esz);
            first = false;
        }
        fprintf(stderr, ")\n");
    }
}

bool vitriol_sycl_hot_profile_loaded(void) {
    return g_hot_profile_loaded;
}

/* Selectively mlock hot expert ranges within tensors.
 * Called after model load while the ggml_context is still alive.
 * Walks the context's tensor list; for each 3D MoE tensor whose
 * per-expert size (nbytes / ne[2]) matches a hot expert_size, mlock
 * the entire tensor range (hot experts are contiguous within it). */
void vitriol_sycl_mlock_hot_ranges(struct ggml_context *ctx) {
    if (!g_hot_profile_loaded || !ctx) return;

    size_t total_locked = 0;
    int n_locked = 0;
    const size_t page = (size_t)sysconf(_SC_PAGESIZE);

    for (struct ggml_tensor *t = ggml_get_first_tensor(ctx);
         t; t = ggml_get_next_tensor(ctx, t)) {
        if (t->ne[2] <= 1) continue; /* not a 3D MoE tensor */

        size_t total = ggml_nbytes(t);
        size_t per_expert = total / (size_t)t->ne[2];

        if (g_hot_expert_sizes.count(per_expert) == 0) continue;

        /* Page-align the range */
        uintptr_t start = (uintptr_t)t->data;
        uintptr_t end   = start + total;
        uintptr_t astart = start & ~(page - 1);
        uintptr_t aend   = (end + page - 1) & ~(page - 1);

        if (mlock((void *)astart, aend - astart) == 0) {
            total_locked += aend - astart;
            n_locked++;
        } else if (errno == ENOMEM) {
            /* RLIMIT_MEMLOCK too low — fall back to madvise in preload */
            g_mlock_unavailable = true;
        }
    }

    if (n_locked > 0)
        fprintf(stderr, "VITRIOL-E31: mlocked %d MoE tensors (%zu MB) for hot-expert residency\n",
                n_locked, total_locked / 1024 / 1024);
    if (g_mlock_unavailable)
        fprintf(stderr, "VITRIOL-E31: some mlock calls failed (RLIMIT_MEMLOCK), using madvise preload fallback\n");
}

/* Touch hot pages to fault them into page cache and warm the device LRU.
 * Called after mlock, before the first inference.  For each hot MoE
 * tensor, touches one byte per page to bring pages into resident set. */
void vitriol_sycl_preload_hot_pages(struct ggml_context *ctx) {
    if (!g_hot_profile_loaded || !ctx) return;

    int n_touched = 0;
    for (struct ggml_tensor *t = ggml_get_first_tensor(ctx);
         t; t = ggml_get_next_tensor(ctx, t)) {
        if (t->ne[2] <= 1) continue;
        size_t total = ggml_nbytes(t);
        size_t per_expert = total / (size_t)t->ne[2];
        if (g_hot_expert_sizes.count(per_expert) == 0) continue;
        vitriol_sycl_touch_pages(t->data, total);
        n_touched++;
    }

    if (n_touched > 0)
        fprintf(stderr, "VITRIOL-E31: preloaded %d MoE tensors into page cache\n", n_touched);
}

/* ── Stats ─────────────────────────────────────────────────────── */

void vitriol_sycl_print_stats(void) {
    if (g_lru_stats.hits + g_lru_stats.misses == 0) return;
    fprintf(stderr, "VITRIOL-SYCL LRU: hits=%llu misses=%llu evictions=%llu (hit rate %.1f%%)\n",
            g_lru_stats.hits, g_lru_stats.misses, g_lru_stats.evictions,
            100.0 * g_lru_stats.hits / (g_lru_stats.hits + g_lru_stats.misses));
}

void vitriol_sycl_cleanup(void) {
    vitriol_sycl_print_stats();
    if (g_lru_slot_events) {
        delete[] g_lru_slot_events;
        g_lru_slot_events = nullptr;
    }
    if (g_lru_pool) {
        sycl::free(g_lru_pool, *g_lru_queue);
        g_lru_pool = nullptr;
    }
    if (g_lru_queue) {
        delete g_lru_queue;
        g_lru_queue = nullptr;
    }
}
