# VITRIOL Architecture History

**Purpose:** Record the evolution of VITRIOL's architecture — what worked, what didn't, and the decisions that shaped it.

---

## Architecture Evolution Overview

```
Phase 1: Llama.cpp Baseline     → Worked (10.6 tok/s, Qwen 3.5 9B)
Phase 2: Kernel Module ($vitriol.ko$) → Built, never tested (too dangerous)
Phase 3: VITRIOL Modes (env flags)  → Stubs, always fell back to cudaMemcpy
Phase 4: GDS Analysis             → Patterns learned, no code
Phase 5: KTransformers Analysis   → Patterns learned, no code
Phase 6: 3LTERN Discovery         → Ternary CUDA kernel, training-focused
Phase 7: Qwen3.6-35B-A3B         → Target model identified
Phase 8: Expert Streaming (-ot)   → WORKS! 775MB VRAM for 35B model
Phase 9: Alka Language            → Spec v4, not battle-tested yet
```

---

## Detailed History

### Phase 1: Baseline (Working)
**2026-05-10**

Built llama.cpp with CUDA support. Ran Qwen 3.5 9B Q4_K_M at 10.6 tok/s with 25 GPU layers on GTX 1070 Ti.

| Metric | Value |
|--------|-------|
| Model | Qwen 3.5 9B Q4_K_M |
| Tokens/sec | ~10.6 |
| VRAM usage | 3974 MiB (model) + 192 MiB (KV) + 565 MiB (compute) |
| GPU layers | 25 |

**Decision:** Use llama.cpp as base engine (proven, maintained, CUDA support).

---

### Phase 2: Kernel Module (Built, Not Tested)
**2026-05-10**

Created `vitriol.ko` — a kernel module with:
- PCI probe for GTX 1070 Ti (10de:1b82)
- BAR 0 mapping (16MB control plane)
- BAR 1 mapping (256MB data plane)
- DMA buffer allocation
- Character device (`/dev/vitriol`)
- IOCTL interface

**Status:** Compiled successfully (410KB). Never loaded on target system.

| | Details |
|--|---------|
| **Why it was built** | Needed kernel-level access for PCIe P2P DMA |
| **Why it wasn't tested** | Too dangerous — loading custom PCI module could crash system |
| **Verdict** | Good foundation, needs safe testing environment |

---

### Phase 3: VITRIOL Modes (Stubs)
**2026-05-10**

Created flag-based mode system with environment variables:

| Mode | Implementation | Status |
|------|---------------|--------|
| `disabled` | Standard cudaMemcpy | ✅ Works (baseline) |
| `sync` | Stub → returns false | ❌ Falls back to baseline |
| `async` | Stub → returns false | ❌ Falls back to baseline |
| `stream` | Stub → returns false | ❌ Falls back to baseline |

**Decision:** All VITRIOL modes return `false` — they never replace cudaMemcpy.

**Why stubs?** No DMA infrastructure to back them yet. The scaffolding was built ahead of the engine.

---

### Phase 4: NVIDIA GDS Analysis
**2026-05-10**

Cloned and studied `github.com/NVIDIA/gds-nvidia-fs`. Key files analyzed:
- `nvfs-core.c` — kiocb completion callbacks
- `nvfs-pci.c` — PCIe DMA setup
- `nvfs-dma.c` — DMA transfer logic

**Key findings:**
- Metapage (4KB shared page) for fast completion signaling
- `wmb()` memory barrier before triggering DMA
- kiocb callbacks for NVMe completion

**Decision:** Use metapage pattern for future DMA completions. Don't copy GDS code — use the patterns.

---

### Phase 5: KTransformers Analysis
**2026-05-10**

Cloned and studied `github.com/kvcache-ai/KTransformers`. Key patterns:
- YAML-based layer placement across CPU/GPU
- Double-buffer prefetch: compute layer N while streaming N+1
- MoE-specific async scheduling

**Decision:** Use double-buffer pattern for future async expert loading. KTransformers targets modern CPUs (AMX/AVX512) — VITRIOL bypasses CPU entirely.

---

### Phase 6: 3LTERN Discovery
**2026-05-11**

Found `github.com/ELX987/3LTERN` — a custom CUDA kernel for W1.58A8 (1.58-bit ternary) inference on Pascal GPUs.

| Aspect | Detail |
|--------|--------|
| Ternary encoding | 16 weights per uint32 (00=0, 01=+1, 10=-1, 11=0) |
| Key instruction | `__dp4a` — available on Pascal (sm_61) |
| No Tensor Cores needed | Standard CUDA cores handle ternary add/sub |
| Training-focused | Not inference-ready, but kernel can be extracted |

**Decision:** Keep in pocket for future compute optimization. Don't integrate yet — focus on expert streaming first.

---

### Phase 7: Qwen3.6-35B-A3B (Target Model)
**2026-05-11**

**The "why bother with ternary" moment.** Downloaded Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf (12.3GB) from Unsloth.

| Metric | Value |
|--------|-------|
| Total params | 34.66 B |
| Active params | ~3B per token (A3B) |
| Expert count | 256 |
| Experts used | 8 per token |
| Active ratio | 8/256 = 3.125% |
| Quantization | UD-Q2_K_XL (2-bit) |
| File size | 11.44 GiB (2.83 BPW) |

**Why this is the target:**
- 3B active params = fast even on Pascal
- 256 experts = massive sparsity for SSD streaming
- 2-bit quant = tiny expert footprints
- Ungated download from Unsloth (no HF token needed)

**Decision:** This is THE model for VITRIOL. The sparsity (3.125% active) makes expert streaming viable.

---

### Phase 8: Expert Streaming (Working!)
**2026-05-11**

**The breakthrough.** Used llama.cpp's existing `-ot` (override tensor) flag to keep experts on CPU:

```bash
./llama-server -ngl 20 -ot ".*exps.*=CPU"
```

| Metric | Before (full load) | After (-ot exps=CPU) |
|--------|-------------------|---------------------|
| GPU VRAM | 8.8 GB | 775 MB |
| CPU/Host | — | 10.6 GB |
| **Total** | **FAIL (OOM)** | **WORKS** |

**Why this works:** The experts are stored as a single 3D tensor `[n_ff_exp, n_embd, n_expert]`. The `-ot` flag overrides placement so expert tensors go to CPU/Host while non-expert tensors (embeddings, attention) go to GPU.

**Decision:** This is the VITRIOL architecture — base model on GPU, experts on demand from CPU/SSD.

---

### Phase 9: Alka Language (Spec Only)
**2026-05-11**

Alka v4 spec exists (~1900 lines) but hasn't been battle-tested. The core insight:

> VITRIOL is the Engine. Alka is the ECU.
> - VITRIOL = raw DMA power
> - Alka = abstraction for safety, repeatability, and multi-device orchestration

**Decision:** Build VITRIOL first (dumb C pipe). Let the friction of managing DMA prove why Alka is needed.

---

## Key Decisions Summary

| # | Decision | Date | Status |
|---|----------|------|--------|
| 1 | Use llama.cpp (not custom) | 2026-05-10 | ✅ Working |
| 2 | Build kernel module for DMA | 2026-05-10 | ⏳ Untested |
| 3 | Mode system via env vars | 2026-05-10 | ❌ Stubs |
| 4 | Learn from NVIDIA GDS patterns | 2026-05-10 | ✅ Documented |
| 5 | Learn from KTransformers patterns | 2026-05-10 | ✅ Documented |
| 6 | Discover 3LTERN for Pascal compute | 2026-05-11 | ⏳ Future |
| 7 | Target Qwen3.6-35B-A3B | 2026-05-11 | ✅ Downloaded |
| 8 | Expert streaming via -ot flag | 2026-05-11 | ✅ Working |
| 9 | VITRIOL first, Alka later | 2026-05-11 | ✅ Decided |

---

## What Worked

| Approach | Status | Why |
|----------|--------|-----|
| llama.cpp + CUDA on 1070 Ti | ✅ Working | Standard inference pipeline |
| Qwen3.6-35B-A3B-UD-Q2_K_XL | ✅ Working | 2-bit MoE fits in constraints |
| Expert streaming via -ot | ✅ Working | Base on GPU, experts on CPU |
| 3LTERN CUDA kernel idea | ⏳ Pending | Pascal-compatible, not wired yet |

## What Didn't Work

| Approach | Problem | Lesson |
|----------|---------|--------|
| Full model load on 8GB GPU | OOM at 8.8GB | Must use sparsity |
| Two GPUs (1070 Ti + 960) | GTX 960 OOM (2GB) | Don't mix GPUs |
| VITRIOL mode stubs | Never wired to actual DMA | Build pipe first, then controls |
| VITRIOL kernel module loading | Too dangerous | Need safe test environment |
| Benchmark script on dual GPU | Memory calculations wrong | Isolate to single GPU |

---

## Current Architecture (v0.4)

```
┌─────────────────────────────────────────────────────────────┐
│ llama.cpp (orchestration)                                    │
│  - Tokenization, KV cache, attention                         │
│  - Expert router (gate_inp) → picks 8/256 experts            │
├─────────────────────────────────────────────────────────────┤
│ GPU (GTX 1070 Ti, 775MB)                                    │
│  - Embeddings + attention layers                             │
│  - 20/41 layers offloaded                                    │
├─────────────────────────────────────────────────────────────┤
│ CPU/Host (10.6GB)                                            │
│  - 256 experts (3 per layer: gate/up/down)                    │
│  - Loaded from GGUF on demand                                │
├─────────────────────────────────────────────────────────────┤
│ DISK (12.3GB GGUF file)                                      │
│  - Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf                          │
└─────────────────────────────────────────────────────────────┘
```

### Next Architecture (v1.0 - Target)

```
┌─────────────────────────────────────────────────────────────┐
│ Alka (orchestration)                                         │
│  - FLOW instruction for expert loading                       │
│  - SHIFT instruction for BAR1 sliding window                  │
├─────────────────────────────────────────────────────────────┤
│ llama.cpp (inference core)                                   │
│  - Tokenization, KV cache, computation                        │
├─────────────────────────────────────────────────────────────┤
│ VITRIOL DMA (vitriol.ko)                                     │
│  - NVMe → GPU BAR1 direct P2P DMA                            │
│  - Metapage completion signaling                              │
├─────────────────────────────────────────────────────────────┤
│ GPU (GTX 1070 Ti, 8GB)                                      │
│  - Embeddings + attention (always)                            │
│  - 8 active experts (swapped on demand)                       │
├─────────────────────────────────────────────────────────────┤
│ SSD (NVMe)                                                   │
│  - All 256 experts (loaded one expert at a time)             │
└─────────────────────────────────────────────────────────────┘
```

---

*VITRIOL v0.4 | 2026-05-11 | Engine built, waiting for ECU*