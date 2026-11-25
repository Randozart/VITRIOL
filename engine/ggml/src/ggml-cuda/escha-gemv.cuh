#include "common.cuh"

// small-batch path of ggml_escha_mul_mat (rows <= 8). Returns false if it does not take the call.
bool ggml_cuda_escha_gemv(ggml_backend_cuda_context & ctx, ggml_tensor * dst);
// the largest batch that ggml_cuda_escha_gemv takes (0: it takes no batch)
int ggml_cuda_escha_gemv_max_rows();

// The two rotations alone, one warp per 128 channels, for the prefill path in escha-moe.cu. Each returns false if it does not take the call (rows not 16-byte aligned).
//   xh[row]  = f16(T128(x[row] * rin))
//   dst[row] = T128(mid[row]) * rout, mid is [rows][OC] and can be the memory of dst
bool ggml_cuda_escha_had_in(ggml_backend_cuda_context & ctx, const ggml_tensor * x, const ggml_tensor * rin, half * xh);
bool ggml_cuda_escha_had_out(ggml_backend_cuda_context & ctx, const float * mid, const ggml_tensor * rout, ggml_tensor * dst);
//   dst[row] = T128(acc[row]*scales[row]) * rout, acc is [rows][OC] int32 and can be the memory of dst
bool ggml_cuda_escha_had_out_i32(ggml_backend_cuda_context & ctx, const int32_t * acc, const float * scales, const ggml_tensor * rout, ggml_tensor * dst);
//   dst[row] = silu(T128(g[row]) * rout_g) * (T128(u[row]) * rout_u). sg == nullptr: g and u are fp32. Else they are int32 sums with the row factors sg and su.
bool ggml_cuda_escha_swiglu_out(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                                const ggml_tensor * rout_g, const ggml_tensor * rout_u, ggml_tensor * dst);
//   xh[row] = f16(T128(silu(T128(g[row]) * rout_g) * (T128(u[row]) * rout_u) * rin)): the SwiGLU result as the rotated input of the next projection, xh is [rows][OC]
bool ggml_cuda_escha_swiglu_xh(ggml_backend_cuda_context & ctx, const void * g, const void * u, const float * sg, const float * su,
                               const ggml_tensor * rout_g, const ggml_tensor * rout_u, const ggml_tensor * rin, half * xh,
                               int OC, int n_rows);
