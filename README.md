# VITRIOL

<img src="vitriol_logo.svg" alt="VITRIOL" width="200"/>

*"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*

(Visit the Interior of the Earth, by Rectifying you will find the Hidden Stone)

## What It Is

VITRIOL runs large language models on **VRAM-constrained hardware** by streaming only the active MoE experts from disk to GPU, instead of loading the full model. The model never fully sits in memory.

**Current approach:** llama.cpp's `-ot` flag keeps 256 experts on CPU while embedding + attention layers run on GPU. This fits a 35B model in 775MB of VRAM.

**Target approach:** Direct NVMe→GPU DMA via PCIe P2P (`vitriol.ko` kernel module), orchestrated by the Alka language, for sub-millisecond expert swapping.

**Origin:** Built and optimized on a GTX 1070 Ti (Pascal, 8GB) + i7-3770 (Ivy Bridge). The architecture generalizes to any hardware where VRAM is the bottleneck.

---

## Current Architecture

```
Model: Qwen3.6-35B-A3B (256 experts, 8 active/token, 2-bit quant)
         │
         ▼
┌─────────────────────┐       ┌─────────────────────┐
│     GPU (775MB)     │       │   CPU/Host (10.6GB)   │
│  Embeddings + Attn  │       │   All 256 experts     │
│  20/41 layers       │       │   (on-demand loading) │
└─────────────────────┘       └─────────────────────┘
```

**Key constraint:** GPU VRAM is 8GB. Full model is 8.8GB. Solution: don't load all 256 experts — only 8 are active per token.

---

## Target Architecture

```
┌───────────────────────────────────────────────────────────────┐
│   Alka (Orchestration)                                        │
│   FLOW = load expert, SHIFT = slide BAR1 window               │
├───────────────────────────────────────────────┬───────────────┤
│   llama.cpp (Inference)                       │ VITRIOL (DMA) │
│   Tokenization, KV cache, attention compute  │ vitriol.ko    │
│                                                │ NVMe→GPU P2P  │
├───────────────────────┬───────────────────────┴───────────────┤
│     GPU (8GB)         │      SSD (NVMe)                       │
│  Base model (always)  │   256 experts (1 at a time)            │
│  Active experts (swapped)│  12GB GGUF file                     │
└───────────────────────┴───────────────────────────────────────┘
```

---

## Hardware (Development Platform)

| Component | Model |
|-----------|-------|
| GPU (primary) | GTX 1070 Ti (Pascal, 8GB, device 1b82) |
| CPU | i7-3770 (Ivy Bridge, no AVX2) |
| GPU (secondary) | GTX 960 (2GB) — future draft model / speculative decoding |
| Storage | NVMe SSD |

## Model: Qwen3.6-35B-A3B

| Metric | Value |
|--------|-------|
| Architecture | MoE, 256 experts, 8 active/token |
| Total params | 34.66 B |
| Active per token | ~3B (8/256 experts) |
| Quantization | UD-Q2_K_XL (2-bit) |
| File size | 11.44 GiB |

---

## Running It

```bash
# Embeddings + attention on GPU, 256 experts on CPU
CUDA_VISIBLE_DEVICES=0 /mnt/data/ai/llama.cpp/bin/llama-server \
    -m /mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf \
    -ngl 20 \
    -ot ".*exps.*=CPU" \
    --port 8279

  curl http://localhost:8279/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"messages":[{"role":"user","content":"Hello"}],"max_tokens":30}'
```

---

## Project Structure

```
├── ARCHITECTURE_HISTORY.md    # Design evolution, decisions, dead ends
├── docs/
│   ├── TESTING_PLAN.md        # Benchmarks, component tests
│   ├── VITRIOL_ARCHITECTURE.md
│   └── VITRIOL_IMPLEMENTATION_PLAN.md
├── vitriol-daemon/
│   └── vitriol.ko             # Kernel module (P2P DMA, untested)
├── include/
│   ├── vitriol-moe-expert-parser.h   # Expert tensor parsing
│   └── vitriol-expert-cache.h        # LRU cache for expert loading
└── src/
    ├── vitriol-moe-expert-parser.cpp
    └── vitriol-expert-cache.cpp
```

---

## Ars Priori & Acknowledgements

VITRIOL stands on the shoulders of giants. Every core insight — DMA over PCIe, metapage completion signaling, async expert prefetching, extreme quantization on legacy hardware — was reverse-engineered from the following works. We document our debt explicitly.

### Inference Engine

| Project | What We Learned |
|---------|-----------------|
| **[llama.cpp](https://github.com/ggml-org/llama.cpp)** (ggml-org) | The core inference engine. GGUF format, CUDA backend, tensor loading pipeline. The `-ot` (override tensor) flag in PR #11397 was the breakthrough that enabled expert streaming. Our `vitriol-cuda-integration.cpp` hooks into `ggml-cuda.cu` at the tensor-copy boundary. |
| **[GGUF Format](https://github.com/ggerganov/llama.cpp/blob/master/ggml/include/gguf.h)** | Binary model format with tensor offsets accessible via `gguf_get_tensor_offset()`, `gguf_get_tensor_name()`, `gguf_get_tensor_type()` — the foundation of our expert parser. |
| **[PR #11397](https://github.com/ggerganov/llama.cpp/pull/11397)** (slaren) | Added `--override-tensor` (`-ot`) for per-tensor-type buffer placement. The exact mechanism we use: `-ot ".*exps.*=CPU"` keeps 8GB of experts on CPU while attention layers run on GPU. |
| **[PR #11571](https://github.com/ggerganov/llama.cpp/pull/11571)** (fairydreaming) | Load-all-experts-during-warmup; `llama_set_warmup()` API for ensuring all expert tensors are resident before inference. |
| **[PR #6387](https://github.com/ggerganov/llama.cpp/pull/6387)** (slaren) | Changed expert storage from per-expert tensors to a single 3D tensor — critical for our approach since all 256 experts are now in one contiguous block. |

### GPUDirect Storage & DMA

| Project | What We Learned |
|---------|-----------------|
| **[gds-nvidia-fs](https://github.com/NVIDIA/gds-nvidia-fs)** (NVIDIA) | Official GPUDirect Storage source code. We studied `nvfs-core.c`, `nvfs-pci.c`, and `nvfs-dma.c` to understand: kiocb completion callbacks for NVMe, shared metapage (4KB) for fast completion signaling, `wmb()` memory barriers before DMA. |
| **[open-gpu-kernel-modules](https://github.com/NVIDIA/open-gpu-kernel-modules)** (NVIDIA) | NVIDIA's open kernel module source for PCIe register-level operations — reference for understanding BAR mapping and GPU PCI config space. |
| **[hw-nvdla](https://github.com/NVIDIA/hw-nvdla)** (NVIDIA) | Hardware DLA documentation for understanding direct memory access patterns on NVIDIA silicon. |

### Async Scheduling & MoE Orchestration

| Project | What We Learned |
|---------|-----------------|
| **[KTransformers](https://github.com/kvcache-ai/KTransformers)** (kvcache-ai) | YAML-based layer placement across CPU/GPU, double-buffer prefetch pattern (compute layer N while streaming N+1), MoE-specific async scheduling. KTransformers targets modern CPUs (AMX/AVX512); VITRIOL inverts this — GPU as primary compute, CPU as orchestrator only. |
| **Qwen3.6-35B-A3B MoE** | 256 experts, 8 active per token — the exact sparsity architecture that makes expert streaming viable. The MoE router (`ffn_gate_inp`) determines which 8 experts to load; only those need to be in VRAM. |

### Extreme Quantization & Compute

| Project | What We Learned |
|---------|-----------------|
| **[3LTERN](https://github.com/ELX987/3LTERN)** (ELX987) | W1.58A8 (1.58-bit ternary) CUDA kernel for Pascal. 16 weights packed per uint32, branchless decode via `bit0 - bit1`, `__dp4a` instruction on sm_61. Future optimization path for compute-bound layers. |
| **[Unsloth](https://huggingface.co/unsloth)** (Daniel & Michael) | Dynamic quantization formats (UD-Q2_K_XL) that are structurally superior to raw 1.58-bit. Ungated model distribution — their Qwen 3.6 releases don't require HF authentication. The model we target was quantized and distributed by them. |

See `ARCHITECTURE_HISTORY.md` for the complete evolution.

---

## Philosophy

**VITRIOL = Engine.** Raw DMA power. Hard-coded, dangerous, fast.

**Alka = ECU.** Abstraction layer. Safe, repeatable, multi-device.

Build the engine first. Let the friction prove why you need the computer.

---

*"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*