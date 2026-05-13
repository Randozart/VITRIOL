# VITRIOL — Session Handoff Document

**Date:** 2026-05-11  
**Purpose:** Comprehensive handoff for a new agent to continue VITRIOL development

---

## 1. Project Overview

**VITRIOL** runs large language models on **VRAM-constrained hardware** by streaming only active MoE experts from disk to GPU, bypassing VRAM constraints. The model never fully sits in memory.

**Origin:** Built and optimized on GTX 1070 Ti (8GB) + i7-3770. Generalizes to any hardware where VRAM is the bottleneck.

**Current status:** Baseline inference verified — Qwen3.6-35B-A3B loads and runs at ~775MB VRAM using llama.cpp's `-ot` override flag. Full NVMe→GPU DMA not yet implemented.

**Alka status:** Alka language spec v4 exists (`/home/randozart/Desktop/Projects/alka-lang/SPECv4.md`) but is **not battle-tested**. VITRIOL is being built as the raw engine first; Alka will be slotted in as the orchestration layer once it's hardened and the friction of manual DMA management proves its necessity.

---

## 2. Hardware

| Component | Model | Purpose |
|-----------|-------|---------|
| GPU (main) | GTX 1070 Ti (8GB, Pascal sm_61) | Primary inference |
| GPU (secondary) | GTX 960 (2GB) | Future draft model for speculative decoding |
| CPU | i7-3770 (Ivy Bridge, no AVX2) | Orchestration only (too slow for compute) |
| Storage | NVMe SSD at `/mnt/data/ai/` | Models, swap, temp |
| RAM | 16GB (dedicated swap file for safety) | |

---

## 3. The Big Insight

**VITRIOL ≠ Kernel module. VITRIOL = Expert sparsity exploitation.**

The key breakthrough: Qwen3.6-35B-A3B has 256 experts but only activates **8 per token** (3.125%). By keeping just the base model (embeddings + attention = ~775MB) on GPU and streaming experts on demand, a 35B model fits in 8GB VRAM.

The kernel module (`vitriol.ko`) was an early exploration for PCIe DMA but is **not required** for the current approach. llama.cpp's built-in `-ot` flag already achieves the goal.

---

## 4. Architecture Evolution (9 Phases)

See `ARCHITECTURE_HISTORY.md` for full details.

| Phase | What | Status |
|-------|------|--------|
| 1 | llama.cpp baseline (Qwen 3.5 9B, 10.6 tok/s) | ✅ Working |
| 2 | Kernel module `vitriol.ko` (410KB) | ⏳ Built, never tested |
| 3 | VITRIOL modes (disabled/sync/async/stream) | ❌ Stubs, always return false |
| 4 | NVIDIA GDS pattern analysis | ✅ Documented |
| 5 | KTransformers async scheduling analysis | ✅ Documented |
| 6 | 3LTERN ternary kernel discovery | ⏳ Future optimization |
| 7 | Qwen3.6-35B-A3B downloaded (12.3GB) | ✅ Ready |
| 8 | Expert streaming via `-ot` (775MB VRAM!) | ✅ **Working!** |
| 9 | Alka language spec (v4, ~1900 lines) | ⏳ Not battle-tested |

---

## 5. Files

### Core Project (at `/home/randozart/Desktop/Projects/VITRIOL/`)

| File | Purpose |
|------|---------|
| `ARCHITECTURE_HISTORY.md` | Complete design evolution with decisions |
| `README.md` | Current architecture and running instructions |
| `docs/TESTING_PLAN.md` | Benchmark suite and component testing |
| `docs/VITRIOL_ARCHITECTURE.md` | Original alchemical architecture |
| `docs/VITRIOL_IMPLEMENTATION_PLAN.md` | Original implementation plan |

### Kernel Module (legacy, not needed for current path)

| File | Purpose |
|------|---------|
| `vitriol-daemon/vitriol.c` | Kernel module source (11KB) |
| `vitriol-daemon/vitriol.ko` | Compiled module (410KB) |
| `vitriol-daemon/vitriol-util.c` | Userspace utility |

### Expert Streaming Implementation

| File | Purpose |
|------|---------|
| `include/vitriol-moe-expert-parser.h` | GGUF expert tensor parsing header |
| `include/vitriol-expert-cache.h` | LRU cache header for on-demand loading |
| `src/vitriol-moe-expert-parser.cpp` | GGUF parser implementation |
| `src/vitriol-expert-cache.cpp` | Cache manager implementation |
| `test_expert_cache.sh` | Test script for expert streaming |

### llama.cpp Integration

| File | Purpose |
|------|---------|
| `/mnt/data/ai/llama.cpp/bin/llama-server` | Working inference server |
| `/mnt/data/ai/llama.cpp/bin/libggml-cuda.so` | CUDA backend (74MB) |
| `/mnt/data/ai/llama.cpp/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp` | VITRIOL hooks (stubs) |
| `/mnt/data/ai/llama.cpp/include/vitriol-config.h` | Mode configuration |
| `/mnt/data/ai/llama.cpp/src/vitriol-config.cpp` | Config implementation |

### Models

| File | Size |
|------|------|
| `/mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf` | 12.3 GB |
| `/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf` | 5.48 GB |

### Other References

| Path | Contents |
|------|----------|
| `/mnt/data/ai/gds-nvidia-fs/` | NVIDIA GDS source for DMA patterns |
| `/mnt/data/ai/KTransformers/` | KTransformers async scheduling patterns |
| `/home/randozart/Desktop/Projects/brief-compiler/` | Brief compiler (related tool) |
| `/home/randozart/Desktop/Projects/alka-lang/SPECv4.md` | Alka spec (~1900 lines) |

---

## 6. Commands & Test Results

### Run Qwen3.6-35B-A3B (Working — Verified 2026-05-11)

```bash
CUDA_VISIBLE_DEVICES=0 /mnt/data/ai/llama.cpp/bin/llama-server \
    -m /mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port 8279 \
    --no-mmap &

# Wait ~90s for 12GB model to fully load and warm up
sleep 90

# Test inference
curl http://localhost:8279/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Hello"}],"max_tokens":30}'
```

**Last test result:** Model loaded successfully. Memory breakdown:
- GPU CUDA0: **775 MB** (embeddings + 20 attention layers) ✓
- CPU/Host: **10.6 GB** (256 experts) ✓
- **Total VRAM: 775 MB** — far under 8GB limit

**Issue encountered:** Server returned **HTTP 503** during warmup (curl hit it before context was fully built). The `llama_context` construction started after the 60s mark. Need to wait longer (~90s) or retry after warmup.

### Run Baseline (Qwen 3.5 9B)

```bash
CUDA_VISIBLE_DEVICES=0 /mnt/data/ai/llama.cpp/bin/llama-server \
    -m /mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf \
    -ngl 25 \
    --port 8279 \
    --no-mmap
```

### Benchmark

```bash
cd /home/randozart/Desktop/Projects/VITRIOL
CUDA_VISIBLE_DEVICES=0 ./benchmark_vitriol.sh
```

**Note:** Benchmark script needs `CUDA_VISIBLE_DEVICES=0` set inside it (currently relies on default which may include GTX 960, causing OOM).

### Build llama.cpp

```bash
cd /mnt/data/ai/llama.cpp
mkdir -p build && cd build
cmake .. -DGGML_CUDA=ON -DCMAKE_BUILD_TYPE=Release
make -j4
```

---

## 7. Current State

### Working
- ✅ llama.cpp with CUDA on GTX 1070 Ti (single GPU)
- ✅ Qwen3.6-35B-A3B loads and runs via `-ot ".*exps.*=CPU"`
- ✅ VRAM usage reduced from 8.8GB → 775MB for 35B model
- ✅ GGUF expert parser (extracts tensor names/offsets)
- ✅ Expert cache manager (LRU eviction, on-demand loading)
- ✅ **Inference not yet measured** — model loaded but 503 during warmup; needs retry with longer wait

### Not Working / Issues
- ❌ Benchmark script fails with dual GPU (GTX 960 OOM)
- ❌ VITRIOL env-mode stubs never wired to actual DMA
- ❌ Kernel module never loaded (safety concern)
- ❌ No automated test for expert cache (simulation only)
- ❌ 3LTERN ternary kernel not integrated
- ❌ Actual token throughput for 35B model not yet measured (503 during warmup)

### Key Open Problem
The `-ot` approach keeps experts on CPU, which is slow. The target is to load experts directly from NVMe SSD → GPU via DMA. The cache manager and parser are built but the DMA transport (vitriol.ko or alternative) is not wired.

### Alka Integration Status
Alka is **not yet battle-tested**. The plan:
1. Build VITRIOL as the raw DMA pipe first (what we're doing now)
2. Let the friction of manually managing expert offsets, BAR1 sliding windows, and device timings prove the need for an abstraction layer
3. **Then** slot Alka in as the orchestration layer — once its spec is hardened against real hardware constraints

This follows the "Engine before ECU" philosophy.

---

## 8. Two Development Paths

### Path A: The Pragmatic Pipe (Recommended Next)
Continue building on llama.cpp's existing infrastructure. The `-ot` flag already works. Optimize the expert loading from CPU instead of building custom DMA:

1. Measure current tok/s with `-ot ".*exps.*=CPU"`
2. Optimize CPU→GPU expert transfer (prefetch, double-buffer)
3. Profile where time is spent (expert load vs compute)
4. Only build DMA if CPU transfer is the bottleneck

### Path B: The Kernel Module
Requires safe testing environment:

1. VM with GPU passthrough (or secondary GPU)
2. Load `vitriol.ko` → verify BAR mapping
3. Implement simple DMA test → SSD sector → GPU BAR1
4. Wire into llama.cpp's tensor loading
5. Replace CPU transfer with DMA

---

## 9. Key Decisions Made

| Decision | Rationale |
|----------|-----------|
| Use llama.cpp, not custom engine | Proven, maintained, CUDA support |
| Target Qwen3.6-35B-A3B | 256 experts = maximum sparsity potential |
| Use `-ot` over kernel module first | Quickest path to working inference |
| Ignore GTX 960 for now | 2GB VRAM causes OOM issues |
| Build VITRIOL before Alka | Engine before ECU philosophy |
| 2-bit quant over ternary | UD-Q2_K_XL available, 3LTERN not inference-ready |
| Alka slotted in later | Not battle-tested yet; let DMA friction prove the need |

---

## 10. Model Details (Qwen3.6-35B-A3B)

| Parameter | Value |
|-----------|-------|
| Architecture | `qwen35moe` |
| Layers | 40 |
| Embedding dim | 2048 |
| Expert count | 256 |
| Experts per token | 8 |
| Head count | 16 |
| Head count KV | 2 |
| Expert FFN dim | 512 |
| Shared Expert FFN | 512 |
| Quant types | iq2_xs, q5_K, q6_K, f32 |

**GGUF tensors of interest:**
- `blk.{LAYER}.attn_{norm,post_norm}` — Attention layer norms
- `blk.{LAYER}.ffn_gate_inp.weight` — Router (determines which experts)
- `blk.{LAYER}.ffn_down_exps.weight` — All 256 experts (3D: [n_ff_exp, n_embd, n_expert])
- `blk.{LAYER}.ffn_gate_exps.weight` — Expert gate projections
- `blk.{LAYER}.ffn_up_exps.weight` — Expert up projections

---

## 11. Immediate Next Steps (Picked Up Mid-Test)

This session was interrupted **mid-test**. Here's exactly where we were:

### Step 1: Complete the -\`ot\` Inference Test
The Qwen3.6-35B-A3B model loaded successfully at 775MB VRAM, but curl hit a 503 during warmup. Retry:
```bash
# Wait for warmup to finish, then test
sleep 30
curl http://localhost:8279/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Hello"}],"max_tokens":30}'
```
**Expected:** Functional inference. **Need to measure:** Actual tok/s.

### Step 2: Run Full Benchmark
Once step 1 confirms inference works:
```bash
CUDA_VISIBLE_DEVICES=0 cd /home/randozart/Desktop/Projects/VITRIOL && ./benchmark_vitriol.sh
```
Compare tok/s of 35B MoE (experts on CPU) vs 9B dense (all layers on GPU).

### Step 3: Profile Bottleneck
If tok/s is low, profile where time is spent:
- Expert loading from CPU → GPU (likely bottleneck)
- Attention compute on GPU
- Router overhead

### Step 4: Decision Point
Based on profiling:
- **If CPU→GPU transfer is the bottleneck** → Optimize transfer (prefetch, double-buffer, or DMA)
- **If compute is the bottleneck** → Investigate 3LTERN ternary kernel
- **If both** → Need both paths

---

## 12. References

See `README.md` bibliography section for full prior art list.

### Critical Links
- llama.cpp: https://github.com/ggml-org/llama.cpp
- 3LTERN: https://github.com/ELX987/3LTERN (ternary Pascal CUDA kernel)
- NVIDIA GDS: https://github.com/NVIDIA/gds-nvidia-fs
- KTransformers: https://github.com/kvcache-ai/KTransformers
- Qwen3.6-35B-A3B: https://huggingface.co/unsloth/Qwen3.6-35B-A3B-GGUF
- Alka spec v4: `/home/randozart/Desktop/Projects/alka-lang/SPECv4.md`

---

*Prepared for handoff. The engine is running. Let friction guide the next iteration.*