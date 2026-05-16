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
} vitriol_config_t;

extern vitriol_config_t g_vitriol_config;

void vitriol_cuda_init(void);

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

void vitriol_cuda_print_stats(void);

__attribute__((visibility("default")))
struct ggml_backend_buffer_type * vitriol_get_expert_buffer_type(void);

#ifdef __cplusplus
}
#endif

#endif // VITRIOL_CUDA_INTEGRATION_H
