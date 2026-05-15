/*
 * vitriol_copy_engine.c — NV_C0B5 Copy Engine DMA via CUDA API
 *
 * Uses CUDA's cuMemcpyDtoDAsync which routes through the GPU's
 * internal Copy Engine. Provides the bounce-buffer abstraction
 * for NVMe→VRAM streaming without CPU-mediated copies.
 *
 * Phase 1: CUDA API path (works now, ~12 GB/s CE bandwidth)
 * Phase 2: Direct doorbell (bypass UMD overhead, planned)
 *
 * Copyright 2026
 * Licensed under Apache 2.0 with Runtime Exception.
 */

#include "vitriol_copy_engine.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>

#define CE_DEBUG(fmt, ...) \
    fprintf(stderr, "CE: " fmt "\n", ##__VA_ARGS__)


int vitriol_ce_init(vitriol_ce_channel_t *chan)
{
    memset(chan, 0, sizeof(*chan));
    CE_DEBUG("Initializing Copy Engine channel (CUDA API path)");

    /* 1. Create dedicated DMA CUDA stream */
    CUresult err = cuStreamCreate(&chan->stream, CU_STREAM_NON_BLOCKING);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuStreamCreate failed: %d", err);
        return -1;
    }
    CE_DEBUG("Created DMA stream");

    /* 2. Allocate bounce buffer (pinned host memory mapped to GPU) */
    chan->bounce_size = CE_BOUNCE_SIZE;
    err = cuMemHostAlloc(&chan->bounce_cpu, chan->bounce_size,
                         CU_MEMHOSTALLOC_DEVICEMAP);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuMemHostAlloc failed: %d", err);
        cuStreamDestroy(chan->stream);
        return -1;
    }

    err = cuMemHostGetDevicePointer(&chan->bounce_gpu, chan->bounce_cpu, 0);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuMemHostGetDevicePointer failed: %d", err);
        cuMemFreeHost(chan->bounce_cpu);
        cuStreamDestroy(chan->stream);
        return -1;
    }
    CE_DEBUG("Bounce buffer: cpu=%p gpu=0x%llx size=%zu",
             chan->bounce_cpu,
             (unsigned long long)chan->bounce_gpu,
             chan->bounce_size);

    /* 3. Allocate metapage fence */
    err = cuMemHostAlloc((void **)&chan->fence_cpu, 64,
                         CU_MEMHOSTALLOC_DEVICEMAP);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuMemHostAlloc(fence) failed: %d", err);
        cuMemFreeHost(chan->bounce_cpu);
        cuStreamDestroy(chan->stream);
        return -1;
    }
    *(volatile uint32_t *)chan->fence_cpu = 0;

    err = cuMemHostGetDevicePointer(&chan->fence_gpu, (void *)chan->fence_cpu, 0);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuMemHostGetDevicePointer(fence) failed: %d", err);
        cuMemFreeHost((void *)chan->fence_cpu);
        cuMemFreeHost(chan->bounce_cpu);
        cuStreamDestroy(chan->stream);
        return -1;
    }
    CE_DEBUG("Metapage fence: cpu=%p gpu=0x%llx",
             chan->fence_cpu, (unsigned long long)chan->fence_gpu);

    CE_DEBUG("CE channel initialized");
    return 0;
}

int vitriol_ce_dma(vitriol_ce_channel_t *chan,
                   CUdeviceptr src_gpu_va,
                   CUdeviceptr dst_gpu_va,
                   uint32_t size)
{
    if (!chan->stream)
        return -1;

    /* Use cuMemcpyDtoDAsync — routes through GPU Copy Engine
     * without CPU copy. The CE reads from bounce (GPU VA of
     * pinned host memory) and writes to VRAM. */

    CUresult err = cuMemcpyDtoDAsync(dst_gpu_va, src_gpu_va,
                                     (size_t)size, chan->stream);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuMemcpyDtoDAsync failed: %d", err);
        return -1;
    }

    err = cuStreamSynchronize(chan->stream);
    if (err != CUDA_SUCCESS) {
        CE_DEBUG("cuStreamSynchronize failed: %d", err);
        return -1;
    }

    return 0;
}

void vitriol_ce_destroy(vitriol_ce_channel_t *chan)
{
    if (chan->fence_cpu)
        cuMemFreeHost((void *)chan->fence_cpu);
    if (chan->bounce_cpu)
        cuMemFreeHost(chan->bounce_cpu);
    if (chan->stream)
        cuStreamDestroy(chan->stream);
    memset(chan, 0, sizeof(*chan));
    CE_DEBUG("CE channel destroyed");
}
