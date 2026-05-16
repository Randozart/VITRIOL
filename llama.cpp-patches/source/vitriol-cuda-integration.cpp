#include "vitriol-cuda-integration.h"
#include "vitriol_copy_engine.h"
#include "vitriol-buffer.h"
#include "ggml-cuda.h"
#include "ggml-backend.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <cuda.h>
#include <cuda_runtime.h>

vitriol_config_t g_vitriol_config = {
    .mode = VITRIOL_MODE_DISABLED,
    .async_prefetch = false,
    .prefetch_ahead = 1,
    .static_layers = 15,
    .window_size_mb = 256,
    .use_double_buffer = true,
    .buffer_count = 2,
    .verbose = false,
    .benchmark = false
};

static bool g_ce_initialized = false;
static vitriol_ce_channel_t g_ce;

/* ── Initialization ────────────────────────────────────────────── */

void vitriol_cuda_init(void) {
    const char* mode_env = getenv("VITRIOL_MODE");
    if (mode_env) {
        if (strcmp(mode_env, "disabled") == 0 || strcmp(mode_env, "off") == 0)
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
            printf("VITRIOL: Stream mode — expert weights in page-locked host RAM\n");

        if (vitriol_ce_init(&g_ce) == 0) {
            g_ce_initialized = true;
            if (g_vitriol_config.verbose)
                printf("VITRIOL: Copy Engine DMA initialized (available for future LRU cache)\n");
        } else {
            printf("VITRIOL: CE init failed (non-fatal, GPU reads via PCIe DMA)\n");
        }
    } else {
        if (g_vitriol_config.verbose)
            printf("VITRIOL: Stream mode not set, VITRIOL inactive\n");
    }
}

ggml_backend_buffer_type_t vitriol_get_expert_buffer_type(void) {
    if (g_vitriol_config.mode != VITRIOL_MODE_STREAM)
        return NULL;
    return vitriol_get_buffer_type(0);
}

void vitriol_cuda_print_stats(void) {
    printf("=== VITRIOL Statistics ===\n");
    printf("Mode: %d\n", g_vitriol_config.mode);
    printf("CE DMA: %s\n", g_ce_initialized ? "available" : "unavailable");
    printf("Strategy: RAM Shot (expert weights in page-locked host memory)\n");
    printf("=========================\n");
}
