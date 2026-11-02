# VITRIOL Resource Locations

This document defines where all external resources, tools, and dependencies
live. All other documents and scripts in this project reference resources
using the environment variables defined here.

---

## Quick Reference

| Env Variable | Default | Description |
|---|---|---|
| `VITRIOL_HOME` | *(project root)* | Root of the VITRIOL project |
| `VITRIOL_ENGINE_DIR` | `./engine` | In-tree engine source + build tree |
| `VITRIOL_LLAMA_DIR` | `$VITRIOL_ENGINE_DIR` | Legacy alias (still honored by scripts) |
| `VITRIOL_LLAMA_BIN` | `$VITRIOL_ENGINE_DIR/build/bin` | Built engine executables |
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

### llama.cpp (in-tree engine, required)

| Resource | Default Path | Description |
|---|---|---|
| Source | `$VITRIOL_ENGINE_DIR` | In-tree fork, imported at `30b128504` (2026-10-23) |
| Build | `$VITRIOL_ENGINE_DIR/build` | CMake build directory (out-of-source) |
| Server binary | `$VITRIOL_LLAMA_BIN/vitriol-server` | The inference server (`llama-server` compat link) |
| CUDA backend | `$VITRIOL_LLAMA_BIN/libggml-cuda.so` | CUDA-accelerated GGML backend |
| VITRIOL hooks | `$VITRIOL_ENGINE_DIR/ggml/src/ggml-cuda/vitriol-cuda-integration.{h,cpp}` | Already applied at import |

**Sync policy:** frozen. See `docs/ENGINE_PROVENANCE.md`; no submodule and no
patch step. Build with `./scripts/build-llama-server.sh --backend cuda`.

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

### Development Machine (historical: i7-3770 + GTX 1070 Ti, 8GB)

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
| `VITRIOL_ENGINE_DIR` | `$VITRIOL_HOME/engine/` (in-tree) |
| `VITRIOL_ALKA_DIR` | `$VITRIOL_HOME/../alka-lang/` or system install |
| `CUDA_VISIBLE_DEVICES` | adjust for your GPU topology |

---

## Quick Start (Setting Up)

```bash
# 1. Set up the project (engine is in-tree; no submodules)
git clone https://github.com/Randozart/VITRIOL.git
cd VITRIOL

# 2. Configure resource locations
export VITRIOL_MODEL_DIR="/path/to/models"
export VITRIOL_ENGINE_DIR="$PWD/engine"

# 3. Build (VITRIOL patches are already in engine/)
CUDA_ARCHITECTURES=86 ./scripts/build-llama-server.sh --backend cuda

# 4. Run
CUDA_VISIBLE_DEVICES=0 ./engine/build/bin/vitriol-server \
    -m "$VITRIOL_MODEL_DIR/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf" \
    -ngl 20 -ot ".*exps.*=CPU" --port 8279 --no-mmap -c 4096
```
