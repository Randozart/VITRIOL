# llama.cpp Patches for VITRIOL (RAM Shot)

These files and patches integrate VITRIOL's custom buffer type into llama.cpp's CUDA backend. Expert weights are placed in page-locked host system RAM, accessible by the GPU over PCIe DMA.

## Files

| File | Purpose |
|------|---------|
| `ggml-cuda.cu.patch` | Adds `#include "vitriol-cuda-integration.h"` + `#include "vitriol-buffer.h"`. Modifies `supports_buft` to accept VITRIOL buffer type. Adds `vitriol_cuda_init()` call in `ggml_backend_cuda_init`. |
| `llama-model-loader.patch` | Adds `#include <dlfcn.h>` and VITRIOL buffer type auto-apply via `dlsym` for expert tensors (name contains "exps"). |
| `CMakeLists.txt.patch` | Changes source glob from `"*.cu"` to `"*.cu" "*.cpp"` so VITRIOL `.cpp` files are compiled. |
| `source/vitriol-cuda-integration.h` | Header: config struct, mode enum, `vitriol_cuda_init()`, `vitriol_get_expert_buffer_type()`, `vitriol_is_stream_enabled()`. |
| `source/vitriol-cuda-integration.cpp` | Implementation: env var parsing, CE DMA channel init (stub for future LRU cache). |
| `source/vitriol-buffer.h` | Header: `vitriol_get_buffer_type()`, `vitriol_is_vitriol_buffer_type()`. |
| `source/vitriol-buffer.cpp` | VITRIOL buffer type: mmap → madvise(HUGEPAGE) → mlock → cudaHostRegister → is_host=true. |

## Architecture (RAM Shot)

```
VITRIOL_MODE=stream
  → vitriol_cuda_init() parses env, inits CE DMA (stub)
  → Model loader: expert tensors → VITRIOL buffer type (via dlsym hook)
  → VITRIOL alloc: mmap(10GB) → madvise(HUGEPAGE) → mlock → cudaHostRegister
  → set_tensor: memcpy from GGUF mmap → VITRIOL buffer
  → Scheduler: is_host=true → intelligent MoE offload → CUDA backend
  → MUL_MAT_ID: GPU reads weights over PCIe DMA
```

## Applying

```bash
cd /path/to/llama.cpp

# 1. Copy the VITRIOL source files
cp /path/to/vitriol/llama.cpp-patches/source/vitriol-*.{cpp,h} \
   ggml/src/ggml-cuda/

# 2. Apply the patches
patch -p1 < /path/to/vitriol/llama.cpp-patches/ggml-cuda.cu.patch
patch -p1 < /path/to/vitriol/llama.cpp-patches/llama-model-loader.patch
patch -p1 < /path/to/vitriol/llama.cpp-patches/CMakeLists.txt.patch

# 3. Build
cd build && cmake .. -DGGML_CUDA=ON -DCMAKE_BUILD_TYPE=Release
make -j$(nproc) llama-server

# 4. Grant capability for mlock + cudaHostRegister on 10GB
sudo setcap cap_ipc_lock=+ep ./bin/llama-server

# 5. Run
CUDA_VISIBLE_DEVICES=0 VITRIOL_MODE=stream ./bin/llama-server \
  -m model.gguf -ngl 41 -c 2048 --port 8279
```

## Environment Variables

| Var | Values | Effect |
|-----|--------|--------|
| `VITRIOL_MODE` | `disabled` (default), `stream` | Enables RAM Shot expert placement |
| `VITRIOL_VERBOSE` | `0` (default), `1` | Enables VITRIOL debug logging |
| `CUDA_VISIBLE_DEVICES` | (CUDA standard) | Restrict to specific GPU |

## Requirements

- **CUDA 12.0+** (tested with 12.0)
- **`CAP_IPC_LOCK`** capability for `mlock(10GB)` + `cudaHostRegister(10GB)`
- **GPU with CC 6.0+** (Pascal or newer, tested on GTX 1070 Ti CC 6.1)
