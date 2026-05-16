/*
 * test_p2p_dma.cu — Verify VITRIOL cooperative P2P DMA to VRAM
 *
 * Usage:
 *   ./test_p2p_dma <model.gguf> [device_id]
 *
 * 1. cudaMalloc a 4KB buffer on specified GPU (default: device 1 = GTX 960)
 * 2. Initialize CUDA driver context, get P2P tokens via cuPointerGetAttribute
 * 3. Print GPU VA and tokens for executor --p2p-token and --va-space-token
 * 4. Wait for user to run executor
 * 5. Read back and compare with GGUF source bytes
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <cuda_runtime.h>
#include <cuda.h>

#define TEST_SIZE 4096

#define CUDA_RT_CHECK(x) do { \
    cudaError_t __e = (x); \
    if (__e != cudaSuccess) { \
        fprintf(stderr, "CUDA error at %s:%d: %s\n", \
                __FILE__, __LINE__, cudaGetErrorString(__e)); \
        return 1; \
    } \
} while(0)

#define CUDA_DRV_CHECK(x) do { \
    CUresult __e = (x); \
    if (__e != CUDA_SUCCESS) { \
        const char *__estr; \
        cuGetErrorString(__e, &__estr); \
        fprintf(stderr, "CUDA driver error at %s:%d: %s (code=%d)\n", \
                __FILE__, __LINE__, __estr, (int)__e); \
        return 1; \
    } \
} while(0)

static const char *GGUF_MAGIC = "GGUF";

/* Initialize CUDA driver context for the given device */
static int init_drv_context(int device_id)
{
    CUdevice cu_dev;
    CUcontext cu_ctx;

    CUresult err = cuDeviceGet(&cu_dev, device_id);
    if (err != CUDA_SUCCESS) {
        const char *estr;
        cuGetErrorString(err, &estr);
        fprintf(stderr, "cuDeviceGet(%d) failed: %s\n", device_id, estr);
        return -1;
    }

    /* Retain primary context — this ensures the driver API
     * has an active context that matches the runtime API's context */
    err = cuDevicePrimaryCtxRetain(&cu_ctx, cu_dev);
    if (err != CUDA_SUCCESS) {
        const char *estr;
        cuGetErrorString(err, &estr);
        fprintf(stderr, "cuDevicePrimaryCtxRetain(%d) failed: %s\n", device_id, estr);
        return -1;
    }

    /* Set as current context */
    err = cuCtxSetCurrent(cu_ctx);
    if (err != CUDA_SUCCESS) {
        const char *estr;
        cuGetErrorString(err, &estr);
        fprintf(stderr, "cuCtxSetCurrent failed: %s\n", estr);
        return -1;
    }

    printf("  Driver context initialized for device %d\n", device_id);
    return 0;
}

int main(int argc, char *argv[])
{
    const char *gguf_path = NULL;
    if (argc > 1)
        gguf_path = argv[1];

    int device_id = 1; /* Default: GTX 960 */
    if (argc > 2)
        device_id = atoi(argv[2]);

    /* ── Initialize CUDA driver API (needed for cuPointerGetAttribute) ── */
    CUDA_DRV_CHECK(cuInit(0));

    /* ── Enumerate GPUs ── */
    int device_count = 0;
    CUDA_RT_CHECK(cudaGetDeviceCount(&device_count));
    printf("Found %d CUDA device(s)\n", device_count);
    for (int i = 0; i < device_count; i++) {
        cudaDeviceProp prop;
        CUDA_RT_CHECK(cudaGetDeviceProperties(&prop, i));
        printf("  [%d] %s (PCI %04x:%02x:%02x, VRAM %luMB, CC %d.%d)\n",
               i, prop.name, prop.pciDomainID, prop.pciBusID,
               prop.pciDeviceID,
               prop.totalGlobalMem / (1024*1024),
               prop.major, prop.minor);
    }

    /* ── Set target device ── */
    if (device_id >= device_count) {
        fprintf(stderr, "ERROR: Device %d not found (max: %d)\n", device_id, device_count - 1);
        return 1;
    }

    CUDA_RT_CHECK(cudaSetDevice(device_id));

    /* ── Initialize driver context AFTER cudaSetDevice ── */
    if (init_drv_context(device_id) != 0) {
        fprintf(stderr, "WARN: Driver context init failed, tokens may be 0\n");
    }

    cudaDeviceProp prop;
    CUDA_RT_CHECK(cudaGetDeviceProperties(&prop, device_id));
    printf("\nUsing device %d: %s\n", device_id, prop.name);

    /* ── Allocate GPU buffer ── */
    void *d_buf = NULL;
    CUDA_RT_CHECK(cudaMalloc(&d_buf, TEST_SIZE));

    /* Zero it so we can detect if DMA wrote anything */
    CUDA_RT_CHECK(cudaMemset(d_buf, 0, TEST_SIZE));
    CUDA_RT_CHECK(cudaDeviceSynchronize());

    unsigned long long gpu_va = (unsigned long long)d_buf;

    /* ── Get P2P tokens from CUDA driver API ── */
    CUDA_POINTER_ATTRIBUTE_P2P_TOKENS p2p_tokens;
    CUresult cu_err = cuPointerGetAttribute(&p2p_tokens,
        CU_POINTER_ATTRIBUTE_P2P_TOKENS, (CUdeviceptr)d_buf);

    printf("\n=== P2P Token Retrieval ===\n");
    if (cu_err == CUDA_SUCCESS) {
        printf("PASS: cuPointerGetAttribute succeeded\n");
        printf("  P2P Token:      0x%llx\n", p2p_tokens.p2pToken);
        printf("  VA Space Token: %u\n", p2p_tokens.vaSpaceToken);
    } else {
        const char *estr;
        cuGetErrorString(cu_err, &estr);
        printf("FAIL: cuPointerGetAttribute failed: %s (code=%d)\n", estr, (int)cu_err);
        printf("  This GPU/driver may not support P2P token retrieval\n");
        printf("  Tokens defaulting to 0\n");
        p2p_tokens.p2pToken = 0;
        p2p_tokens.vaSpaceToken = 0;
    }

    /* ── Also check P2P peer access capabilities ── */
    printf("\n=== P2P Peer Access ===\n");
    for (int i = 0; i < device_count; i++) {
        if (i == device_id) continue;
        int can_access = 0;
        CUDA_RT_CHECK(cudaDeviceCanAccessPeer(&can_access, device_id, i));
        cudaDeviceProp peer_prop;
        CUDA_RT_CHECK(cudaGetDeviceProperties(&peer_prop, i));
        printf("  Device %d (%s) can access device %d (%s): %s\n",
               device_id, prop.name, i, peer_prop.name,
               can_access ? "YES" : "NO");
    }

    /* ── Print executor command ── */
    printf("\nGPU VA:      0x%llx\n", gpu_va);
    printf("Buffer size:  %d bytes\n", TEST_SIZE);

    printf("\nRun executor in another terminal:\n");
    printf("  alka-executor test_p2p.alkas alka-handoff/gtx960_2gb.alkavl \\\n");
    printf("    --cooperative --gpu-va 0x%llx \\\n", gpu_va);
    printf("    --p2p-token 0x%llx --va-space-token %u \\\n",
           p2p_tokens.p2pToken, p2p_tokens.vaSpaceToken);
    printf("    --source <model.gguf>\n\n");

    /* ── Wait for user to run executor ── */
    printf("Press Enter after executor completes (or Ctrl+C to abort)...");
    getchar();

    /* ── Read back from GPU ── */
    unsigned char *h_buf = (unsigned char *)malloc(TEST_SIZE);
    if (!h_buf) {
        fprintf(stderr, "malloc failed\n");
        cudaFree(d_buf);
        return 1;
    }

    CUDA_RT_CHECK(cudaMemcpy(h_buf, d_buf, TEST_SIZE, cudaMemcpyDeviceToHost));
    CUDA_RT_CHECK(cudaFree(d_buf));

    /* ── Verify ── */
    printf("\n=== DMA Verification ===\n");

    int all_zero = 1;
    for (int i = 0; i < TEST_SIZE; i++) {
        if (h_buf[i] != 0) {
            all_zero = 0;
            break;
        }
    }

    if (all_zero) {
        printf("FAIL: Buffer is still all zeros - DMA did not write\n");
        free(h_buf);
        return 1;
    }

    printf("Buffer is non-zero - DMA wrote something\n");

    if (memcmp(h_buf, GGUF_MAGIC, 4) == 0) {
        printf("PASS: First 4 bytes = GGUF magic (47 47 55 46)\n");
    } else {
        printf("WARN: First 4 bytes are not GGUF magic\n");
        printf("  Got: %02x %02x %02x %02x\n", h_buf[0], h_buf[1], h_buf[2], h_buf[3]);
    }

    printf("\nFirst 64 bytes of GPU buffer:\n");
    for (int i = 0; i < 64; i++) {
        if (i > 0 && i % 16 == 0) printf("\n");
        printf("%02x ", h_buf[i]);
    }
    printf("\n");

    if (gguf_path) {
        FILE *f = fopen(gguf_path, "rb");
        if (!f) {
            fprintf(stderr, "Cannot open GGUF: %s\n", gguf_path);
        } else {
            unsigned char *expected = (unsigned char *)malloc(TEST_SIZE);
            size_t n = fread(expected, 1, TEST_SIZE, f);
            fclose(f);

            if (n < TEST_SIZE) {
                printf("WARN: GGUF file is smaller than test buffer (%zu < %d)\n", n, TEST_SIZE);
            }

            int match = 1;
            size_t check_len = (n < TEST_SIZE) ? n : TEST_SIZE;
            for (size_t i = 0; i < check_len; i++) {
                if (h_buf[i] != expected[i]) {
                    match = 0;
                    printf("MISMATCH at byte %zu: GPU=%02x expected=%02x\n",
                           i, h_buf[i], expected[i]);
                    if (i > 64) break;
                }
            }

            if (match) {
                printf("\nPASS: All %zu bytes match GGUF source - DMA is CORRECT\n", check_len);
            }
            free(expected);
        }
    }

    free(h_buf);
    return 0;
}
