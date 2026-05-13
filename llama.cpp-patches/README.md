# llama.cpp Patches for VITRIOL

These files and patches integrate VITRIOL's expert streaming hooks into llama.cpp's CUDA backend.

## Files

| File | Purpose |
|------|---------|
| `ggml-cuda.cu.patch` | Adds `#include "vitriol-cuda-integration.h"` and wires `vitriol_cuda_set_tensor_hook()` before `cudaMemcpyAsync` at lines 682, 699, and 2992. Adds `vitriol_cuda_init()` call in `ggml_backend_cuda_init`. |
| `CMakeLists.txt.patch` | Changes source glob from `"*.cu"` to `"*.cu" "*.cpp"` so `vitriol-cuda-integration.cpp` is compiled. |
| `source/vitriol-cuda-integration.h` | Header declaring hook functions, config struct, and VITRIOL modes. |
| `source/vitriol-cuda-integration.cpp` | Implementation: state tracking, env var parsing, hook stubs that currently fall through to standard `cudaMemcpyAsync`. |

## Applying

```bash
cd /mnt/data/ai/llama.cpp

# 1. Copy the VITRIOL source files
cp /path/to/vitriol/llama.cpp-patches/source/vitriol-cuda-integration.* \
   ggml/src/ggml-cuda/

# 2. Apply the patches
patch -p1 < /path/to/vitriol/llama.cpp-patches/ggml-cuda.cu.patch
patch -p1 < /path/to/vitriol/llama.cpp-patches/CMakeLists.txt.patch

# 3. Rebuild
cd build && cmake .. -DGGML_CUDA=ON -DCMAKE_BUILD_TYPE=Release
make -j4 llama-server
```

## Hook Insertion Points

| Function | File:Line | When Called |
|----------|-----------|-------------|
| `ggml_backend_cuda_buffer_set_tensor` | `ggml-cuda.cu:682` | Model loading, once per tensor |
| `ggml_backend_cuda_buffer_set_tensor_2d` | `ggml-cuda.cu:699` | 2D tensor loading |
| `ggml_backend_cuda_set_tensor_async` | `ggml-cuda.cu:2992` | **Inference hot path**, every token |

## Environment Variables

| Var | Values | Effect |
|-----|--------|--------|
| `VITRIOL_MODE` | `disabled` (default), `sync`, `async`, `stream` | Sets VITRIOL operating mode |
| `VITRIOL_VERBOSE` | `0` (default), `1` | Enables VITRIOL debug logging |

When `VITRIOL_MODE` is `disabled` (default), the hook returns `false` immediately — zero overhead.

## Verification

```bash
# Check symbols are linked
nm -D /mnt/data/ai/llama.cpp/bin/libggml-cuda.so | grep vitriol

# Run with verbose logging
VITRIOL_MODE=sync VITRIOL_VERBOSE=1 ./bin/llama-server ...
```

Expected symbols: `g_vitriol_config`, `vitriol_cuda_init`, `vitriol_cuda_set_tensor_hook`, `vitriol_cuda_set_current_layer`, `vitriol_cuda_trigger_prefetch`, `vitriol_cuda_print_stats`.
