// Build and run:  nvcc -O2 -arch=sm_86 scripts/probes/probe-kv-tier-vmm.cu -o /tmp/b0_probe -lcuda && /tmp/b0_probe
// Exits 0 when the driver maps pinned system RAM into a device VA range (tiered KV feasible),
// 2 when no host location type is accepted, 3 when the bytes across the line do not match.
// Evidence for .opencode/plans/kv-tier-vram-cells-2026-11-25.md section 9 (B0).
// B0: does this host's driver map pinned system RAM into a device VA range?
// Mirrors kintsugi-engine 8f12baacf (MIT) ggml_cuda_hostmem_create / tier_alloc ordering.
#include <cuda.h>
#include <cuda_runtime.h>
#include <cstdio>
#include <cstring>
#include <vector>

#define CK(x) do { CUresult r = (x); if (r != CUDA_SUCCESS) { const char * s = nullptr; cuGetErrorString(r, &s); \
    printf("  FAIL %-28s %s\n", #x, s ? s : "?"); return 1; } } while (0)

static CUmemAllocationProp prop(bool host, bool numa, int node) {
    CUmemAllocationProp p = {};
    p.type = CU_MEM_ALLOCATION_TYPE_PINNED;
    p.location.type = host ? (numa ? CU_MEM_LOCATION_TYPE_HOST_NUMA : CU_MEM_LOCATION_TYPE_HOST)
                           : CU_MEM_LOCATION_TYPE_DEVICE;
    p.location.id = host && numa ? node : (host ? 0 : 0);
    return p;
}

__global__ void fill(int * p, int n, int v) { for (int i = blockIdx.x*blockDim.x + threadIdx.x; i < n; i += blockDim.x*gridDim.x) p[i] = v + i; }

int main() {
    int ndev = 0;
    if (cuInit(0) != CUDA_SUCCESS) { printf("cuInit failed\n"); return 1; }
    cuDeviceGetCount(&ndev);
    printf("devices: %d\n", ndev);
    CUdevice dev; cuDeviceGet(&dev, 0);
    char name[256] = {}; cuDeviceGetName(name, sizeof name, dev);
    int drv = 0, rt = 0; cuDriverGetVersion(&drv); cudaRuntimeGetVersion(&rt);
    int node = -1; cuDeviceGetAttribute(&node, CU_DEVICE_ATTRIBUTE_HOST_NUMA_ID, dev);
    printf("gpu: %s | CUDA driver API %.1f | runtime %.1f | host NUMA id %d\n",
           name, drv/1000.0, rt/1000.0, node);
    if (node < 0) node = 0;

    // 1. does the driver accept a host location type at all?
    CUmemGenericAllocationHandle hv = 0, hh = 0;
    const size_t len = 64ull << 20;   // 64 MiB
    bool host_ok = false, numa_ok = false, host_first = false;
    CUmemAllocationProp ph = prop(true, false, node);
    printf("\n[1] host location types\n");
    CUresult r = cuMemCreate(&hh, len, &ph, 0);
    if (r == CUDA_SUCCESS) {
        host_ok = true; host_first = true;
        printf("  OK   CU_MEM_LOCATION_TYPE_HOST (64 MiB)\n");
    } else {
        const char * s = nullptr; cuGetErrorString(r, &s);
        printf("  no   CU_MEM_LOCATION_TYPE_HOST: %s\n", s ? s : "?");
    }
    if (!host_ok) {
        CUmemAllocationProp pn = prop(true, true, node);
        r = cuMemCreate(&hh, len, &pn, 0);
        if (r == CUDA_SUCCESS) { numa_ok = true; printf("  OK   CU_MEM_LOCATION_TYPE_HOST_NUMA node %d (fallback)\n", node); }
        else { const char * s = nullptr; cuGetErrorString(r, &s); printf("  no   HOST_NUMA: %s\n", s ? s : "?"); }
    }
    if (!host_ok && !numa_ok) { printf("\nVERDICT: no host location type -> tiered KV is IMPOSSIBLE on this driver\n"); return 2; }
    (void) host_first;

    size_t gh = 0;
    { CUmemAllocationProp p = host_ok ? prop(true, false, node) : prop(true, true, node);
      if (cuMemGetAllocationGranularity(&gh, &p, CU_MEM_ALLOC_GRANULARITY_MINIMUM) != CUDA_SUCCESS) return 1; }
    printf("  host granularity %zu KiB\n", gh/1024);

    // 2. VRAM handle + granularity
    CUmemAllocationProp pd = prop(false, false, 0);
    pd.location.id = 0;
    size_t gd = 0;
    CK(cuMemGetAllocationGranularity(&gd, &pd, CU_MEM_ALLOC_GRANULARITY_MINIMUM));
    printf("\n[2] vram granularity %zu KiB\n", gd/1024);

    // 3. one VA range, head VRAM + tail host, as the tier buffer does
    const size_t gran = gd > gh ? gd : gh;
    const size_t vlen = ((len/2) + gran - 1)/gran*gran;   // VRAM head
    const size_t hlen = (len - vlen + gran - 1)/gran*gran; // host tail
    const size_t total = vlen + hlen;
    printf("\n[3] VA range %.1f MiB = vram %.1f + host %.1f (gran %zu KiB)\n",
           total/1048576.0, vlen/1048576.0, hlen/1048576.0, gran/1024);

    CUdeviceptr va = 0;
    CK(cuMemAddressReserve(&va, total, gran, 0, 0));
    printf("  reserved va %p\n", (void *) va);

    if (cuMemCreate(&hv, vlen, &pd, 0) != CUDA_SUCCESS) { printf("  FAIL vram handle\n"); return 1; }
    // host handle: reuse the one created above if its size matched, else create
    if (hlen != len) {
        cuMemRelease(hh);
        CUmemAllocationProp p = host_ok ? prop(true, false, node) : prop(true, true, node);
        if (cuMemCreate(&hh, hlen, &p, 0) != CUDA_SUCCESS) { printf("  FAIL host handle\n"); return 1; }
    }
    CK(cuMemMap(va, vlen, 0, hv, 0));
    CK(cuMemMap(va + vlen, hlen, 0, hh, 0));
    printf("  mapped vram + host runs in one range\n");

    CUmemAccessDesc ad = {};
    ad.location = pd.location;
    ad.flags = CU_MEM_ACCESS_FLAGS_PROT_READWRITE;
    CK(cuMemSetAccess(va, total, &ad, 1));
    printf("  set access readwrite for device 0\n");

    // 4. a kernel writes across the boundary and a device->host copy reads it back
    printf("\n[4] kernel access across the vram/host line\n");
    size_t n = total/4;                                  // ints
    CUdeviceptr mid = va + vlen - 4096;                  // straddles the line
    fill<<<(unsigned) ((n+255)/256 > 1024 ? 1024 : (n+255)/256), 256>>>((int *) va, (int) n, 0);
    if (cudaDeviceSynchronize() != cudaSuccess) { printf("  FAIL kernel: %s\n", cudaGetErrorString(cudaGetLastError())); return 1; }
    std::vector<int> out(4096);
    if (cudaMemcpy(out.data(), (void *) mid, out.size()*4, cudaMemcpyDeviceToHost) != cudaSuccess) {
        printf("  FAIL copy back: %s\n", cudaGetErrorString(cudaGetLastError())); return 1;
    }
    int bad = 0;
    for (size_t i = 0; i < out.size(); ++i) {
        int want = (int) ((mid - va)/4) + (int) i;
        if (out[i] != want) { if (bad < 3) printf("  mismatch idx %zu got %d want %d\n", i, out[i], want); ++bad; }
    }
    printf("  %s: %zu ints verified around the line (%d bad)\n", bad ? "FAILED" : "PASS", out.size(), bad);

    // 5. clean release (leak-free teardown, invariant I9 in the plan)
    cuMemUnmap(va + vlen, hlen); cuMemUnmap(va, vlen);
    cuMemRelease(hv); cuMemRelease(hh);
    if (cuMemAddressFree(va, total) != CUDA_SUCCESS) { printf("  FAIL address free\n"); return 1; }
    printf("\nVERDICT: driver %s maps pinned host RAM into a device VA range; tiered KV is FEASIBLE here\n",
           host_ok ? "accepts CU_MEM_LOCATION_TYPE_HOST" : "accepts HOST_NUMA only (fallback path required)");
    return bad ? 3 : 0;
}
