# VITRIOL Benchmark Results

## Hardware
- GPU: NVIDIA GeForce GTX 1070 Ti (8112 MiB VRAM)
- RAM: 15 GB
- CPU: 4 threads

## Model
- Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf (12 GB)
- Architecture: qwen35moe, 40 layers, 256 experts (8 active/token)
- VITRIOL mode: stream (page-locked RAM + LRU VRAM cache)

## KV Cache
- Quantization: Q4_0 (4-bit)
- Per token: ~20 KB (40 layers × 2 KV heads × 256 dim × 0.5 bytes)
- VRAM cost: ~2.5 GB at 128K, ~5.1 GB at 256K (measured: 1396 MiB at 254K — lower than estimate due to efficient format)

## Generation Speed Tests

### 100k Context, LRU=512 MB
| Length | Gen (tok/s) | Eval (tok/s) | Prefill (ms) |
|--------|------------|-------------|-------------|
| 10     | 6.19       | 37.51       | 453         |
| 50     | 5.69       | 45.49       | 374         |
| 100    | 5.64       | 44.48       | 382         |

### 150k Context, LRU=2048 MB
| Length | Gen (tok/s) | Eval (tok/s) |
|--------|------------|-------------|
| 10     | 6.19       | 43.90       |
| 50     | 5.70       | 45.97       |
| 100    | 5.63       | 45.53       |

### 150k Context, LRU=4096 MB
| Length | Gen (tok/s) | Eval (tok/s) |
|--------|------------|-------------|
| 50     | 5.69       | 43.67       |
| 100    | 5.63       | 43.11       |

### 150k Context, LRU=1024 MB
| Length | Gen (tok/s) | Eval (tok/s) | Prefill (ms) |
|--------|------------|-------------|-------------|
| 50     | 5.69       | 43.67       | 377         |
| 100    | 5.63       | 43.11       | 377         |

### 254k Context, LRU=1024 MB — MAXIMUM VERIFIED
| Length | Gen (tok/s) | Eval (tok/s) | Prefill (ms) |
|--------|------------|-------------|-------------|
| 10     | 6.20       | 44.57       | 381         |
| 50     | 5.70       | 45.13       | 377         |
| 100    | 5.62       | 45.12       | 377         |

### Frozen Prompt Enabled (500k ctx) — BEST CONFIG
| Config | Length | Gen (tok/s) | Eval (tok/s) | Prefill (ms) |
|--------|--------|------------|-------------|-------------|
| Frozen, LRU=1024, t=4 | 50 | **5.79** | **64.84** | 416 |
| Frozen, LRU=1024, t=4 | 100 | **5.72** | **62.76** | 430 |
| Frozen, LRU=2048, t=4 | 50 | **5.89** | **58.57** | 461 |
| Frozen, LRU=2048, t=4 | 100 | **5.87** | **66.29** | 407 |
| Frozen, LRU=4096, t=4 | 50 | 5.78 | 61.69 | 438 |
| Frozen, LRU=4096, t=4 | 100 | 5.71 | 65.21 | 414 |

> **Note:** LRU cache has zero effect on quantized models — see `docs/LRU_DIAGNOSTIC_FINDING.md`.

### Thread Tuning
| Config | Gen (tok/s) | Notes |
|--------|------------|-------|
| t=4 (baseline) | 5.63-5.70 | **Best** |
| t=8 | 4.14-4.21 | 25% worse — thread contention |

### VRAM Usage
- Model weights (GPU): 1337 MiB
- VITRIOL buffer (RAM): 10040 MiB
- KV cache (500k Q4_0): ~2800 MiB (on GPU, estimated)
- Compute/RS buffers: ~556 MiB
- Total VRAM used: ~3921 MiB (measured at 254k)
- Headroom: ~4191 MiB

## Conclusions
1. **Frozen prompt provides the biggest gain** — eval speed +50% (43→65 tok/s), gen +3% (5.63→5.79)
2. **LRU cache is unreachable** for quantized MoE models — all LRU settings gave identical VRAM and speed (see `docs/LRU_DIAGNOSTIC_FINDING.md`)
3. More threads (t=8) makes things **worse** — contention overhead
4. Q4_0 KV cache enables massive context (500k+ fits in ~4 GB VRAM)
5. VITRIOL stream mode is required for 35B MoE on 8 GB VRAM

## Conclusions
1. LRU size (512-4096 MB) does not significantly affect generation speed — bottleneck is GPU compute
2. Q4_0 KV cache enables large context (150k+ fits in ~3 GB)
3. VITRIOL stream mode is required for 35B MoE on 8 GB VRAM (experts in RAM, cached on GPU)
4. Token lookahead (prompt lookup decoding) not useful for general conversation
