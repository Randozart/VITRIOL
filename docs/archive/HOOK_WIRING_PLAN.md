# VITRIOL Hook Wiring Plan — 2026-05-13

## Goal
Wire the existing `vitriol_cuda_set_tensor_hook()` into llama.cpp's `ggml-cuda.cu` so it actually gets called during tensor transfers. Currently the hook exists but is never invoked from any code path.

## Changes

### File: `ggml/src/ggml-cuda/ggml-cuda.cu`

#### 1. Add include (after existing includes)
```c
#include "vitriol-cuda-integration.h"
```

#### 2. Wire into `ggml_backend_cuda_buffer_set_tensor` (line 678)
Model load path. Called once per tensor when loading the model from disk.
```c
static void ggml_backend_cuda_buffer_set_tensor(...) {
    ggml_backend_cuda_buffer_context * ctx = ...;
    ggml_cuda_set_device(ctx->device);
    if (vitriol_cuda_set_tensor_hook(tensor, data, size, 0)) {
        return;  // VITRIOL handled the transfer
    }
    CUDA_CHECK(cudaMemcpyAsync(...));  // standard path
    CUDA_CHECK(cudaStreamSynchronize(...));
}
```

#### 3. Wire into `ggml_backend_cuda_buffer_set_tensor_2d` (line 694)
Same pattern for 2D transfers.

#### 4. Wire into `ggml_backend_cuda_set_tensor_async` (line 2986)
Inference hot path. Called every token for expert slice transfers.
```c
static void ggml_backend_cuda_set_tensor_async(...) {
    ...
    if (vitriol_cuda_set_tensor_hook(tensor, data, size, 0)) {
        return;  // VITRIOL handled the transfer
    }
    CUDA_CHECK(cudaMemcpyAsync(...));
}
```

#### 5. Add initialization call
Call `vitriol_cuda_init()` in the CUDA backend initialization.

### Behavior When Not Enabled

When `g_vitriol_config.mode == VITRIOL_MODE_DISABLED` (default), the hook immediately returns `false` at line 109 of `vitriol-cuda-integration.cpp`:
```c
if (g_vitriol_config.mode == VITRIOL_MODE_DISABLED) {
    return false;
}
```
**Zero overhead when disabled.** Single boolean check, then standard cudaMemcpyAsync proceeds as before.

## Test Plan

1. **No-regression test**: Build llama.cpp, run 35B model without VITRIOL_MODE set. Confirm tok/s unchanged from baseline (7.19 tok/s).

2. **Hook active test**: Run with `VITRIOL_MODE=1` and `VITRIOL_VERBOSE=1`. Verify log output shows VITRIOL messages during tensor loading (the hook's sync_copies counter increments).

3. **Benchmark**: Confirm no performance regression when VITRIOL_MODE is DISABLED (the common case) and when mode is SYNC but returns false (hook is called but falls through).

## Files Modified

| File | Lines | Change |
|------|-------|--------|
| `ggml-cuda.cu` | ~30 (includes) | Add `#include "vitriol-cuda-integration.h"` |
| `ggml-cuda.cu` | 682-683 | Add hook call before cudaMemcpyAsync in buffer_set_tensor |
| `ggml-cuda.cu` | 699-701 | Add hook call before cudaMemcpy2DAsync in buffer_set_tensor_2d |
| `ggml-cuda.cu` | 2992 | Add hook call before cudaMemcpyAsync in set_tensor_async |
