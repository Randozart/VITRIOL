# VITRIOL Testing & Optimization Plan

**Last Updated:** 2026-05-11  
**Purpose:** Systematic benchmarking and optimization of the VITRIOL pipeline

---

## Executive Summary

This plan defines a systematic approach to benchmark and optimize the VITRIOL pipeline. The strategy is **measure-first** — establish baselines for each component, then progressively integrate for maximum efficiency.

**Core Philosophy:**
1. Test conventional methods first (baseline)
2. Then use Alka and wire everything together for max efficiency
3. Each component tested in isolation before integration

---

## Phase 1: Baseline Testing (Current State)

### 1.1 llama.cpp Baseline Performance

| Metric | Current Value | Target |
|--------|--------------|--------|
| Model | Qwen 3.5 9B Q4_K_M | Qwen 3.6 (ternary) |
| Token throughput | ~10.6 tok/s | TBD |
| GPU layers | 25 | TBD |
| VRAM usage | 3974 MiB (model) + 192 MiB (KV) + 565 MiB (compute) | ~5GB total |

**Test scripts available:**
- `benchmark_vitriol.sh` - tests VITRIOL modes
- `test_vitriol_baseline.sh` - baseline tests

**Actions needed:**
1. Run `benchmark_vitriol.sh` to confirm current performance
2. Record exact token throughput for baseline comparison

**Run command:**
```bash
cd /home/randozart/Desktop/Projects/VITRIOL
./benchmark_vitriol.sh
```

---

## Phase 2: Component Isolation Testing

### 2.1 VITRIOL Kernel Module (`vitriol.ko`)

| Test | Method | Success Criteria |
|------|--------|-------------------|
| Module loads | `sudo insmod vitriol.ko` | No errors in `dmesg` |
| PCI probe | `dmesg \| grep vitriol` | GTX 1070 Ti detected (10de:1b82) |
| BAR mapping | `./vitriol-util bar1` | 256MB window mapped |
| Character device | `ls -l /dev/vitriol` | Device exists |
| IOCTL interface | `./vitriol-util status` | Returns device info |

**Location:** `vitriol-daemon/vitriol.ko`

**Risks:** Loading kernel module may cause instability on current system. Consider VM or secondary GPU for safety.

**Module build:**
```bash
cd vitriol-daemon && make
```

**Load test:**
```bash
sudo insmod vitriol.ko
dmesg | tail
```

**Utility test:**
```bash
sudo ./vitriol-util status
sudo ./vitriol-util bar1
```

### 2.2 3LTERN CUDA Kernel Standalone

| Test | Method | Success Criteria |
|------|--------|-------------------|
| Compiles on Pascal | `nvcc -arch=sm_61` | No errors |
| Forward pass | Run test kernel | Valid output |
| Performance | Benchmark `el_bitlinear_forward_async()` | Compare vs standard GEMM |

**Source:** https://github.com/ELX987/3LTERN

**Key files:**
- `EL_ternCUDA_kernel.cu` - Main CUDA kernel (~1900 lines)
- `EL_ternCUDA_kernel.h` - Header file
- `pretrain_ternary_llm.py` - Training/quantization script

**Why Pascal works:**
- `__dp4a` instruction available on sm_61 (Pascal)
- No Tensor Cores needed - ternary = add/subtract only
- 16 weights packed into 1 uint32

**Ternary encoding:**
```
00 ->  0
01 -> +1
10 -> -1
11 ->  0 / reserved

Branchless decode: q = bit0 - bit1
```

**Compile test:**
```bash
git clone https://github.com/ELX987/3LTERN.git
cd 3LTERN
nvcc -arch=sm_61 -c EL_ternCUDA_kernel.cu -o el_tern.o
```

**Benchmark expected improvement:**
- 60% smaller than Q4 (1.58 bits vs 4 bits)
- Faster compute (no dequantization, no multiplication)
- Expected: 2-3x throughput improvement on Pascal

### 2.3 Alka Language/Compiler

| Test | Method | Success Criteria |
|------|--------|-------------------|
| Compiler exists | Check `/home/randozart/Desktop/Projects/alka-lang/` | Binary present |
| Spec compiles | Compile SPECv4.md | Valid output |
| Vial system | Test `.alkavl` parsing | Valid config |

**Location:** `/home/randozart/Desktop/Projects/alka-lang/SPECv4.md`

**Status:** SPECv4.md exists (~1900 lines), implementation status unclear.

**Key concepts:**
- `.alka` - Recipe files (high-level intent)
- `.alkavl` - Vial files (hardware config)
- `.alkas` - Compiled binary (Metrod format, 32-byte packets)
- `.azoth` - Rollback binary (safety)

---

## Phase 3: Integration Testing

### 3.1 VITRIOL Modes in llama.cpp

Current status: **Stubs only** — all modes return `false` and fall back to `cudaMemcpy`.

| Mode | Implementation Status | Next Step |
|------|----------------------|-----------|
| disabled | Works (baseline) | Baseline measurement |
| sync | Stub (no optimization) | Wire to kernel module |
| async | Stub (KTransformers pattern) | Implement double-buffer |
| stream | Stub (on-demand) | Implement NVMe→GPU DMA |

**Files to modify:**
- `/mnt/data/ai/llama.cpp/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`
- Replace `return false` with actual kernel module IOCTL calls

**Key functions:**
```cpp
// Current stub - always returns false
bool vitriol_cuda_set_tensor_hook(...) {
    // ...
    return false;  // <-- Always falls back to cudaMemcpy
}

// What needs to be done:
bool vitriol_cuda_set_tensor_hook(...) {
    // 1. Open /dev/vitriol
    // 2. IOCTL to request DMA transfer
    // 3. Wait for completion (metapage pattern from NVIDIA GDS)
    // 4. Return true to skip cudaMemcpy
}
```

### 3.2 Integration with 3LTERN

| Integration Point | File | Action |
|------------------|------|--------|
| Replace GEMM | `ggml-cuda.cu` | Swap `cublasGemm` → `el_bitlinear_forward_async()` |
| Quantization | 3LTERN packing | Use `el_pack_ternary_weights_async()` |
| Layer integration | VITRIOL hooks | Map MoE experts to 3LTERN weights |

**3LTERN API entry point:**
```cuda
extern "C" cudaError_t el_bitlinear_forward_async(
    const __half* X,           // Input activations (FP16)
    const uint32_t* W_packed,  // Ternary weights (packed)
    const float* W_scale,      // Per-row scales
    __half* Y,                 // Output
    int M, int N, int K,       // Matrix dimensions
    cudaStream_t stream
);
```

---

## Phase 4: Full Pipeline Testing

### 4.1 MoE Expert Streaming

**Architecture:**
```
Qwen 3.6 MoE (16 experts, 2-3 active per token)
         │
         ▼
┌────────────────┐     ┌──────────────┐
│ MoE Router     │────>│ Active Expert│
│ (predicts top-k)│     │   IDs (2-3)  │
└────────────────┘     └──────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │ VITRIOL (DMA)  │
                    │ Load expert N   │
                    │ from SSD → GPU  │
                    └─────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │ 3LTERN Kernel  │
                    │ W1.58A8 compute │
                    └─────────────────┘
```

| Test | Method | Success Criteria |
|------|--------|-------------------|
| Expert detection | Monitor MoE router | 2-3 experts active per token |
| Expert loading | Trace SSD→GPU | Correct expert loaded on demand |
| VRAM usage | Monitor nvidia-smi | Only active experts in VRAM |

**Expected VRAM savings:**
- 16 experts total
- 2-3 active at any time
- 80%+ VRAM reduction vs loading all experts

### 4.2 Speculative Decoding (Two-GPU)

**Architecture:**
```
[Draft GPU - Smaller/Faster] ──tokens──> [GTX 1070 Ti (Verifier)] ──output──> User
         │                                            │
         │                                            │
    Generate                                   Verify &
    candidates                                accept/reject
```

| Component | Spec Requirements |
|-----------|------------------|
| Draft GPU | Smaller/faster than 1070 Ti (e.g., GTX 1650, integrated) |
| Protocol | Standard speculative decoding (EAG, Medusa, etc.) |
| Integration | Alka orchestrates data transfer via PCIe P2P |

**Draft GPU options:**
- GTX 1650 (TU117) - 4GB, faster than 1070 Ti for small models
- Integrated graphics (CPU) - lowest cost, slowest
- Secondary 1070 Ti - expensive but consistent

**Alka orchestration:**
```alka
REQUIRE gpu_main.alkavl;
REQUIRE gpu_draft.alkavl;

// Draft generates candidate tokens
CLAIM gpu_draft;
FLOW model_draft[layer0] -> gpu_draft.VRAM[0];

// Main verifies
CLAIM gpu_main;
FENCE gpu_main.input_ready == 1;

// Verify and accept/reject
REFRACT gpu_draft.output -> gpu_main.input;
```

---

## Testing Matrix

| Component | Test Name | Script | Metrics |
|-----------|-----------|--------|---------|
| Baseline | llama.cpp current | benchmark_vitriol.sh | tok/s |
| Kernel | vitriol.ko load | manual | dmesg |
| Kernel | BAR mapping | vitriol-util | MB mapped |
| Kernel | IOCTL | vitriol-util status | latency |
| Kernel | DMA transfer | custom benchmark | throughput (GB/s) |
| 3LTERN | Compile for Pascal | nvcc test | compile success |
| 3LTERN | Forward pass | kernel benchmark | tok/s vs cuBLAS |
| Integration | VITRIOL modes | benchmark_vitriol.sh | tok/s each mode |
| Integration | 3LTERN+llama.cpp | custom | tok/s |
| Full pipeline | MoE expert swap | nvidia-smi + trace | VRAM, latency |
| Full pipeline | Speculative decoding | benchmark | tok/s |

---

## Model Acquisition

### Target: Qwen3.6-35B-A3B (The God-Mode Target)

**This is the optimal model for VITRIOL + Alka architecture.**

| Metric | Value | VITRIOL Benefit |
|--------|-------|-----------------|
| Total Parameters | 35B | Massive knowledge |
| Activated Parameters | 3B/token | 1070 Ti can handle 3B easily |
| Active Ratio | 3B/35B = 8.6% | 91.4% of model can stay on SSD |
| Architecture | MoE (16+ experts) | Expert swapping via FLOW |
| Quantization | UD-Q2_K_XL (2-bit) | Tiny PCIe footprint |

### Why Qwen3.6-35B-A3B is Perfect for This Stack

1. **A3B = Activate 3 Billion** - The 1070 Ti only computes 3B params/token
   - 3B on Pascal = blisteringly fast
   - No need for Tensor Cores (3LTERN can handle this too)

2. **MoE = Modular Experts** - The "Base" attention fits in VRAM permanently
   - Only 2-3 experts loaded per token from SSD
   - Perfect use case for Alka `FLOW` instruction

3. **2-bit Quant = Tiny Footprint** - UD-Q2_K_XL compresses 35B to ~9GB
   - PCIe bus barely breaks a sweat
   - Expert swapping is lightning fast

### Download Command

```bash
# From Unsloth (ungated, no auth needed)
hf download unsloth/Qwen3.6-35B-A3B-GGUF Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf --local-dir /mnt/data/ai/koboldcpp/
```

### Alternative Options

| Model | Size | Active | Best For |
|-------|------|--------|----------|
| Qwen3.6-35B-A3B | 35B | 3B | **TARGET** - Full MoE test |
| Qwen3.6-14B | 14B | 14B | Simpler, fits in VRAM |
| Qwen3.5-9B | 9B | 9B | Current baseline |
| Qwen3-72B-MoE | 72B | ? | Larger MoE |

**Quantization preferences:**
1. UD-Q2_K_XL (2-bit) - Unsloth's Dynamic Quant, optimal
2. Q3_K - Native GGUF 3-bit
3. W1.58 (ternary) - 3LTERN native

---

## Recommended Test Sequence

### Week 1: Baseline
- [ ] Run `benchmark_vitriol.sh`
- [ ] Record Q4 baseline tok/s
- [ ] Document current VRAM usage

### Week 2: Kernel Module
- [ ] Test `vitriol.ko` on safe system (VM/secondary GPU)
- [ ] Verify BAR1 mapping
- [ ] Measure kernel module latency

### Week 3: 3LTERN Standalone
- [ ] Clone 3LTERN
- [ ] Compile for sm_61 (Pascal)
- [ ] Benchmark forward pass
- [ ] Compare vs cuBLAS GEMM

### Week 4: Integration Alpha
- [ ] Wire VITRIOL modes to actual DMA
- [ ] Replace GEMM with 3LTERN in llama.cpp
- [ ] Test Q4 vs ternary on same model

### Week 5: Full Pipeline
- [ ] Test MoE expert swapping
- [ ] Measure VRAM savings
- [ ] Benchmark token throughput

### Week 6: Speculative Decoding
- [ ] Set up second GPU as draft
- [ ] Implement verification protocol
- [ ] Final benchmark

---

## Key Files Reference

| Path | Description |
|------|-------------|
| `/mnt/data/ai/llama.cpp/bin/llama-server` | CUDA inference server |
| `/mnt/data/ai/llama.cpp/bin/libggml-cuda.so` | CUDA backend (74MB) |
| `/mnt/data/ai/koboldcpp/Qwen_Qwen3.5-9B-Q4_K_M.gguf` | Current model (5.5GB) |
| `vitriol-daemon/vitriol.ko` | Kernel module (410KB) |
| `vitriol-daemon/vitriol-util` | Userspace utility |
| `benchmark_vitriol.sh` | Mode comparison script |
| `/mnt/data/ai/llama.cpp/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp` | CUDA hooks |
| `/mnt/data/ai/llama.cpp/include/vitriol-config.h` | Mode configuration |
| `docs/VITRIOL_ARCHITECTURE.md` | Architecture docs |
| `docs/TESTING_PLAN.md` | This file |

---

## Questions Before Implementation

1. **Safety first**: Should we test vitriol.ko on current system, or wait for VM/secondary GPU setup?

2. **3LTERN standalone**: Should we first compile and test 3LTERN kernel in isolation before integrating with llama.cpp?

3. **Model acquisition**: Any specific Qwen 3.6 variant you're targeting (dense vs MoE, size)?

4. **Speculative decoding hardware**: What is the second GPU model? This determines the draft/verify protocol.

---

## IMPLEMENTATION: VITRIOL Expert Streaming for MoE

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                    VITRIOL Expert Streaming Architecture                    │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  GGUF File: /mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf          │
│  ├── Embeddings (layer 0)     → GPU VRAM (always)   ~50MB                   │
│  ├── Attention (layers 1-40)  → GPU VRAM (always)   ~500MB                  │
│  └── Experts (256 total)      → SSD (on-demand)     ~8GB                    │
│       ├── expert_0.weight     → offset 0x...                                  │
│       ├── expert_1.weight     → offset 0x...                                  │
│       ...                                                                     │
│       └── expert_255.weight   → offset 0x...                                  │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  GGUF Parsing                                                                │
│  └── Extract: expert_count=256, expert_used_count=8, tensor offsets         │
│                                                                              │
├─────────────────────────────────────────────────────────────────────────────┤
│  Runtime                                                                    │
│  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐                     │
│  │   Router    │ -> │   Expert     │ -> │   Compute   │                     │
│  │  Predicts   │    │    DMA       │    │   3LTERN    │                     │
│  │  experts:   │    │  Load from   │    │  Inference  │                     │
│  │  [7,12,42]  │    │    SSD       │    │             │                     │
│  └─────────────┘    └─────────────┘    └─────────────┘                     │
│       │                   │                   │                            │
│       v                   v                   v                            │
│  ┌─────────────────────────────────────────────────────────────────────┐     │
│  │                        VRAM Budget                                  │     │
│  │  Embeddings: 50MB | Attention: 500MB | 8 Experts: ~500MB | KV: 200MB│     │
│  │  Total: ~1.25GB (fits easily in 8GB!)                                │     │
│  └─────────────────────────────────────────────────────────────────────┘     │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Implementation Steps - COMPLETED

| Step | Task | Status | Files |
|------|------|--------|-------|
| 1 | Parse GGUF to extract expert tensor offsets | ✅ DONE | `vitriol-moe-expert-parser.h/cpp` |
| 2 | Identify expert vs attention tensor types | ✅ DONE | Uses llama.cpp patterns: ffn_gate_exps, ffn_up_exps, ffn_down_exps |
| 3 | Implement expert cache with LRU | ✅ DONE | `vitriol-expert-cache.h/cpp` |
| 4 | Wire up VITRIOL DMA for SSD→GPU | 🔄 IN PROGRESS | Uses existing -ot override system |
| 5 | Test with Qwen3.6-35B-A3B | ⏳ PENDING | `test_expert_cache.sh` |

### Implementation Files Created

```
include/
├── vitriol-moe-expert-parser.h    # GGUF parsing for expert tensors
└── vitriol-expert-cache.h         # LRU cache for expert loading

src/
├── vitriol-moe-expert-parser.cpp  # Implementation
└── vitriol-expert-cache.cpp       # Cache manager

test_expert_cache.sh                # Test script
```

### Immediate Test: Use llama.cpp's existing expert override

```bash
# Keep experts on CPU, rest on GPU - reduces VRAM by ~8GB!
./llama-server -m Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port 8279
```

This uses the existing llama.cpp `-ot` (override tensor) feature to keep expert tensors on CPU while offloading other layers to GPU.

### Memory Analysis

| Component | Traditional (full load) | VITRIOL (expert streaming) |
|-----------|------------------------|---------------------------|
| Embeddings | 50MB | 50MB (always GPU) |
| Attention | 500MB | 500MB (always GPU) |
| Experts (256) | 8.3GB | 500MB (8 at a time) |
| KV Cache | 200MB | 200MB |
| **Total VRAM** | **9GB+ (FAIL)** | **~1.25GB (SUCCESS)** |

### Key GGUF Tensor Patterns for Experts

```
# MoE layer structure in GGUF:
model.layers.{layer}.mlp.gate.weight           # Router
model.layers.{layer}.mlp.experts.0.weight     # Expert 0
model.layers.{layer}.mlp.experts.1.weight     # Expert 1
...
model.layers.{layer}.mlp.experts.255.weight   # Expert 255

# Pattern to identify experts (from llama.cpp src/llama-model.cpp):
- ffn_gate_exps  - gate weights for experts
- ffn_up_exps    - up projection for experts  
- ffn_down_exps  - down projection for experts

# GGUF API already provides tensor offset access!
gguf_get_tensor_offset(ctx, tensor_idx)  // Get file offset
gguf_get_tensor_name(ctx, tensor_idx)     // Get tensor name
gguf_get_tensor_type(ctx, tensor_idx)     // Get tensor type
```

### PRIOR ART: llama.cpp Already Has This!

**PR #11397** added `--override-tensor` (`-ot`) to control where tensors go:
```bash
# Keep experts on CPU, rest on GPU
llama-server -ngl 99 -ot exps=CPU

# More specific: keep expert gate/up on CPU, down on GPU
llama-server -ot ffn_gate_exps=CPU -ot ffn_up_exps=CPU -ot ffn_down_exps=CUDA
```

This means we can:
1. Use existing override system but with a **custom buffer type** (SSD)
2. Or add lazy-loading to the existing tensor loading pipeline

### Expert Loading Strategy

1. **Initialization**: Load embeddings + attention (1GB total) to GPU
2. **First token**: Router predicts top-8 experts → Load those from SSD
3. **Subsequent tokens**: 
   - Check if needed experts already in VRAM cache
   - If not, evict LRU expert → Load new from SSD
4. **BAR1 Window**: 256MB can hold ~8 experts (each ~30MB compressed)

---

## NEW: Optimized Architecture for Qwen3.6-35B-A3B

### The Complete Pipeline

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        USER REQUEST                                         │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Stage 1: TOKENIZATION (CPU - i7-3770)                                       │
│  - llama.cpp tokenization                                                    │
│  - Minimal overhead, single-thread OK                                       │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Stage 2: MoE ROUTER (GPU - 1070 Ti)                                        │
│  - Small "Base" model (always in VRAM)                                      │
│  - Predicts which 2-3 experts needed                                        │
│  - Output: Expert IDs [7, 12, 3]                                            │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                    ┌────────────┴────────────┐
                    │  Alka Orchestration    │
                    │  0x03_FLOW instruction │
                    │  "Blast Expert 7 from  │
                    │   SSD offset X to     │
                    │   BAR1[0]"             │
                    └────────────┬────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Stage 3: EXPERT LOADING (VITRIOL DMA)                                       │
│  - Load expert N from NVMe SSD                                               │
│  - Direct GPU VRAM via PCIe P2P                                              │
│  - No CPU involvement                                                        │
│  - Target: 256MB BAR1 window                                                │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Stage 4: INFERENCE (3LTERN Kernel)                                         │
│  - el_bitlinear_forward_async()                                             │
│  - W1.58A8 or W2A8 compute                                                   │
│  - __dp4a on Pascal cores (fast!)                                           │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│  Stage 5: KV CACHE UPDATE (GPU VRAM)                                        │
│  - Store attention keys/values                                              │
│  - Sliding window (4096 tokens)                                              │
└────────────────────────────────┬────────────────────────────────────────────┘
                                 │
                                 ▼
                              OUTPUT
```

### VRAM Budget Analysis

| Component | Size (Qwen3.6-35B-A3B UD-Q2_K) | Status |
|-----------|-------------------------------|--------|
| Base attention model | ~500 MB | Always in VRAM |
| Active experts (2-3) | ~1-2 GB | Loaded on demand |
| KV cache | ~512 MB | Sliding window |
| CUDA runtime | ~200 MB | Fixed |
| **Total VRAM** | **~2.5 GB** | **1070 Ti has 8GB = 5.5GB headroom!** |

### The Optimization Hierarchy

| Priority | Optimization | Expected Improvement |
|----------|--------------|---------------------|
| 1 | MoE expert loading (VITRIOL) | 10x (91% of model on SSD) |
| 2 | 2-bit quantization (UD-Q2_K) | 2x (vs Q4) |
| 3 | 3LTERN compute (optional) | 1.5x (vs standard GEMM) |
| 4 | Speculative decoding (2-GPU) | 2x (token throughput) |

### Alka Recipe for Qwen3.6-35B-A3B

```alka
REQUIRE athanor_1070ti.alkavl;

// 1. Load base attention (always in VRAM)
CLAIM GPU_MAIN;
SHIFT GPU_MAIN.BASE_PLANE @ 0;

// 2. Main inference loop
FOR each_token {
    // Router predicts expert IDs
    ROUTE GPU_MAIN.ROUTER -> expert_ids[3];
    
    // Load each needed expert from SSD
    FOR expert_id IN expert_ids {
        FLOW NVME_BOOT[expert_offset(expert_id)] 
            -> GPU_MAIN.BAR1[expert_slot] 256MB;
    }
    
    // Execute inference with 3LTERN
    DISTILL GPU_MAIN.BAR1[0] 
            VIA 3LTERN_KERNEL 
            -> GPU_MAIN.OUTPUT;
    
    // Update KV cache
    UPDATE KV_CACHE GPU_MAIN.KV;
}
```

### Speculative Decoding Addition (Draft GPU)

```
[Draft GPU: Qwen3-0.6B] ──3-5 tokens──> [Main GPU: Qwen3.6-35B-A3B]
       │                                        │
       │  Fast generation                      Verify
       │  (small model)                        accept/reject
       │                                        │
       └────────────────────────────────────────┘
                          │
                   Final output
```

**Draft GPU candidates:**
- GTX 1650 (4GB) - Good balance
- Intel UHD 630 (i7-3770) - Free, but slower
- Secondary GTX 1070 Ti - Most compatible

---

## Summary: The VITRIOL Unique Value

| Layer | Technology | Benefit |
|-------|-----------|---------|
| **Logistics** | VITRIOL + Alka | SSD→GPU DMA without CPU |
| **Ballistics** | 3LTERN W1.58A8 | Ternary compute on Pascal (no Tensor Cores) |
| **Sparsity** | MoE expert loading (Qwen3.6-35B-A3B) | Only 2-3/16 experts in VRAM |
| **Efficiency** | Speculative decoding | 2-3x token throughput |

**The combination of all four creates the most efficient legacy hardware AI pipeline on the planet.**

---

*Status: Plan documented. Target model identified: Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf*