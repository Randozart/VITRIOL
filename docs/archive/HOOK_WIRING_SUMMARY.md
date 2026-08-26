# VITRIOL Hook Wiring — 2026-05-13

## Summary
The existing VITRIOL CUDA integration hooks were successfully wired into llama.cpp's `ggml-cuda.cu`. Previously the hooks existed as stubs but were never compiled or called from any code path.

## Changes Made (llama.cpp source tree)

### Hooks Wired
Three insertion points in `/mnt/data/ai/llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu`:

| Location | Line | Function | When Called |
|----------|------|----------|-------------|
| `ggml_backend_cuda_buffer_set_tensor` | 682 | `vitriol_cuda_set_tensor_hook()` | Model loading, one-shot per tensor |
| `ggml_backend_cuda_buffer_set_tensor_2d` | 699 | `vitriol_cuda_set_tensor_hook()` | 2D tensor loading |
| `ggml_backend_cuda_set_tensor_async` | 2992 | `vitriol_cuda_set_tensor_hook()` | **Inference hot path**, every token |

### Build System
- `/mnt/data/ai/llama.cpp/ggml/src/ggml-cuda/CMakeLists.txt`: Changed source GLOB from `"*.cu"` to `"*.cu" "*.cpp"` to compile `vitriol-cuda-integration.cpp`

### Init & Env Vars
- `vitriol_cuda_init()` called from `ggml_backend_cuda_init()` (called once per CUDA device at startup)
- Reads `VITRIOL_MODE` (disabled/sync/async/stream) and `VITRIOL_VERBOSE` (0/1) from environment

### Behavior
- **Default (no env vars):** Hook returns false immediately, standard `cudaMemcpyAsync` runs. Zero overhead.
- **VITRIOL_MODE=sync:** Hook increments `sync_copies` counter, falls through to standard path.
- **VITRIOL_MODE=async:** Placeholder for future double-buffer prefetch.
- **VITRIOL_MODE=stream:** Placeholder for future DMA streaming.

### Verification
- Build: `make -j4 llama-server` succeeds, all VITRIOL symbols in `libggml-cuda.so`
- Init confirmed: `grep VITRIOL` in server log shows init messages
- Symbols verified: `nm -D libggml-cuda.so | grep vitriol` shows 6 symbols

## Next Steps
When Alka's DMA backend is ready, implement `vitriol_cuda_set_tensor_hook` for `VITRIOL_MODE_STREAM`:
1. Parse tensor name to extract GGUF file offset
2. Map NVMe file region to GPU BAR1
3. Issue DMA via FLOW instruction (Alka Metrod packet → /dev/vitriol IOCTL)
4. Return true to skip the standard `cudaMemcpyAsync`
