#include "common.cuh"

void ggml_cuda_op_escha_moe(ggml_backend_cuda_context & ctx, ggml_tensor * dst);
void ggml_cuda_op_escha_mul_mat(ggml_backend_cuda_context & ctx, ggml_tensor * dst);

// mul = silu(gate(x)) * up(x) for two dense projection nodes on the prefill path, with one epilogue kernel. Returns false if it does not take the nodes.
// down != nullptr: also the projection that reads mul; mul then gets no data.
bool ggml_cuda_op_escha_swiglu(ggml_backend_cuda_context & ctx, ggml_tensor * gate, ggml_tensor * up, ggml_tensor * mul, ggml_tensor * down);
