#pragma once

#include "ggml.h"
#include "ggml-backend.h"

#ifdef  __cplusplus
extern "C" {
#endif

#ifdef GGML_USE_HIP
#define GGML_CUDA_NAME "ROCm"
#define GGML_CUBLAS_NAME "hipBLAS"
#elif defined(GGML_USE_MUSA)
#define GGML_CUDA_NAME "MUSA"
#define GGML_CUBLAS_NAME "muBLAS"
#else
#define GGML_CUDA_NAME "CUDA"
#define GGML_CUBLAS_NAME "cuBLAS"
#endif
#define GGML_CUDA_MAX_DEVICES       16

// backend API
GGML_BACKEND_API ggml_backend_t ggml_backend_cuda_init(int device);

GGML_BACKEND_API bool ggml_backend_is_cuda(ggml_backend_t backend);

// device buffer
GGML_BACKEND_API ggml_backend_buffer_type_t ggml_backend_cuda_buffer_type(int device);

// conduct allreduce operation between devices
GGML_BACKEND_API bool ggml_backend_cuda_allreduce_tensor(ggml_backend_t * backends, struct ggml_tensor ** tensors, size_t n_backends);

// pinned host buffer for use with the CPU backend for faster copies between CPU and GPU
GGML_BACKEND_API ggml_backend_buffer_type_t ggml_backend_cuda_host_buffer_type(void);

GGML_BACKEND_API int  ggml_backend_cuda_get_device_count(void);
GGML_BACKEND_API void ggml_backend_cuda_get_device_description(int device, char * description, size_t description_size);
GGML_BACKEND_API void ggml_backend_cuda_get_device_memory(int device, size_t * free, size_t * total);

// ── VITRIOL perf diagnostics (env-gated by GGML_CUDA_GDN_PROFILE) ────────────
// Rolling counters accumulated across all CUDA devices/contexts during graph
// compute. read by llama-context to emit the [PERF] decode-breakdown line.
typedef struct ggml_cuda_perf_snapshot {
    uint64_t n_capture;              // CUDA graph captures this decode
    uint64_t n_replay;               // CUDA graph replays this decode
    uint64_t op_n  [GGML_OP_COUNT];  // per-op-class node count (capture/eval path only)
} ggml_cuda_perf_snapshot;

GGML_BACKEND_API void ggml_cuda_perf_reset(void);
GGML_BACKEND_API struct ggml_cuda_perf_snapshot ggml_cuda_perf_get(void);

// VITRIOL stack-pull S1: cumulative CUDA-graph capture accounting since boot
// (captures / no-recapture launches / exec updates / idle-sweep evictions).
GGML_BACKEND_API void ggml_cuda_graph_stats(uint64_t out[4]);

GGML_BACKEND_API bool ggml_backend_cuda_register_host_buffer(void * buffer, size_t size);
GGML_BACKEND_API void ggml_backend_cuda_unregister_host_buffer(void * buffer);

GGML_BACKEND_API ggml_backend_reg_t ggml_backend_cuda_reg(void);

#ifdef  __cplusplus
}
#endif
