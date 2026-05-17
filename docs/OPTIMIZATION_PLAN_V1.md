# VITRIOL Optimization Plan

> *"Measure first, optimize second."*

## Current Baseline (2026-05-12)

| Model | Configuration | VRAM | tok/s | GPU Util | Per Token |
|-------|--------------|------|-------|----------|-----------|
| Qwen3.6-35B-A3B | -ngl 20, -ot ".*exps.*=CPU" | 2016 MB | 5.5-7.2 | 15-18% | 162-181ms |
| Qwen3.5-9B | -ngl 25 (full GPU) | 5320 MB | 8.3-9.8 | 25-35% | 113-120ms |

**Key insight:** GPU utilization is 15-18% during 35B inference. The GPU is idle 82-85% of the time waiting for CPU to feed expert data.

---

## Step 1: Profile Exact Time Breakdown ✅ COMPLETE

### Results
- **Expert loading**: ~100-120ms per token (60-70% of total time)
- **Attention compute**: ~30-40ms (20-25%)
- **Expert compute**: ~20-30ms (10-15%)
- **Router + overhead**: ~10ms (5%)

### Key Findings
1. First token overhead is massive (2.4s for 35B, 1.3s for 9B)
2. Generation speed degrades with context (6.16 t/s at 50 tokens → 5.53 t/s at 200 tokens)
3. GPU is severely underutilized (15-18% for 35B, 25-35% for 9B)
4. The 9B model is only 1.35-1.56x faster despite having all weights on GPU
5. **Data movement, not compute, is the bottleneck**

### Files
- `alka/results/profile_step1.md` — Full profiling data

---

## Step 2: Double-Buffer / Prefetch Optimization ⏳ PENDING

**Goal:** Hide expert loading latency behind GPU compute.

### Current behavior:
```
Token N:  [load experts] → [compute] → [done]
Token N+1: [load experts] → [compute] → [done]
```

### Target behavior:
```
Token N:  [load experts N] → [compute N] → [done]
Token N+1: [load experts N+1] → [compute N+1] → [done]
           ↑ happens during compute N
```

### Challenge
- llama.cpp builds the computation graph at initialization
- Tensor placement is fixed at load time
- MoE routing is dynamic per token (we don't know which experts are needed until after computing)
- Would require modifying llama.cpp's source code to implement async expert loading

### Potential Approaches
1. **Async CUDA streams**: Modify ggml to use separate streams for expert loading vs compute
2. **Pinned memory**: Use `cudaHostAlloc` for faster CPU→GPU transfers
3. **Layer-level prefetch**: While computing layer N, preload experts for layer N+1

### Expected Outcome
- tok/s: 7.2 → 10-12+ (if we can fully overlap loading with compute)
- GPU utilization: 15-18% → 30-50%

### Status
Blocked by need to modify llama.cpp source. Requires significant engineering effort.

---

## Step 3: Speculative Decoding with GTX 960 ⏳ BLOCKED

**Goal:** Use idle GTX 960 as draft model to accelerate inference.

### Architecture
```
GTX 960 (2GB)          GTX 1070 Ti (8GB)
─────────────          ─────────────────
Qwen 0.5B draft        Qwen3.6-35B-A3B
500MB → fits in VRAM   775MB base + expert swap
100+ tok/s             7.2 tok/s
     │                       ▲
     │   P2P DMA via PCIe    │
     │   (8 tokens guessed)  │
     └───────────────────────┘
```

### Blocker
No compatible draft model available locally. Need:
- Qwen2.5-0.5B or Qwen3-0.6B (same tokenizer family)
- Quantized to Q4_K_M or smaller (~1GB)
- Must fit in GTX 960's 2GB VRAM

### Expected Outcome
- Effective tok/s: 7.2 × (1 + 8 × acceptance_rate)
- If acceptance rate is 60%: 7.2 × 5.8 = **41.8 tok/s**
- If acceptance rate is 40%: 7.2 × 4.2 = **30.2 tok/s**

### Command (once model is available)
```bash
CUDA_VISIBLE_DEVICES=0,1 /mnt/data/ai/llama.cpp/bin/llama-server \
    -m /mnt/data/ai/koboldcpp/Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf \
    -ngl 20 -ot ".*exps.*=CPU" \
    --spec-draft-hf Qwen/Qwen2.5-0.5B-Instruct-GGUF:q4_k_m \
    --tensor-split 8192,0 --main-gpu 0 \
    --port 8279 --no-mmap --threads 4 -c 4096
```

### Files
- `alka/results/speculative_step3_plan.md` — Detailed plan

---

## Step 4: NVMe→GPU DMA Path ✅ PIPELINE COMPLETE

**Status:** Recipe generation pipeline working. Benchmark pending execution.

### What's Done
1. ✅ `vitriol.ko` v0.2 with Alka ABI (0xA1 magic) — 5 new IOCTLs, 11 opcode handlers
2. ✅ `gguf-offset-resolver` — Parses GGUF v3, extracts real tensor offsets (733 tensors)
3. ✅ `generate-alka-recipe.sh` — Generates `.alka` from GGUF + `.alkavl`
4. ✅ `alka-executor` — Validates and executes `.alkas` streams via `/dev/vitriol`
5. ✅ Alka compiler integration — Auto-compiles recipes to `.alkas` + `.azoth`
6. ✅ Generated streams: base (1,226 packets, 39KB), full (1,466 packets, 47KB)
7. ✅ Executor dry-run passes for both streams

### Generated Stream Stats
| Recipe | Packets | Binary | Total Data |
|--------|---------|--------|------------|
| Base (non-expert) | 1,226 | 39 KB | 81.2 GB |
| Full (all tensors) | 1,466 | 47 KB | 105.1 GB |

### Remaining
- [ ] Run `./scripts/benchmark_alka.sh` (requires sudo, ~15 min)
- [ ] Replace staged FLOW with direct NVMe→GPU DMA (`blkdev_direct_read()`)
- [ ] Interrupt-driven FENCE (NVMe completion queue vs polling)
- [ ] GPU kernel launch via SIGNAL

### Files
- `FINDINGS_2026-05-13-alka-benchmark.md` — Full pipeline documentation
- `docs/ALKA_EXECUTOR_DESIGN.md` — Executor + ABI design
- `vitriol-daemon/vitriol.c` — Kernel module v0.2
- `alka-executor/executor.c` — Userspace executor
- `alka-executor/gguf-offset-resolver.c` — GGUF parser
- `scripts/generate-alka-recipe.sh` — Dynamic recipe generator
- `scripts/benchmark_alka.sh` — Benchmark pipeline

---

## Summary & Recommendations

### What We Know
1. **Expert loading is 60-70% of per-token time** — this is the bottleneck to attack
2. **GPU is idle 82-85% of the time** — massive headroom for optimization
3. **9B is only 1.35-1.56x faster** — confirms data movement is the constraint, not compute
4. **PCIe 3.0 x16 theoretical: 12 GB/s** — DMA could reduce expert loading from ~100ms to ~20ms

### Recommended Order
1. **Run benchmark first** — `./scripts/benchmark_alka.sh` establishes Alka baseline
2. **Step 3** (speculative decoding) — highest potential impact (4-6x speedup)
3. **Step 2** (prefetch) — moderate impact (1.5-2x speedup)
4. **Step 4 DMA optimization** — replace staged copy with direct NVMe→GPU

### Quick Win
Download a Qwen2.5-0.5B draft model and test speculative decoding. This could push us from 7.2 tok/s to 30-40+ tok/s with minimal engineering effort.
