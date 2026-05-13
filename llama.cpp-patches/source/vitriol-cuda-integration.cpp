#include "vitriol-cuda-integration.h"
#include "ggml-cuda.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Dummy global config - will be replaced when linked
vitriol_config_t g_vitriol_config = {
    .mode = (vitriol_mode_t)0,
    .async_prefetch = false,
    .prefetch_ahead = 1,
    .static_layers = 15,
    .window_size_mb = 256,
    .use_double_buffer = true,
    .buffer_count = 2,
    .verbose = false,
    .benchmark = false
};

// Layer state for async prefetching
static vitriol_layer_state_t vitriol_state;

// Statistics
static struct {
    size_t sync_copies;
    size_t async_prefetch;
    size_t stream_loads;
    double total_time_ms;
} vitriol_stats;

void vitriol_cuda_init(void) {
    memset(&vitriol_state, 0, sizeof(vitriol_state));
    vitriol_state.current_layer = -1;
    vitriol_state.prefetch_layer = -1;

    g_vitriol_config.mode = (vitriol_mode_t)0;

    // Read mode from environment
    const char* mode_env = getenv("VITRIOL_MODE");
    if (mode_env) {
        if (strcmp(mode_env, "disabled") == 0 || strcmp(mode_env, "off") == 0) {
            g_vitriol_config.mode = (vitriol_mode_t)0;
        } else if (strcmp(mode_env, "sync") == 0) {
            g_vitriol_config.mode = (vitriol_mode_t)1;
        } else if (strcmp(mode_env, "async") == 0) {
            g_vitriol_config.mode = (vitriol_mode_t)2;
            g_vitriol_config.async_prefetch = true;
        } else if (strcmp(mode_env, "stream") == 0) {
            g_vitriol_config.mode = (vitriol_mode_t)3;
        }
    }

    // Check for verbose flag
    const char* verbose_env = getenv("VITRIOL_VERBOSE");
    if (verbose_env && strcmp(verbose_env, "1") == 0) {
        g_vitriol_config.verbose = true;
    }

    if (g_vitriol_config.verbose) {
        const char* mode_names[] = {"disabled", "sync", "async", "stream"};
        printf("VITRIOL: Initialized (mode=%s)\n",
               mode_names[g_vitriol_config.mode & 3]);
    }

    // Initialize double-buffer if needed
    if (g_vitriol_config.use_double_buffer && g_vitriol_config.mode == (vitriol_mode_t)2) {
        if (g_vitriol_config.verbose) {
            printf("VITRIOL: Double-buffer mode enabled\n");
        }
    }

    memset(&vitriol_stats, 0, sizeof(vitriol_stats));
}

void vitriol_cuda_set_current_layer(int layer_id) {
    vitriol_state.current_layer = layer_id;
    
    if (g_vitriol_config.verbose && vitriol_is_async_enabled()) {
        printf("VITRIOL: Computing layer %d (static=%s)\n", 
               layer_id, 
               vitriol_is_static_layer(layer_id) ? "yes" : "no");
    }
    
    // Trigger prefetch for next layer if in async mode
    if (vitriol_is_async_enabled() && !vitriol_is_static_layer(layer_id)) {
        int next_layer = layer_id + g_vitriol_config.prefetch_ahead;
        vitriol_cuda_trigger_prefetch(next_layer);
    }
}

void vitriol_cuda_trigger_prefetch(int next_layer) {
    if (next_layer <= vitriol_state.current_layer) {
        return; // Already computed
    }
    
    vitriol_state.prefetch_layer = next_layer;
    vitriol_state.prefetch_pending = true;
    
    vitriol_stats.async_prefetch++;
    
    if (g_vitriol_config.verbose) {
        printf("VITRIOL: Triggered prefetch for layer %d\n", next_layer);
    }
}

bool vitriol_cuda_set_tensor_hook(
    struct ggml_tensor* tensor,
    const void* data,
    size_t size,
    uint64_t tensor_file_offset
) {
    (void)tensor_file_offset;
    (void)data;
    (void)size;

    // If VITRIOL is disabled, return false to use standard cudaMemcpy
    if (g_vitriol_config.mode == VITRIOL_MODE_DISABLED) {
        return false;
    }
    
    // Extract layer info from tensor name
    // Format: "model.layers.X.<component>.<subcomponent>"
    // e.g., "model.layers.15.mlp.gate_proj"
    const char* name = tensor->name;
    int layer_id = -1;
    
    // Simple parsing - look for "layers." followed by number
    const char* layers_prefix = strstr(name, "layers.");
    if (layers_prefix) {
        char* end;
        layer_id = strtol(layers_prefix + 7, &end, 10);
    }
    
    if (layer_id < 0) {
        // Not a layer tensor, use standard path
        return false;
    }
    
    // Update current layer tracking
    if (layer_id != vitriol_state.current_layer) {
        vitriol_cuda_set_current_layer(layer_id);
    }
    
    // Handle based on mode
    switch (g_vitriol_config.mode) {
        case VITRIOL_MODE_SYNC:
            // Standard sync - load all to VRAM, no optimization
            vitriol_stats.sync_copies++;
            return false;
            
        case VITRIOL_MODE_ASYNC:
            // Async prefetch mode - try to overlap with computation
            vitriol_stats.async_prefetch++;
            // For now, fall through to standard (placeholder for true async)
            return false;
            
         case VITRIOL_MODE_STREAM:
            // Stream mode - on-demand loading from SSD
            if (!vitriol_is_static_layer(layer_id)) {
                vitriol_stats.stream_loads++;
                // TODO: Implement actual SSD->GPU DMA
                // For now, fall through to standard
                printf("VITRIOL: Stream loading tensor %s (layer %d) from SSD\n",
                       name, layer_id);
            }
            return false;
            
        default:
            return false;
    }
}

void vitriol_cuda_print_stats(void) {
    printf("=== VITRIOL Statistics ===\n");
    printf("Mode: %d\n", g_vitriol_config.mode);
    printf("Sync Copies: %zu\n", vitriol_stats.sync_copies);
    printf("Async Prefetch: %zu\n", vitriol_stats.async_prefetch);
    printf("Stream Loads: %zu\n", vitriol_stats.stream_loads);
    printf("=========================\n");
}