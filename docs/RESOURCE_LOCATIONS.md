# VITRIOL Resource Locations

This document defines where all external resources, tools, and dependencies
live. All other documents and scripts in this project reference resources
using the environment variables defined here.

---

## Quick Reference

| Env Variable | Default | Description |
|---|---|---|
| `VITRIOL_HOME` | *(project root)* | Root of the VITRIOL project |
| `VITRIOL_LLAMA_DIR` | `./llama.cpp` | llama.cpp source + build tree |
| `VITRIOL_LLAMA_BIN` | `$VITRIOL_LLAMA_DIR/build/bin` | Built llama.cpp executables |
| `VITRIOL_MODEL_DIR` | *(see below)* | Directory containing model .gguf files |
| `VITRIOL_DATA_DIR` | `$VITRIOL_HOME/data` | Large data files (models, swap, temp) |
| `VITRIOL_ALKA_DIR` | *(external)* | Alka language compiler project root |
| `VITRIOL_ALKA_BIN` | `$VITRIOL_ALKA_DIR/zig-out/bin/alka` | Alka compiler binary |
| `VITRIOL_TMP_DIR` | `$VITRIOL_DATA_DIR/tmp` | Temporary files during inference |
| `VITRIOL_SWAP_DIR` | `$VITRIOL_DATA_DIR/swap` | Swap files for memory safety |

---

## Models

| Resource | Default Path | Description |
|---|---|---|
| **Qwen3.6-35B-A3B** (target) | `$VITRIOL_MODEL_DIR/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf` | 35B MoE model, 256 experts, 12.3 GB |
| **Qwen3.5-9B** (baseline) | `$VITRIOL_MODEL_DIR/Qwen_Qwen3.5-9B-Q4_K_M.gguf` | 9B dense model for comparison, 5.5 GB |
| **Qwen3-0.6B** (draft) | `$VITRIOL_MODEL_DIR/draft/Qwen3-0.6B-Q4_K_M.gguf` | Small draft model for speculative decoding, 379 MB |

---

## Third-Party Projects

### llama.cpp (required)

| Resource | Default Path | Description |
|---|---|---|
| Source | `$VITRIOL_LLAMA_DIR` | git submodule at `https://github.com/ggerganov/llama.cpp.git` |
| Build | `$VITRIOL_LLAMA_DIR/build` | CMake build directory |
| Server binary | `$VITRIOL_LLAMA_BIN/llama-server` | The inference server |
| CUDA backend | `$VITRIOL_LLAMA_BIN/libggml-cuda.so` | CUDA-accelerated GGML backend |
| VITRIOL hooks | `$VITRIOL_LLAMA_DIR/ggml/src/ggml-cuda/vitriol-cuda-integration.{h,cpp}` | Our patched integration files |

**Commit pinned:** `1e5ad35d5` (b9090 + 3 commits)
**To apply VITRIOL patches:** `./scripts/apply-llama-patches.sh`

### Alka Language Compiler (required for DMA path, optional for benchmarking)

| Resource | Default Path | Description |
|---|---|---|
| Source | `$VITRIOL_ALKA_DIR` | https://github.com/anomalyco/alka-lang |
| Binary | `$VITRIOL_ALKA_BIN` | Compiled Zig binary |
| Spec v4 | `$VITRIOL_ALKA_DIR/SPECv4.md` | Language specification |

### Reference Projects (documented only, not required)

| Project | Path | Purpose |
|---|---|---|
| NVIDIA GDS | `$VITRIOL_DATA_DIR/gds-nvidia-fs/` | GPUDirect Storage pattern analysis |
| KTransformers | `$VITRIOL_DATA_DIR/KTransformers/` | Async scheduling pattern analysis |
| Brief Compiler | `$VITRIOL_HOME/../brief-compiler/` | Kernel bindings (legacy) |
| 3LTERN | *(external)* | Ternary CUDA kernel for Pascal GPUs |

---

## VITRIOL Patches

| Resource | Path |
|---|---|
| Patch: ggml-cuda.cu | `llama.cpp-patches/ggml-cuda.cu.patch` |
| Patch: CMakeLists.txt | `llama.cpp-patches/CMakeLists.txt.patch` |
| VITRIOL header | `llama.cpp-patches/source/vitriol-cuda-integration.h` |
| VITRIOL source | `llama.cpp-patches/source/vitriol-cuda-integration.cpp` |

---

## Defaults by Platform

### Development Machine (GTX 1070 Ti, 8GB)

| Variable | Value |
|---|---|
| `VITRIOL_MODEL_DIR` | `/mnt/data/ai/koboldcpp/` |
| `VITRIOL_DATA_DIR` | `/mnt/data/ai/` |
| `VITRIOL_LLAMA_DIR` | `/mnt/data/ai/llama.cpp/` |
| `VITRIOL_ALKA_DIR` | `~/Desktop/Projects/alka-lang/` |
| `VITRIOL_SWAP_DIR` | `/mnt/data/ai/swap/` |
| `CUDA_VISIBLE_DEVICES` | `0` (only GTX 1070 Ti, not GTX 960) |

### Generic / CI / Other Hardware

| Variable | Suggested Value |
|---|---|
| `VITRIOL_MODEL_DIR` | `$VITRIOL_DATA_DIR/models/` |
| `VITRIOL_LLAMA_DIR` | `$VITRIOL_HOME/llama.cpp/` (submodule) |
| `VITRIOL_ALKA_DIR` | `$VITRIOL_HOME/../alka-lang/` or system install |
| `CUDA_VISIBLE_DEVICES` | adjust for your GPU topology |

---

## Quick Start (Setting Up)

```bash
# 1. Set up the project
git clone https://github.com/anomalyco/VITRIOL
cd VITRIOL
git submodule update --init

# 2. Configure resource locations
export VITRIOL_MODEL_DIR="/path/to/models"
export VITRIOL_LLAMA_DIR="$PWD/llama.cpp"

# 3. Apply VITRIOL patches to llama.cpp
./scripts/apply-llama-patches.sh

# 4. Build
./scripts/build-llama-server.sh

# 5. Run
CUDA_VISIBLE_DEVICES=0 ./llama.cpp/build/bin/llama-server \
    -m "$VITRIOL_MODEL_DIR/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf" \
    -ngl 20 -ot ".*exps.*=CPU" --port 8279 --no-mmap -c 4096
```
