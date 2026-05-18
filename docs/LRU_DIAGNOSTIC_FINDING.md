# VITRIOL LRU Cache — Diagnostic Finding (2026-05-18)

## Discovery

The LRU VRAM cache (`vitriol_lru_ensure`) is **never called** during inference with Q2_K_XL quantized MoE models. It is unreachable dead code.

## Root Cause

The function `ggml_cuda_mul_mat_id` in `ggml-cuda.cu` has **three early return paths** that bypass the LRU cache entirely:

```cpp
// All three return before reaching the LRU code at line 2597+

// Fast path 1: quantized matrix-vector
if (ggml_is_quantized(src0->type)) {
    ggml_cuda_mul_mat_vec_q(ctx, src0, src1, ids, dst);
    return;
}

// Fast path 2: quantized matrix-matrix (matches Q2_K_XL)
if (ggml_cuda_should_use_mmq(src0->type, cc, ne12, /*n_experts=*/ne02)) {
    ggml_cuda_mul_mat_q(ctx, src0, src1, ids, dst);
    return;
}

// Fast path 3: FP16 matrix-matrix
if (ggml_cuda_should_use_mmf(...)) {
    ggml_cuda_mul_mat_f(ctx, src0, src1, ids, dst);
    return;
}

// Slow path — never reached for quantized types:
//   - Iterates over 256 experts manually
//   - Calls vitriol_lru_ensure() per expert
//   - Computes matmul one expert at a time
for (int64_t i02 = 0; i02 < ne02; ++i02) {
    if (vitriol_is_stream_enabled()) {
        CUdeviceptr vram_ptr = vitriol_lru_ensure(...); // NEVER CALLED
    }
    // ...
}
```

For Q2_K_XL (and any quantized type), the MMQ fast path (path 2) matches on line 2552 and returns immediately. The LRU code at line 2644+ is never reached.

## How Inference Actually Works

1. VITRIOL buffer type allocates page-locked system RAM via `cudaHostRegister`
2. Expert tensors are loaded into this buffer (10 GB total)
3. During inference, `ggml_cuda_mul_mat_id` takes the MMQ fast path
4. The fast MMQ kernel reads expert weights directly from the page-locked host buffer via **PCIe DMA**
5. No VRAM caching occurs — every expert access is a PCIe DMA read from host memory

The `cudaHostRegister` call is what enables this — it makes the host buffer accessible to GPU kernels as if it were device memory. The LRU cache is not involved.

## Impact on lru_mb Setting

The `lru_mb` configuration setting (and `VITRIOL_LRU_MB` env var, and `--lru` CLI flag) **does nothing** for Q2_K_XL and any quantized MoE model. The LRU pool is never allocated, never consulted, and never used. The VRAM never changes regardless of the LRU value.

This was confirmed by:
1. VRAM measurements: identical at LRU=512, LRU=1024, LRU=2048, LRU=4096 (all ~3921 MiB at 254k context)
2. Code trace: `vitriol_lru_ensure` has zero calls during inference (confirmed via `fprintf(stderr)` diagnostic)
3. Binary examination: `lru_init_pool` string present in library, function never invoked at runtime

## When Would the LRU Cache Be Used?

The LRU cache would only be active if:
1. The model uses **unquantized (FP16/FP32) expert weights** — the MMQ fast path would not match, and the function would fall through to the slow path
2. OR the model type forces the slow path (e.g., certain non-standard formats)
3. OR the early-return conditions in `ggml_cuda_mul_mat_id` are modified to skip the fast paths when VITRIOL stream mode is active

**Do not do option 3** — the fast MMQ kernel is 2-3× more efficient than the slow path even WITH LRU caching. Forcing the slow path would reduce performance.

## Recommendation

| Scenario | lru_mb Setting | Reasoning |
|----------|---------------|-----------|
| Q2_K_XL or any quantized MoE | **0** (disable) | LRU is unreachable; setting has no effect |
| FP16/FP32 MoE | 512-2048 | Slow path is used; LRU would help |
| Unknown model type | 0 | Safe default; enable only if profiling confirms LRU is reached |

The LRU C++ code (`vitriol_lru_ensure`, `lru_init_pool`, `vitriol_lru_prefetch`) is retained for potential use with higher-precision models. The config default is set to 0 to avoid confusion.

## Verification

To verify LRU is not being used on your system:
```bash
# Add to vitriol-cuda-integration.cpp and rebuild:
fprintf(stderr, "VITRIOL: lru_ensure called\n");
# Then run inference and grep for it:
grep "lru_ensure" server.log  # → zero results for quantized models
```
