// VITRIOL Integration for ggml-cuda.cu
// RAM Shot: expert weights in page-locked host RAM, GPU reads over PCIe DMA.
// LRU Cache: hot experts in VRAM for near-VRAM matmul speed on cache hit.
//
// Architecture:
//   - Custom VITRIOL buffer type allocates hugepage-backed system RAM
//   - mmap + madvise(MADV_HUGEPAGE) + mlock + cudaHostRegister
//   - LRU VRAM cache (~512 MB) for hot experts with composite key (tensor_base, expert_idx)
//   - Async DMA on dedicated stream, sync'd via cuStreamWaitEvent before matmul
//   - Falls through to host RAM read on cache miss

#ifndef VITRIOL_CUDA_INTEGRATION_H
#define VITRIOL_CUDA_INTEGRATION_H

#include "ggml-cuda.h"
#include "ggml-backend.h"
#include <stdint.h>
#include <stdbool.h>
#include <cuda.h>

#ifdef __cplusplus
extern "C" {
#endif

struct ggml_tensor;

typedef enum {
    VITRIOL_MODE_DISABLED = 0,
    VITRIOL_MODE_SYNC = 1,
    VITRIOL_MODE_ASYNC = 2,
    VITRIOL_MODE_STREAM = 3
} vitriol_mode_t;

typedef struct {
    vitriol_mode_t mode;
    bool async_prefetch;
    int prefetch_ahead;
    int static_layers;
    int window_size_mb;
    bool use_double_buffer;
    int buffer_count;
    bool verbose;
    bool benchmark;
    bool disk_offload;
    bool store_backed;        // 0=mmap GGUF (default), 1=read experts from store
    bool output_cache;
    int  pin_first_n_layers;  // 0=off, N=pin first N model layers' expert tensors in VRAM
    int  pin_tensors_per_layer;  // auto-detected: number of tensor ops per model layer (default 2 for fused gate+up + down)
    bool pin_active;          // true after first pin allocation
    int  prune_experts;       // 0=off, N=drop bottom N of 8 active experts before compute
    int  max_locked_mb;       // 0=all (eager), N=max MiB of page-locked host RAM (lazy/chunked)
} vitriol_config_t;

extern vitriol_config_t g_vitriol_config;

/* Store backing path (host-only; null when VITRIOL_STORE unset). */
const char * vitriol_store_path(void);

bool vitriol_store_register_buffer(
        ggml_backend_buffer_t buffer,
        void * store,
        const void * base,
        size_t size);
void vitriol_store_release_buffer(
        ggml_backend_buffer_t buffer,
        const void * base,
        size_t size);
bool vitriol_store_bind_tensor(
        ggml_backend_buffer_t buffer,
        const void * tensor_base,
        const char * name,
        size_t tensor_bytes,
        size_t row_bytes);

void vitriol_cuda_init(void);

/* ── Lazy (Chunked) Page Locking ──────────────────────────────────
 * When max_locked_mb > 0, expert weight buffers are allocated as
 * pageable mmap (no mlock, no cudaHostRegister). Individual expert
 * slices are page-locked on demand via vitriol_ensure_expert_locked(),
 * limited to max_locked_mb total. Stale slices are evicted via LRU.
 * ──────────────────────────────────────────────────────────────────*/

/* Returns true if lazy/chunked page locking is active. */
static inline bool vitriol_lazy_lock_active(void) {
    return g_vitriol_config.max_locked_mb > 0;
}

/* Ensure a single expert slice (nb02 bytes at tensor_base + i02*nb02)
 * is page-locked and DMA-registered. May evict a stale locked slice
 * to stay within max_locked_mb budget. Idempotent — safe to call
 * multiple times for the same slice. */
void vitriol_ensure_expert_locked(
    const void    *tensor_base,
    int            expert_idx,
    size_t         expert_size);

static inline bool vitriol_is_stream_enabled(void) {
    return g_vitriol_config.mode == VITRIOL_MODE_STREAM;
}

/* LRU Cache: ensure expert data is in VRAM.
 * Called from ggml_cuda_mul_mat_id before matmul.
 * Returns VRAM pointer (cached), or 0 to fall through to host read.
 * tensor_base: the base data pointer of the full expert tensor (dst->src[0]->data)
 * expert_data: pointer to the specific expert slice within tensor_base
 * compute_stream: the CUDA stream that will run the matmul */
CUdeviceptr vitriol_lru_ensure(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size,
    CUstream       compute_stream);

/* Fire-and-forget prefetch for fast-path (MMQ/MMF/MMVQ with ids).
 * Queues async DMA on the LRU stream; compute_stream waits via event. */
void vitriol_lru_prefetch(
    const void    *tensor_base,
    int            expert_idx,
    const void    *expert_data,
    size_t         expert_size,
    CUstream       compute_stream);

/* Mark an LRU VRAM slot as no longer in use by compute.
 * Called from ggml-cuda.cu after each matmul so the LRU stream knows
 * it is safe to overwrite this slot on eviction. */
void vitriol_lru_mark_compute_done(CUdeviceptr vram_ptr, CUstream stream);

/* ── Predictive Prefetching ──────────────────────────────────────────
 * Combined cross-layer + temporal prediction (Fate-style heuristic).
 *
 * Cross-layer: experts that fired in layer N are likely to fire again in
 *              layer N+1 of the same token (~40-60% overlap).
 * Temporal:    experts that fired at layer N of the previous token are
 *              likely to fire at layer N of the current token (~50% overlap).
 *
 * Both are free — no training, no profiling, no offline data.
 * Union of both predictions is prefetched into the LRU VRAM pool on a
 * dedicated CUDA stream, async with GPU compute.
 *
 * Controlled by env VITRIOL_PREDICTIVE_PREFETCH=1.
 * ────────────────────────────────────────────────────────────────────*/

/* Called at the START of ggml_cuda_mul_mat_id (before ids copy).
 * Fires async prefetch for experts predicted from the previous call. */
void vitriol_predictor_prefetch(
    const void    *tensor_base,
    size_t         expert_size,
    CUstream       compute_stream);

/* Called at the END of ggml_cuda_mul_mat_id (after expert selection).
 * Stores the actual expert IDs used this call for next call's prediction. */
void vitriol_predictor_update(
    const void    *tensor_base,
    size_t         expert_size,
    const int32_t *expert_ids,
    int            n_experts);

/* Returns true if predictive prefetching is enabled. */
static inline bool vitriol_predictive_enabled(void) {
    return g_vitriol_config.async_prefetch;
}

/* ── Expert Pinning ──────────────────────────────────────────────────
 * Pre-load full expert weight tensors of the first N layers into VRAM
 * at first use, so MMVQ/MMQ/MMF fast-path kernels read from VRAM
 * instead of PCIe-hosted page-locked RAM.
 *
 * Controlled by env VITRIOL_PIN_FIRST_N_LAYERS (default 0 = off)
 * or config vitriol.pin_first_n_layers.
 *
 * When active, output cache is auto-disabled (they target different
 * decode paths: pinning helps the fast path, output cache helps the
 * per-expert loop).
 * ──────────────────────────────────────────────────────────────────*/

/* Ensure the tensor for a given layer is pinned in VRAM.
 * Called lazily on first encounter during prefill/decode.
 * Returns VRAM pointer, or 0 on failure/skip. */
CUdeviceptr vitriol_pin_ensure(
    const void    *tensor_base,
    size_t         tensor_nb02,
    int64_t        n_experts,
    CUstream       stream);

/* Look up an already-pinned tensor by its base address.
 * Returns VRAM pointer or 0. */
CUdeviceptr vitriol_pin_lookup(const void *tensor_base);

static inline int vitriol_prune_experts(void) {
    return g_vitriol_config.prune_experts;
}

/* Returns true if at least one tensor has been pinned. */
static inline bool vitriol_pin_active(void) {
    return g_vitriol_config.pin_active;
}

/* Returns true if expert pinning is enabled — either the global
 * VITRIOL_PIN_FIRST_N_LAYERS or any per-device override
 * VITRIOL_PIN_FIRST_N_LAYERS_GPU<d> is > 0. */
bool vitriol_pin_enabled(void);

void vitriol_cuda_print_stats(void);

__attribute__((visibility("default")))
struct ggml_backend_buffer_type * vitriol_get_expert_buffer_type(void);

/* Device-aware variant: returns the VITRIOL expert buffer type bound to the
 * given CUDA device (for multi-GPU layer splits). Called by the model loader. */
__attribute__((visibility("default")))
struct ggml_backend_buffer_type * vitriol_get_expert_buffer_type_dev(int device);

/* ── Expert Output Cache (approximate) ──────────────────────────────
 * During single-token generation, caches each expert's FFN output
 * vector (n_embd floats) keyed by (tensor_base, expert_id).
 *
 * On the next token, if the same (layer, expert) is selected again,
 * the cached output is reused instead of recomputing. This is
 * APPROXIMATE — cached outputs are from a different token's input.
 * However, consecutive tokens are often similar, so quality impact
 * may be acceptable in practice.
 *
 * Controlled by env VITRIOL_OUTPUT_CACHE=1.
 * ──────────────────────────────────────────────────────────────────*/

/* Initialize the output cache. Allocates one ring buffer per layer. */
void vitriol_output_cache_init(
    int    n_layers,
    size_t n_embd);

/* Look up cached output for (tensor_base, expert_id).
 * Returns pointer to cached float[n_embd] data, or NULL on miss. */
const float * vitriol_output_cache_lookup(
    const void *tensor_base,
    int         expert_id);

/* Store output for (tensor_base, expert_id) into the cache.
 * output_data: float[n_embd] vector on the device (dst_slice.data). */
void vitriol_output_cache_store(
    const void *tensor_base,
    int         expert_id,
    const float *output_data,
    size_t       n_embd,
    CUstream     stream);

/* Advance to next token: marks all cached entries as stale
 * (first call stores per-layer expert masks; second call discards). */
void vitriol_output_cache_advance_token(void);

/* Returns true if output caching is enabled and initialized. */
static inline bool vitriol_output_cache_active(void) {
    return g_vitriol_config.output_cache;
}

/* Print output cache stats. */
void vitriol_output_cache_print_stats(void);

#ifdef __cplusplus
}
#endif

bool vitriol_store_source_device(void);

bool vitriol_skip_host_read(struct ggml_tensor * cur);

#endif // VITRIOL_CUDA_INTEGRATION_H
