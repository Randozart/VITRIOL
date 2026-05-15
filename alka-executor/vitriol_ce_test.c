/*
 * vitriol_ce_test.cu — Standalone Copy Engine DMA test
 *
 * Verifies NV_C0B5 DMA works by:
 *   1. Initializing CE channel (scans UserD, locates GPFIFO)
 *   2. Reading GGUF header into bounce buffer via CPU
 *   3. Allocating VRAM destination buffer via cuMemAlloc
 *   4. Issuing CE DMA from bounce (GPU VA) → VRAM (GPU VA)
 *   5. Reading VRAM back to verify GGUF magic (47 47 55 46)
 *
 * Build: nvcc -o vitriol_ce_test vitriol_ce_test.cu
 * Run:   ./vitriol_ce_test llama.cpp/models/ggml-vocab-gemma-4.gguf
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/stat.h>
#include <cuda.h>
#include <cuda_runtime.h>

#include "vitriol_copy_engine.h"

/* Read from file into bounce buffer at the CPU side */
static int read_file_to_bounce(const char *path, void *buf, size_t size, size_t file_off)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0) {
        perror("open");
        return -1;
    }
    if (lseek(fd, file_off, SEEK_SET) < 0) {
        perror("lseek");
        close(fd);
        return -1;
    }
    ssize_t n = read(fd, buf, size);
    if (n < 0 || (size_t)n != size) {
        perror("read");
        close(fd);
        return -1;
    }
    close(fd);
    return 0;
}

int main(int argc, char *argv[])
{
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <gguf_file>\n", argv[0]);
        return 1;
    }

    const char *gguf_path = argv[1];
    const uint32_t test_size = 4096;

    printf("=== VITRIOL Copy Engine Test ===\n\n");

    /* Initialize CUDA driver (needed for cuMem* API) */
    CUresult cuerr = cuInit(0);
    if (cuerr != CUDA_SUCCESS) {
        fprintf(stderr, "cuInit failed: %d\n", cuerr);
        return 1;
    }
    printf("CUDA driver initialized\n");

    CUdevice device;
    cuDeviceGet(&device, 0);
    printf("Using CUDA device 0\n");

    CUcontext ctx;
    cuCtxCreate(&ctx, 0, device);
    printf("CUDA context created\n\n");

    /* Step 1: Initialize Copy Engine channel */
    printf("--- Step 1: Initialize CE Channel ---\n");
    vitriol_ce_channel_t ce;
    if (vitriol_ce_init(&ce) != 0) {
        fprintf(stderr, "CE init failed\n");
        cuCtxDestroy(ctx);
        return 1;
    }
    printf("CE channel initialized\n\n");

    /* Step 2: Read GGUF header into bounce buffer */
    printf("--- Step 2: Read GGUF into bounce buffer ---\n");
    memset(ce.bounce_cpu, 0, test_size);
    if (read_file_to_bounce(gguf_path, ce.bounce_cpu, test_size, 0) != 0) {
        fprintf(stderr, "Failed to read GGUF file\n");
        vitriol_ce_destroy(&ce);
        cuCtxDestroy(ctx);
        return 1;
    }

    /* Verify bounce buffer has GGUF magic */
    unsigned char *bounce_bytes = (unsigned char *)ce.bounce_cpu;
    printf("Bounce buffer first 4 bytes: %02x %02x %02x %02x\n",
           bounce_bytes[0], bounce_bytes[1],
           bounce_bytes[2], bounce_bytes[3]);

    if (bounce_bytes[0] != 'G' || bounce_bytes[1] != 'G' ||
        bounce_bytes[2] != 'U' || bounce_bytes[3] != 'F') {
        fprintf(stderr, "Bounce buffer doesn't contain GGUF data!\n");
        vitriol_ce_destroy(&ce);
        cuCtxDestroy(ctx);
        return 1;
    }
    printf("GGUF magic verified in bounce buffer: 47 47 55 46\n\n");

    /* Step 3: Allocate VRAM destination and get its GPU VA */
    printf("--- Step 3: Allocate VRAM destination ---\n");
    CUdeviceptr vram_dst;
    cuerr = cuMemAlloc(&vram_dst, test_size);
    if (cuerr != CUDA_SUCCESS) {
        fprintf(stderr, "cuMemAlloc failed: %d\n", cuerr);
        vitriol_ce_destroy(&ce);
        cuCtxDestroy(ctx);
        return 1;
    }

    /* Zero the VRAM so we can tell if DMA actually wrote */
    cuMemsetD8(vram_dst, 0xAB, test_size);
    printf("VRAM destination: 0x%llx\n", (unsigned long long)vram_dst);
    printf("  (zeroed with 0xAB for verification)\n\n");

    /* Step 4: Issue CE DMA from bounce → VRAM */
    printf("--- Step 4: CE DMA transfer ---\n");
    printf("  Source:      0x%llx (bounce GPU VA)\n",
           (unsigned long long)ce.bounce_gpu);
    printf("  Dest:        0x%llx (VRAM GPU VA)\n",
           (unsigned long long)vram_dst);
    printf("  Size:        %u bytes\n", test_size);
    printf("  Metapage:    0x%llx (fence GPU VA)\n",
           (unsigned long long)ce.fence_gpu);

    int ret = vitriol_ce_dma(&ce, ce.bounce_gpu, vram_dst, test_size);
    if (ret != 0) {
        fprintf(stderr, "CE DMA failed: %d\n", ret);
        cuMemFree(vram_dst);
        vitriol_ce_destroy(&ce);
        cuCtxDestroy(ctx);
        return 1;
    }
    printf("CE DMA completed successfully\n\n");

    /* Step 5: Read VRAM back to verify */
    printf("--- Step 5: Verify VRAM contents ---\n");
    unsigned char verify[64];
    cuMemcpyDtoH(verify, vram_dst, 64);

    printf("VRAM first 64 bytes:\n");
    for (int i = 0; i < 64; i += 16) {
        printf("  [%02x] ", i);
        for (int j = 0; j < 16 && i + j < 64; j++)
            printf("%02x ", verify[i + j]);
        printf("\n");
    }

    /* Check vs expected */
    int match = 1;
    for (int i = 0; i < 64 && i < test_size; i++) {
        if (verify[i] != bounce_bytes[i]) {
            if (match) {
                printf("\nMISMATCH at byte %d: VRAM=0x%02x Expected=0x%02x\n",
                       i, verify[i], bounce_bytes[i]);
            }
            match = 0;
        }
    }

    if (match) {
        printf("\n=== PASS: DMA data matches GGUF source! ===\n");
        printf("First 4 bytes: %02x %02x %02x %02x (GGUF)\n",
               verify[0], verify[1], verify[2], verify[3]);
    } else {
        fprintf(stderr, "\n=== FAIL: DMA data mismatch ===\n");
    }

    /* Cleanup */
    cuMemFree(vram_dst);
    vitriol_ce_destroy(&ce);
    cuCtxDestroy(ctx);

    printf("\nTest complete.\n");
    return match ? 0 : 1;
}
