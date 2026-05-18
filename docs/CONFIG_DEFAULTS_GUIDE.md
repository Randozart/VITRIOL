# VITRIOL Configuration Guide — Recommended Defaults

## Hardware Baseline

The recommended defaults are tuned for:

| Component | Spec |
|-----------|------|
| GPU | NVIDIA GTX 1070 Ti (8 GB VRAM, Pascal CC 6.1) |
| RAM | 15 GB |
| Storage | NVMe SSD |
| Model | Qwen3.6-35B-A3B-UD-Q2_K_XL (MoE, 256 experts, 8 active/token) |

> **If your hardware differs**, see *When to Diverge* at the end of each section.

---

## 1. Operation Mode — `stream`

**Default:** `mode = stream`

### Why It Works

The model has 256 experts but only 8 are active per token. With 8 GB VRAM, we can't fit all 10 GB of expert weights on the GPU. Stream mode solves this:

```
NVMe → System RAM (10 GB VITRIOL buffer, page-locked via cudaHostRegister)
   → LRU VRAM cache (2 GB, hot experts cached)
   → GPU compute
```

- Expert weights live in page-locked system RAM
- Hot experts (frequently used) are cached in the 2 GB VRAM LRU pool
- Cold experts stream across PCIe on demand
- Non-expert layers (attention, norms) stay on GPU

**Measured throughput:** ~5.9 tok/s generation, ~66 tok/s eval (with all optimizations enabled)

### When to Diverge

| Scenario | Recommended Mode | Reason |
|----------|-----------------|--------|
| You have 24+ GB VRAM | `mode = off` | All weights fit on GPU, no need for stream mode |
| < 8 GB VRAM | `mode = async` | Lower memory overhead, slower but more stable |
| You only have 8-12 GB system RAM | Use `--disk-offload` | Avoids 10 GB page-locked buffer (see Disk Offload) |

---

## 2. Frozen Prompt — `on`

**Default:** `frozen_prompt = on`

### Why It Works

The KV cache for the system prompt is computed once and reused across all turns in a conversation. Without this, every new message reprocesses the entire system prompt, wasting ~400ms on re-prefixing.

**Measured impact:**

| Metric | Without Frozen | With Frozen | Improvement |
|--------|---------------|-------------|-------------|
| Eval speed | 43 tok/s | 66 tok/s | **+53%** |
| Gen speed | 5.63 tok/s | 5.89 tok/s | **+5%** |

This is the single highest-impact optimization in VITRIOL.

### How It Works

When `frozen_prompt = on`, the server:
1. Detects the system prompt (first message)
2. Caches its KV computation in GPU VRAM (~0.5-2 MB depending on prompt length)
3. On subsequent requests, appends new user messages to the cached KV instead of recomputing

### When to Diverge

| Scenario | Recommended | Reason |
|----------|-------------|--------|
| No system prompt used | `off` (no benefit) | No prompt to freeze |
| System prompt changes per request | `off` | Frozen KV would be stale |
| Extremely long system prompts (>8K tokens) | `off` | Frozen KV consumes too much VRAM |

---

## 3. Context Window — `500000`

**Default:** `context = 500000` (500K tokens)

### Why It Works

With Q4_0 KV cache quantization, each KV cell is 4 bits (0.5 bytes per element). The model architecture (40 layers, 2 KV heads, 256-dim) means each token consumes ~5.6 KB of KV cache.

| Context | KV Cache Size | Total VRAM | Headroom |
|---------|---------------|------------|----------|
| 254K | 1,396 MiB | 3,738 MiB | 4,374 MiB |
| 500K | ~2,800 MiB | ~5,100 MiB | ~3,000 MiB |
| 1,000K | ~5,600 MiB | ~8,000 MiB | ~100 MiB |

At 500K tokens, we use ~5.1 GB of VRAM (model + KV + compute), leaving 3 GB headroom for LRU cache and temporary buffers.

**VRAM budget breakdown at 500K context:**

```
GTX 1070 Ti (8,112 MiB):
  Model weights (GPU)          1,337 MiB
  KV cache (500K, Q4_0)        2,800 MiB
  Compute/RS buffers             556 MiB
  LRU VRAM cache               2,048 MiB
  ───────────────────────────────────
  Total                         6,741 MiB ← Headroom: 1,371 MiB
```

### When to Diverge

| Scenario | Recommended Context | Reason |
|----------|-------------------|--------|
| Short chat sessions | 32K-128K | Faster prefill, less VRAM used |
| Code generation | 8K-32K | Code rarely needs 500K context |
| Document analysis | 500K-1M | Need full document in context |
| < 8 GB VRAM | Reduce to 128K-254K | Fit within VRAM budget |

---

## 4. LRU Cache — `0` (disabled)

**Default:** `lru_mb = 0`

> **Diagnostic finding (2026-05-18):** The LRU VRAM cache is **unreachable** for quantized MoE models. See [`docs/LRU_DIAGNOSTIC_FINDING.md`](LRU_DIAGNOSTIC_FINDING.md) for full analysis.

### Why Disabled by Default

The LRU cache (`vitriol_lru_ensure`) is never called during inference with Q2_K_XL or any quantized MoE model. The `ggml_cuda_mul_mat_id` function has three early-return fast paths (MMQ/MMF/MMVQ) that bypass the LRU code entirely. For quantized models, the MMQ fast path always matches, and the function returns before reaching the LRU logic.

Expert weights are read directly from the page-locked VITRIOL buffer via GPU PCIe DMA — no VRAM caching occurs. The `lru_mb` setting has **zero effect** on performance or VRAM usage regardless of its value.

**Measured proof:**
| LRU Setting | VRAM Used (254k ctx) | Gen Speed |
|-------------|---------------------|-----------|
| 0 MB | 3,921 MiB | 5.87 tok/s |
| 1024 MB | 3,921 MiB | 5.72 tok/s |
| 2048 MB | 3,921 MiB | 5.87 tok/s |
| 4096 MB | 3,921 MiB | 5.71 tok/s |

Identical VRAM and performance across all values — confirming the LRU pool is never allocated.

### When to Enable

| Scenario | Recommended LRU | Reason |
|----------|----------------|--------|
| Q2_K_XL or any quantized MoE | **0** | LRU is unreachable; no effect |
| FP16/FP32 expert weights | 512-2048 | Slow path used; LRU caches hot experts |
| Unknown model type | 0 | Safe default |

---

## 5. KV Cache Quantization — `q4_0`

**Default:** `quant_mode = q4_0`

### Why It Works

Q4_0 compresses each KV cache element from 16 bits (FP16) to 4 bits — a 4× compression with minimal accuracy loss. For a model with 40 layers × 2 KV heads × 256-dim, this reduces per-token KV cost from ~80 KB to ~20 KB (measured: ~5.6 KB/token with additional optimizations).

| Quant | KV Size at 500K | Quality Impact |
|-------|-----------------|---------------|
| FP16 | ~11.2 GB | Full precision |
| Q8_0 | ~5.6 GB | Negligible loss |
| **Q4_0** | **~2.8 GB** | Minimal (KIVI research) |

### When to Diverge

| Scenario | Recommended Quant | Reason |
|----------|------------------|--------|
| Quality-critical work | `f16` | No quantization artifacts |
| Benchmarking perplexity | `f16` | Baseline comparison |
| < 500K context | `q8_0` | Higher quality, still fits VRAM |
| > 500K context | `q4_0` | Required to fit |

---

## 6. Thread Count — `4`

**Default:** `threads = 4`

### Why It Works

The GTX 1070 Ti has 8 CPU cores, but using all 8 for inference introduces thread contention on CUDA API calls, memory allocation, and expert routing synchronization.

**Measured impact:**

| Threads | Gen Speed | vs Baseline |
|---------|-----------|-------------|
| 4 | 5.87 tok/s | **Best** |
| 8 | 4.21 tok/s | -28% |

### When to Diverge

| Scenario | Recommended Threads | Reason |
|----------|-------------------|--------|
| CPU-bound model | Match core count | More threads help CPU-bound workloads |
| Batch inference | 4-8 | More threads for parallel batch processing |
| This GPU | 4 | Tuned for this specific setup |

---

## 7. Disk Offload — `off` (optional)

**Flag:** `--disk-offload`

### Why Available

Disk offload replaces the anonymous 10 GB VITRIOL buffer with a file-backed mmap of the GGUF file, reducing system RAM pressure. Pages are shared with the OS page cache and can be evicted under memory pressure.

**Measured impact:**

| Mode | Gen Speed | RAM Used |
|------|-----------|----------|
| Normal | 5.87 tok/s | 12-14 GB |
| Disk offload | 5.06 tok/s | 10-12 GB |

**Tradeoff:** -14% speed for -2 GB RAM savings. Only use if your system is memory-constrained.

### When to Enable

| Scenario | Recommended | Reason |
|----------|-------------|--------|
| 16+ GB RAM | Normal mode | Use the speed |
| 8-12 GB RAM | `--disk-offload` | Avoid OOM kills |
| Running other memory-heavy apps alongside | `--disk-offload` | Free RAM for other processes |

---

## Putting It All Together

### Recommended Command
```bash
vitriol serve --detach -c 500000 --kv-quant q4_0
```

### Config File (`~/.vitriol/config`)
```ini
[model]
context = 500000
threads = 4
ngl = 99

[vitriol]
mode = stream
# lru_mb = 2048  # only for FP16/FP32 expert weights; no effect on quantized models

[kv]
mode = offload
quant_mode = q4_0
frozen_prompt = on
```

### For Max Context (1M tokens)
```bash
vitriol serve --detach -c 1000000 --kv-quant q4_0
```
VRAM at 1M context: ~8.0 GB (tight but fits on 8 GB card).

---

*Benchmark methodology: Each test ran 3 inference passes after server warmup. Timings from `/v1/chat/completions` endpoint. Frozen prompt tested with system prompt "You are a helpful assistant." VRAM measured via `nvidia-smi`. Full benchmark data: `BENCHMARK_RESULTS.md`.*
