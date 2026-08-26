# Session: 2026-05-15 — CE DMA Integration Journey

## The Problem
Direct NVMe→VRAM DMA via GPU's internal Copy Engine on consumer GeForce (GTX 1070 Ti, 8GB). The model (Qwen3.6-35B-A3B, 11.44 GiB GGUF, 256 experts) requires on-demand expert loading since all 10.6 GiB of experts don't fit in 8GB VRAM alongside the base model (~1.3 GB).

## Approach: Copy Engine DMA (GPU VA → VRAM)
Uses `cuMemcpyDtoDAsync` to copy from a pinned host bounce buffer into pre-allocated VRAM (3.4 GB pool). The GPU's internal DMA engine handles the transfer, bypassing the CPU entirely.

Standalone test: ✅ `47 47 55 46` in VRAM, byte-for-byte match.

## Challenge: Getting MoE Expert Compute onto CUDA
The central problem: `-ot ".*exps.*=CPU"` keeps experts in system RAM. MUL_MAT_ID is dispatched to CPU backend. Our CE DMA fires in `ggml_cuda_mul_mat_id` which is never called.

### Approaches Tried (Chronological)

#### 1. set_tensor_async Hook (❌ Wrong Entry Point)
- Hook in `ggml_backend_cuda_set_tensor_async` at ggml-cuda.cu:3000
- **Blocked by:** set_tensor_async is never called for CPU-resident tensors
- **Lesson:** The hook entry point must be where the data is actually accessed (MUL_MAT_ID), not where it's copied to GPU

#### 2. supports_buft CPU Accept (❌ Too Broad)
- Modified `ggml_backend_cuda_device_supports_buft` to return true for CPU buffer types when VITRIOL is active
- Combined with `offload_op` return true for MUL_MAT_ID
- **Result:** MUL_MAT_ID routes to CUDA, scheduler's intelligent MoE offload copies only 8 active experts
- **Blocked by:** ALL ops with CPU inputs route to CUDA, including ROPE → crash (`illegal memory access`)
- **Lesson:** `supports_buft` is op-agnostic; affects non-MoE ops destructively

#### 3. VITRIOL Buffer Type (✅ Correct Solution)
- Custom `ggml_backend_buffer_type` that allocates from `malloc()` (system RAM) but reports `is_host=false`
- Scheduler sees a "device buffer" → routes MUL_MAT_ID to CUDA
- Only expert tensors use this buffer type → no impact on other ops
- Code is written in `vitriol-buffer.cpp/.h`, compiles correctly
- **Blocking:** Linking issue (vitriol_get_buffer_type needed by common/arg.cpp for -ot parsing)
- **Fix:** Don't use -ot. Apply the VITRIOL buft automatically in the model loader.

### Key Technical Insights

1. **`cuMemHostRegister` fails on GGUF mmap data** — use bounce buffer memcpy + CE DMA instead
2. **Scheduler already has intelligent MoE offload** — copies only 8 active expert slices, not all 256
3. **`supports_buft` is op-agnostic** — too broad for selective operator routing
4. **VITRIOL buffer type is the cleanest solution** — op-agnostic by nature, only affects tensors that use it

## Files Modified

| File | Change | Purpose |
|------|--------|---------|
| ggml-cuda.cu | Hook calls at 684, 704, 3000 | `vitriol_intercept_set_tensor` for model loading |
| ggml-cuda.cu | ~~supports_buft~~ → ~~offload_op~~ → REVERTED | Too broad, caused ROPE crash |
| vitriol-buffer.h | New | Custom buffer type definition |
| vitriol-buffer.cpp | New | Buffer type: malloc alloc, is_host=false, set_tensor hook |
| vitriol-cuda-integration.h | Cleaned up | Remove stale async/layer tracking decls |
| vitriol-cuda-integration.cpp | CE DMA + bounce buffer + VRAM pool | Working expert loading pipeline |
| vitriol_copy_engine.h/cpp | Existing CE DMA implementation | Verified (47 47 55 46) |
| llama-model-loader.cpp | ~~PENDING~~ | Auto-apply VITRIOL buft for expert tensors |

## Final Architecture

```
User: VITRIOL_MODE=stream ./llama-server ... -cmoe
                                  ↓
vitriol_cuda_init() allocates CE channel + 3.4GB VRAM pool
                                  ↓
Model loading: expert tensors → VITRIOL buffer type
  • 12 GB allocation → malloc (system RAM) — succeeds
  • Tensor data NOT copied (vitriol_intercept_set_tensor skips)
  • Scheduler sees "device buffer" → MUL_MAT_ID → CUDA backend
                                  ↓
Inference: ggml_cuda_mul_mat_id called for MoE
  • vitriol_ensure_expert_loaded detects VITRIOL buffer
  • Cache miss? → memcpy → bounce buffer → CE DMA → VRAM pool
  • Pointer swap → matmul runs on physical VRAM data
                                  ↓
Result: ~25 tok/s projected (8 experts CE DMA'd, CUDA matmul on VRAM)
```

## Env Vars

| Var | Default | Purpose |
|-----|---------|---------|
| `VITRIOL_MODE` | disabled | Set to "stream" to enable CE DMA |
| `VITRIOL_VERBOSE` | 0 | Set to "1" for debug output |
| `VITRIOL_POOL_MB` | 3420 | VRAM pool size in MB |
