/*
 * test_p2p_attrs.cu — Debug CUDA pointer attributes for GPUDirect
 *
 * Tests every relevant pointer attribute to determine what's
 * actually supported on this driver/GPU combination.
 */

#include <stdio.h>
#include <stdlib.h>
#include <cuda_runtime.h>
#include <cuda.h>

#define TEST_ALLOC_SIZE 65536

static const char *attr_name(int id)
{
    switch (id) {
    case 1:  return "CONTEXT";
    case 2:  return "MEMORY_TYPE";
    case 3:  return "DEVICE_POINTER";
    case 4:  return "HOST_POINTER";
    case 5:  return "P2P_TOKENS";
    case 6:  return "SYNC_MEMOPS";
    case 7:  return "BUFFER_ID";
    case 8:  return "IS_MANAGED";
    case 9:  return "DEVICE_ORDINAL";
    case 10: return "IS_LEGACY_CUDA_IPC_CAPABLE";
    case 11: return "RANGE_START_ADDR";
    case 12: return "RANGE_SIZE";
    case 13: return "MAPPED";
    case 14: return "ALLOWED_HANDLE_TYPES";
    case 15: return "IS_GPU_DIRECT_RDMA_CAPABLE";
    case 16: return "ACCESS_FLAGS";
    case 17: return "MEMPOOL_HANDLE";
    case 18: return "MAPPING_SIZE";
    case 19: return "MAPPING_BASE_ADDR";
    case 20: return "MEMORY_BLOCK_ID";
    default: return "UNKNOWN";
    }
}

int main(int argc, char *argv[])
{
    int target_device = (argc > 1) ? atoi(argv[1]) : -1;

    cuInit(0);
    int device_count;
    cudaGetDeviceCount(&device_count);
    printf("=== CUDA Pointer Attribute Diagnostic ===\n\n");

    for (int dev = 0; dev < device_count; dev++) {
        if (target_device >= 0 && dev != target_device) continue;

        cudaDeviceProp prop;
        cudaGetDeviceProperties(&prop, dev);
        printf("Device %d: %s (CC %d.%d, VRAM %lu MB)\n",
               dev, prop.name, prop.major, prop.minor,
               prop.totalGlobalMem / (1024*1024));

        CUdevice cu_dev;
        CUcontext cu_ctx;
        cuDeviceGet(&cu_dev, dev);
        cuDevicePrimaryCtxRetain(&cu_ctx, cu_dev);
        cuCtxSetCurrent(cu_ctx);
        cudaSetDevice(dev);

        void *d_buf;
        cudaMalloc(&d_buf, TEST_ALLOC_SIZE);
        cudaMemset(d_buf, 0, TEST_ALLOC_SIZE);
        cudaDeviceSynchronize();

        printf("  GPU VA: 0x%llx\n\n", (unsigned long long)d_buf);

        /* Test every useful pointer attribute */
        int test_attrs[] = {1, 2, 3, 4, 5, 7, 8, 9, 13, 14, 15, 16};
        int num_tests = sizeof(test_attrs) / sizeof(test_attrs[0]);

        for (int t = 0; t < num_tests; t++) {
            int attr_id = test_attrs[t];

            switch (attr_id) {
            case 5: { /* P2P_TOKENS */
                CUDA_POINTER_ATTRIBUTE_P2P_TOKENS tokens;
                CUresult e = cuPointerGetAttribute(&tokens, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS  (token=0x%llx, va_token=%u)\n",
                           attr_name(attr_id), tokens.p2pToken, tokens.vaSpaceToken);
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            case 15: { /* IS_GPU_DIRECT_RDMA_CAPABLE */
                int val = -1;
                CUresult e = cuPointerGetAttribute(&val, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS  (val=%d)\n", attr_name(attr_id), val);
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            case 9: { /* DEVICE_ORDINAL */
                int val = -1;
                CUresult e = cuPointerGetAttribute(&val, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS  (dev=%d)\n", attr_name(attr_id), val);
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            case 14: { /* ALLOWED_HANDLE_TYPES */
                unsigned int val = 0;
                CUresult e = cuPointerGetAttribute(&val, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS  (mask=0x%x)\n", attr_name(attr_id), val);
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            case 7: { /* BUFFER_ID */
                unsigned long long val = 0;
                CUresult e = cuPointerGetAttribute(&val, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS  (id=0x%llx)\n", attr_name(attr_id), val);
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            default: {
                void *val = NULL;
                CUresult e = cuPointerGetAttribute(&val, (CUpointer_attribute)attr_id, (CUdeviceptr)d_buf);
                if (e == CUDA_SUCCESS) {
                    printf("  %-30s  PASS\n", attr_name(attr_id));
                } else {
                    const char *estr;
                    cuGetErrorString(e, &estr);
                    printf("  %-30s  FAIL  (%s)\n", attr_name(attr_id), estr);
                }
                break;
            }
            }
        }

        cudaFree(d_buf);
        printf("\n");
    }

    printf("\nNote: nvidia-peermem kernel module was NOT loaded.\n");
    printf("  'modprobe nvidia-peermem' failed with EINVAL.\n");
    printf("  This may be why P2P_TOKENS is unavailable.\n");

    return 0;
}
