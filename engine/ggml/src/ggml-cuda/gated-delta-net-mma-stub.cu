// PROVENANCE: kintsugi-engine 8f12baacf (MIT). The chunked GDN prefill MMA kernel is
// Blackwell-gated (blackwell_mma_available) and needs mma.cuh features this fork lacks;
// stubbed to "not supported", matching what the reference returns on sm_86.
#include "gated-delta-net-mma.cuh"

bool ggml_cuda_gdn_mma_device_supported(int /*device*/) {
    return false;
}

bool ggml_cuda_gdn_mma_launch(int /*device*/, const ggml_cuda_gdn_mma_args & /*args*/, cudaStream_t /*stream*/) {
    return false;
}
