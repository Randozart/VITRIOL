/*
 * test_mem_create.cu — Test cuMemCreate for RDMA-capable allocation
 *
 * CUDA 11+ Virtual Memory Management API can allocate memory
 * with specific handle types (including POSIX fd for RDMA).
 *
 * Tests whether this works on our GPUs.
 */

#include <stdio.h>
#include <stdlib.h>
#include <cuda_runtime.h>
#include <cuda.h>

#define TEST_SIZE (64 * 1024)  /* 64KB */
#define CHECK(x) do { CUresult __e = (x); if (__e != CUDA_SUCCESS) { \
    const char *estr; cuGetErrorString(__e, &estr); \
    printf("  FAIL at %s:%d: %s (code=%d)\n", __FILE__, __LINE__, estr, (int)__e); \
    goto error; } } while(0)

int main(void)
{
    cuInit(0);

    int device_count;
    cudaGetDeviceCount(&device_count);

    printf("=== cuMemCreate RDMA-capable Allocation Test ===\n\n");

    for (int dev = 0; dev < device_count; dev++) {
        cudaDeviceProp prop;
        cudaGetDeviceProperties(&prop, dev);
        printf("Device %d: %s (CC %d.%d)\n", dev, prop.name, prop.major, prop.minor);

        CUdevice cu_dev;
        CUcontext cu_ctx;
        CHECK(cuDeviceGet(&cu_dev, dev));

        /* Check virtual memory management support */
        int virt_mem_supported = 0;
        cuDeviceGetAttribute(&virt_mem_supported,
            CU_DEVICE_ATTRIBUTE_VIRTUAL_MEMORY_MANAGEMENT_SUPPORTED, cu_dev);
        printf("  Virtual Memory Mgmt: %s\n", virt_mem_supported ? "YES" : "NO");

        if (!virt_mem_supported) {
            printf("  SKIP: Device doesn't support virtual memory management\n\n");
            continue;
        }

        CHECK(cuDevicePrimaryCtxRetain(&cu_ctx, cu_dev));
        CHECK(cuCtxSetCurrent(cu_ctx));
        cudaSetDevice(dev);

        /* Try cuMemCreate with RDMA handle type */
        CUmemAllocationProp alloc_prop = {};
        alloc_prop.type = CU_MEM_ALLOCATION_TYPE_PINNED;
        alloc_prop.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
        alloc_prop.location.id = dev;
        alloc_prop.requestedHandleTypes = (CUmemAllocationHandleType)CU_MEM_HANDLE_TYPE_POSIX_FILE_DESCRIPTOR;

        CUmemGenericAllocationHandle handle;
        CUresult err = cuMemCreate(&handle, TEST_SIZE, &alloc_prop, 0);

        if (err == CUDA_SUCCESS) {
            printf("  cuMemCreate: PASS (handle=0x%llx)\n", (unsigned long long)handle);

            /* Allocate VA and map */
            CUdeviceptr dptr;
            CHECK(cuMemAddressReserve(&dptr, TEST_SIZE, 0, 0, 0));
            CHECK(cuMemMap(dptr, TEST_SIZE, 0, handle, 0));

            /* Set access */
            CUmemAccessDesc access_desc = {};
            access_desc.location.type = CU_MEM_LOCATION_TYPE_DEVICE;
            access_desc.location.id = dev;
            access_desc.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
            CHECK(cuMemSetAccess(dptr, TEST_SIZE, &access_desc, 1));

            printf("  Mapped VA: 0x%llx\n", (unsigned long long)dptr);

            /* Zero it */
            CHECK(cuMemsetD8(dptr, 0, TEST_SIZE));
            cuCtxSynchronize();

            /* Now test cuPointerGetAttribute for P2P tokens */
            CUDA_POINTER_ATTRIBUTE_P2P_TOKENS tokens;
            err = cuPointerGetAttribute(&tokens,
                CU_POINTER_ATTRIBUTE_P2P_TOKENS, dptr);

            if (err == CUDA_SUCCESS) {
                printf("  P2P Tokens: token=0x%llx va_token=%u\n",
                       tokens.p2pToken, tokens.vaSpaceToken);
                printf("  → RDMA-capable allocation SUCCEEDED!\n");
            } else {
                const char *estr;
                cuGetErrorString(err, &estr);
                printf("  P2P Tokens: FAIL (%s)\n", estr);

                /* Check IS_GPU_DIRECT_RDMA_CAPABLE */
                int is_rdma = -1;
                cuPointerGetAttribute(&is_rdma,
                    CU_POINTER_ATTRIBUTE_IS_GPU_DIRECT_RDMA_CAPABLE, dptr);
                printf("  IS_GPU_DIRECT_RDMA_CAPABLE: %d\n", is_rdma);
            }

            /* Check allowed handle types */
            unsigned int handle_mask = 0;
            cuPointerGetAttribute(&handle_mask,
                CU_POINTER_ATTRIBUTE_ALLOWED_HANDLE_TYPES, dptr);
            printf("  ALLOWED_HANDLE_TYPES: 0x%x\n", handle_mask);

            cuMemUnmap(dptr, TEST_SIZE);
            cuMemAddressFree(dptr, TEST_SIZE);
            cuMemRelease(handle);
        } else {
            const char *estr;
            cuGetErrorString(err, &estr);
            printf("  cuMemCreate: FAIL (%s)\n", estr);

            /* Try without requested handle types */
            alloc_prop.requestedHandleTypes = (CUmemAllocationHandleType)CU_MEM_HANDLE_TYPE_NONE;
            err = cuMemCreate(&handle, TEST_SIZE, &alloc_prop, 0);
            if (err == CUDA_SUCCESS) {
                printf("  cuMemCreate (no handle types): PASS\n");
                cuMemRelease(handle);
            } else {
                cuGetErrorString(err, &estr);
                printf("  cuMemCreate (no handle types): FAIL (%s)\n", estr);
            }
        }

        printf("\n");
    }

    return 0;

error:
    return 1;
}
