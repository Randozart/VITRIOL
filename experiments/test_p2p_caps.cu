/*
 * test_p2p_caps.cu — VITRIOL P2P Capability Diagnostic Tool
 *
 * Tests every GPU for P2P token support, peer access, and
 * physical page pinning — records results for findings doc.
 *
 * Usage:
 *   ./test_p2p_caps        # Test all GPUs
 *   ./test_p2p_caps 0      # Test specific device only
 *
 * Tests both runtime (cudaMalloc) and driver (cuMemAlloc) allocations.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <cuda_runtime.h>
#include <cuda.h>

#define TEST_ALLOC_SIZE 65536  /* 64KB */
#define MAX_DEVICES 8

static int device_count = 0;

/* Test P2P tokens on a driver API allocation (guarantees same context) */
static void test_driver_alloc(int dev_id, CUdevice cu_dev, CUcontext cu_ctx,
                              cudaDeviceProp *prop)
{
    CUdeviceptr d_ptr;
    CUresult err;

    err = cuMemAlloc(&d_ptr, TEST_ALLOC_SIZE);
    if (err != CUDA_SUCCESS) {
        const char *estr;
        cuGetErrorString(err, &estr);
        printf("  [DRV] cuMemAlloc: FAIL (%s)\n", estr);
        return;
    }

    err = cuMemsetD8(d_ptr, 0, TEST_ALLOC_SIZE);
    cuCtxSynchronize();

    unsigned long long gpu_va = (unsigned long long)d_ptr;
    printf("  [DRV] GPU VA:             0x%llx\n", gpu_va);

    /* cuMemGetAddressRange */
    CUdeviceptr base_ptr;
    size_t alloc_size;
    err = cuMemGetAddressRange(&base_ptr, &alloc_size, d_ptr);
    if (err == CUDA_SUCCESS) {
        printf("  [DRV] cuMemGetAddressRange: PASS (base=0x%llx, size=%zu)\n",
               (unsigned long long)base_ptr, alloc_size);
    } else {
        const char *estr;
        cuGetErrorString(err, &estr);
        printf("  [DRV] cuMemGetAddressRange: FAIL (%s)\n", estr);
    }

    /* cuPointerGetAttribute for P2P tokens */
    CUDA_POINTER_ATTRIBUTE_P2P_TOKENS p2p_tokens;
    err = cuPointerGetAttribute(&p2p_tokens,
        CU_POINTER_ATTRIBUTE_P2P_TOKENS, d_ptr);

    if (err == CUDA_SUCCESS) {
        printf("  [DRV] cuPointerGetAttribute: PASS\n");
        printf("  [DRV]   P2P Token:      0x%llx\n", p2p_tokens.p2pToken);
        printf("  [DRV]   VA Space Token: %u\n", p2p_tokens.vaSpaceToken);
        if (p2p_tokens.p2pToken != 0 || p2p_tokens.vaSpaceToken != 0)
            printf("  [DRV]   → MEANINGFUL TOKENS\n");
        else
            printf("  [DRV]   → TOKENS ARE ZERO\n");
    } else {
        const char *estr;
        cuGetErrorString(err, &estr);
        printf("  [DRV] cuPointerGetAttribute: FAIL (%s)\n", estr);
    }

    cuMemFree(d_ptr);
}

/* Test P2P tokens on a runtime API allocation */
static void test_runtime_alloc(int dev_id, CUdevice cu_dev, CUcontext cu_ctx,
                               cudaDeviceProp *prop)
{
    void *d_buf = NULL;
    cudaError_t ce = cudaMalloc(&d_buf, TEST_ALLOC_SIZE);
    if (ce != cudaSuccess) {
        printf("  [RT]  cudaMalloc: FAIL (%s)\n", cudaGetErrorString(ce));
        return;
    }

    cudaMemset(d_buf, 0, TEST_ALLOC_SIZE);
    cudaDeviceSynchronize();

    unsigned long long gpu_va = (unsigned long long)d_buf;
    printf("  [RT]  GPU VA:             0x%llx\n", gpu_va);

    CUDA_POINTER_ATTRIBUTE_P2P_TOKENS p2p_tokens;
    CUresult err = cuPointerGetAttribute(&p2p_tokens,
        CU_POINTER_ATTRIBUTE_P2P_TOKENS, (CUdeviceptr)d_buf);

    if (err == CUDA_SUCCESS) {
        printf("  [RT]  cuPointerGetAttribute: PASS\n");
        printf("  [RT]    P2P Token:      0x%llx\n", p2p_tokens.p2pToken);
        printf("  [RT]    VA Space Token: %u\n", p2p_tokens.vaSpaceToken);
        if (p2p_tokens.p2pToken != 0 || p2p_tokens.vaSpaceToken != 0)
            printf("  [RT]    → MEANINGFUL TOKENS\n");
        else
            printf("  [RT]    → TOKENS ARE ZERO\n");
    } else {
        const char *estr;
        cuGetErrorString(err, &estr);
        printf("  [RT]  cuPointerGetAttribute: FAIL (%s)\n", estr);
    }

    cudaFree(d_buf);
}

int main(int argc, char *argv[])
{
    int target_device = -1;
    if (argc > 1) target_device = atoi(argv[1]);

    printf("=== VITRIOL P2P Capability Diagnostic ===\n");
    printf("Date: 2026-05-15\n\n");

    CUresult cu_err = cuInit(0);
    if (cu_err != CUDA_SUCCESS) {
        const char *estr;
        cuGetErrorString(cu_err, &estr);
        fprintf(stderr, "FATAL: cuInit failed: %s\n", estr);
        return 1;
    }

    cudaError_t ce = cudaGetDeviceCount(&device_count);
    if (ce != cudaSuccess || device_count == 0) {
        fprintf(stderr, "No CUDA devices found\n");
        return 1;
    }

    printf("Found %d CUDA device(s)\n\n", device_count);

    /* ── Device enumeration ── */
    printf("Test 1: Device Enumeration\n");
    printf("----------------------------------------------------------------\n");
    for (int i = 0; i < device_count; i++) {
        cudaDeviceProp prop;
        cudaGetDeviceProperties(&prop, i);
        printf("[%d] %s  PCI=%04x:%02x:%02x  VRAM=%luMB  CC=%d.%d\n",
               i, prop.name, prop.pciDomainID, prop.pciBusID,
               prop.pciDeviceID, prop.totalGlobalMem / (1024*1024),
               prop.major, prop.minor);
    }
    printf("----------------------------------------------------------------\n\n");

    /* ── Per-device tests ── */
    for (int dev = 0; dev < device_count; dev++) {
        if (target_device >= 0 && dev != target_device)
            continue;

        cudaDeviceProp prop;
        cudaGetDeviceProperties(&prop, dev);
        printf("Test 2: P2P Token Retrieval on Device %d (%s)\n", dev, prop.name);
        printf("----------------------------------------------------------------\n");

        /* Get driver device handle */
        CUdevice cu_dev;
        CUcontext cu_ctx;
        cu_err = cuDeviceGet(&cu_dev, dev);
        if (cu_err != CUDA_SUCCESS) {
            printf("  SKIP: cuDeviceGet failed\n");
            printf("----------------------------------------------------------------\n\n");
            continue;
        }

        /* Retain and set primary context */
        cu_err = cuDevicePrimaryCtxRetain(&cu_ctx, cu_dev);
        if (cu_err != CUDA_SUCCESS) {
            printf("  SKIP: cuDevicePrimaryCtxRetain failed\n");
            printf("----------------------------------------------------------------\n\n");
            continue;
        }

        cu_err = cuCtxSetCurrent(cu_ctx);
        if (cu_err != CUDA_SUCCESS) {
            printf("  SKIP: cuCtxSetCurrent failed\n");
            printf("----------------------------------------------------------------\n\n");
            continue;
        }

        /* Also set runtime device for cudaMalloc tests */
        cudaSetDevice(dev);

        /* Test with driver API allocation (guaranteed same context) */
        printf("  → Driver API allocation:\n");
        test_driver_alloc(dev, cu_dev, cu_ctx, &prop);

        /* Test with runtime API allocation (may be different context) */
        printf("  → Runtime API allocation:\n");
        test_runtime_alloc(dev, cu_dev, cu_ctx, &prop);

        printf("----------------------------------------------------------------\n\n");
    }

    /* ── Peer access matrix (using cudaDeviceCanAccessPeer) ── */
    printf("Test 3: P2P Peer Access Matrix (cudaDeviceCanAccessPeer)\n");
    printf("----------------------------------------------------------------\n");
    printf("         ");
    for (int j = 0; j < device_count; j++) {
        cudaDeviceProp p;
        cudaGetDeviceProperties(&p, j);
        printf("  [%d]  ", j);
    }
    printf("\n");

    for (int i = 0; i < device_count; i++) {
        cudaDeviceProp pi;
        cudaGetDeviceProperties(&pi, i);
        printf(" [%d] %-8s", i, pi.name);
        for (int j = 0; j < device_count; j++) {
            if (i == j) {
                printf("  X   ");
                continue;
            }
            int can_access = 0;
            cudaSetDevice(i);
            cudaError_t err = cudaDeviceCanAccessPeer(&can_access, i, j);
            if (err == cudaSuccess)
                printf("  %s  ", can_access ? "YES" : "NO ");
            else
                printf("  ERR ");
        }
        printf("\n");
    }
    printf("----------------------------------------------------------------\n\n");

    /* ── nvidia-smi topology check ── */
    printf("Test 4: nvidia-smi P2P Topology\n");
    printf("----------------------------------------------------------------\n");
    int ret = system("nvidia-smi topo -p2p r 2>&1");
    if (ret != 0)
        printf("  (nvidia-smi not available or error)\n");
    printf("----------------------------------------------------------------\n\n");

    /* ── Summary ── */
    printf("Test 5: Summary & Recommended Path\n");
    printf("----------------------------------------------------------------\n");
    for (int dev = 0; dev < device_count; dev++) {
        cudaDeviceProp prop;
        cudaGetDeviceProperties(&prop, dev);
        printf("Device %d (%s): CC %d.%d\n", dev, prop.name, prop.major, prop.minor);

        CUdevice cu_dev;
        CUcontext cu_ctx;
        if (cuDeviceGet(&cu_dev, dev) != CUDA_SUCCESS ||
            cuDevicePrimaryCtxRetain(&cu_ctx, cu_dev) != CUDA_SUCCESS) {
            printf("  → Level 1 (userspace BIND) only\n");
            continue;
        }
        cuCtxSetCurrent(cu_ctx);

        CUdeviceptr d_ptr;
        if (cuMemAlloc(&d_ptr, TEST_ALLOC_SIZE) != CUDA_SUCCESS) {
            printf("  → Level 1 (userspace BIND) only\n");
            continue;
        }

        CUDA_POINTER_ATTRIBUTE_P2P_TOKENS tokens;
        CUresult err = cuPointerGetAttribute(&tokens,
            CU_POINTER_ATTRIBUTE_P2P_TOKENS, d_ptr);
        cuMemFree(d_ptr);

        if (err == CUDA_SUCCESS && (tokens.p2pToken != 0 || tokens.vaSpaceToken != 0)) {
            printf("  → Level 3 (cooperative P2P) ✓\n");
        } else if (err == CUDA_SUCCESS && tokens.p2pToken == 0 && tokens.vaSpaceToken == 0) {
            printf("  → Level 3 (cooperative P2P) — tokens=0, may still work\n");
        } else {
            printf("  → Level 2 (kernel BIND) or Level 1 (userspace BIND)\n");
        }
    }
    printf("----------------------------------------------------------------\n");

    return 0;
}
