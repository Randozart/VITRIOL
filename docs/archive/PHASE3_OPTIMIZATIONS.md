# Phase 3 Optimizations — Prior Art & Implementation Plan

## Sprint 1: Built-in llama.cpp Flags

### 1. KV Cache Quantization (`--kv-quant TYPE`)

**Prior art:**
- **vLLM PagedAttention** (2023) — Kwon et al. Block-level KV cache management
- **KIVI** (2024) — Liu et al. 2-bit KV cache quantization
- **llama.cpp** `--cache-type-k` flag — Built-in since 2024

**How it works:** The KV cache typically lives at FP16 (16-bit). By quantizing K and V to Q4_0 or Q8_0, the same VRAM footprint holds 2-4x the tokens. For a 10-layer Qwen3.6 model, the KV cache shrinks from 480 MiB to ~120 MiB at Q4_0, pushing the effective context from 24K to ~96K tokens.

**Integration:** Already supported by `llama-server` — just needs the flag plumbed through `scripts/vitriol`.

### 2. Prompt Lookup Decoding (`--lookup N`)

**Prior art:**
- **Prompt Lookup Decoding** (2024) — Apoorv Umang. N-gram matching in existing context.
- **LLM Accelerator** (2024) — Reuses common response patterns.

**How it works:** If a string of tokens appeared earlier in the prompt (e.g., a function name, variable declaration, import statement), the model reuses those tokens as a "draft" and verifies them in one parallel pass. For coding tasks with repetitive patterns, this provides ~1.5-2x speedup with zero model modification.

**Integration:** Already supported by `llama-server` — just needs the flag plumbed through.

### 3. Engine Mode (`--engine-mode MODE`)

**Prior art:**
- **llama.cpp** native mode — Standard execution, all tensors in VRAM if possible
- **VITRIOL** vitriol-dma mode — Current RAM Shot + LRU cache behavior

**How it works:** Switches between VITRIOL's custom buffer type (page-locked host RAM, LRU VRAM cache) and standard llama.cpp native buffers. `native` mode is for users with >24 GB VRAM who don't need offloading.

---

## Sprint 2: Speculative Decoding

**Prior art:**
- **Speculative Sampling** (2022) — Leviathan et al. (Google). Proved verification is parallelizable.
- **Speculative Sampling** (2023) — Chen et al. (DeepMind). Rejection sampling math.
- **Medusa** (2024) — Cai et al. Multiple decoding heads on a single model.
- **EAGLE** (2024) — Li et al. Feature-vector speculation, SOTA for self-speculation.

See `docs/OPTIMIZATION_PLAN.md` §1.8 for the full speculative decoding plan.

---

## Sprint 3: Top-1 Expert Self-Speculation

**Prior art:**
- **Self-Speculative Decoding** (2023) — Layer skipping for draft generation.
- **Mixture of Speculative Experts** (2024) — Top-1 expert draft for MoE models.

See `docs/vitriol-cuda-integration.cpp` — hooks into `vitriol_predictor_prefetch()` draft mode.

---

## Sprint 4: Fiddler CPU Activation Offload

**Prior art:**
- **Fiddler** (2024) — Kamahori et al. CPU-GPU orchestration for MoE.
- **T-MAC** (2024) — Microsoft. LUT-based CPU inference for low-bit models.
- **BitNet b1.58** (2024) — Ma et al. Ternary weights eliminating multiplication.

Deferred until TQ1_0 GGUF models are available.

---

*Last updated: 2026-05-17*
