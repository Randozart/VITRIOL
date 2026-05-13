// VITRIOL Integration for ggml-cuda.cu
// This file adds async double-buffer prefetch support

#ifndef VITRIOL_CUDA_INTEGRATION_H
#define VITRIOL_CUDA_INTEGRATION_H

#include "ggml-cuda.h"
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

// Forward declarations
struct ggml_tensor;

// VITRIOL Configuration (from vitriol-config.h)
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

// Global config (defined elsewhere)
extern vitriol_config_t g_vitriol_config;

// Layer tracking for async prefetch
typedef struct {
    int current_layer;      // Currently computing layer
    int prefetch_layer;    // Layer being prefetched
    bool prefetch_pending; // Is prefetch in progress?
    void* buffer_a;        // Double-buffer A
    void* buffer_b;        // Double-buffer B
    bool buffer_active;   // Which buffer is active (A=false, B=true)
} vitriol_layer_state_t;

// Initialize VITRIOL state
void vitriol_cuda_init(void);

// Check if VITRIOL is enabled and in async mode
static inline bool vitriol_is_async_enabled(void) {
    return g_vitriol_config.mode == VITRIOL_MODE_ASYNC && 
           g_vitriol_config.async_prefetch;
}

// Check if VITRIOL is in stream mode (on-demand loading)
static inline bool vitriol_is_stream_enabled(void) {
    return g_vitriol_config.mode == VITRIOL_MODE_STREAM;
}

// Get layer info for async tracking
void vitriol_cuda_set_current_layer(int layer_id);
void vitriol_cuda_trigger_prefetch(int next_layer_id);

// Hook for tensor operations - returns true if VITRIOL handled it
bool vitriol_cuda_set_tensor_hook(
    struct ggml_tensor* tensor,
    const void* data,
    size_t size,
    uint64_t tensor_file_offset
);

// Print VITRIOL stats
void vitriol_cuda_print_stats(void);

// Helper: Check if tensor should be prefetched based on layer
static inline bool vitriol_should_prefetch(int layer_id) {
    if (!vitriol_is_async_enabled()) return false;
    return layer_id >= g_vitriol_config.static_layers;
}

// Helper: Check if tensor is in "static" VRAM zone
static inline bool vitriol_is_static_layer(int layer_id) {
    return layer_id < g_vitriol_config.static_layers;
}

#ifdef __cplusplus
}
#endif

#endif // VITRIOL_CUDA_INTEGRATION_H