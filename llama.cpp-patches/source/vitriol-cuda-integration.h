// VITRIOL Integration for ggml-cuda.cu
// RAM Shot: expert weights in page-locked host RAM, GPU reads over PCIe DMA.
//
// Architecture:
//   - Custom VITRIOL buffer type allocates hugepage-backed system RAM
//   - mmap + madvise(MADV_HUGEPAGE) + mlock + cudaHostRegister
//   - Set_tensor copies expert data into the buffer normally
//   - MUL_MAT_ID on CUDA reads weights directly from host memory
//   - No VRAM used for weight storage
//
// Future: CE DMA + VRAM pool for LRU hot-expert cache (optional perf optimization)

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

/* Initialize VITRIOL state (called from ggml_backend_cuda_init) */
void vitriol_cuda_init(void);

/* Check if stream mode (expert weights in page-locked host RAM) */
static inline bool vitriol_is_stream_enabled(void) {
    return g_vitriol_config.mode == VITRIOL_MODE_STREAM;
}

/* Print VITRIOL stats */
void vitriol_cuda_print_stats(void);

/* Get the VITRIOL expert buffer type for device 0.
 * Uses default visibility so it's findable via dlsym from the model loader.
 * Returns a buffer type that allocates page-locked host memory.
 * GPU accesses expert weights directly over PCIe DMA during MUL_MAT_ID. */
__attribute__((visibility("default")))
struct ggml_backend_buffer_type * vitriol_get_expert_buffer_type(void);

#ifdef __cplusplus
}
#endif

#endif // VITRIOL_CUDA_INTEGRATION_H
