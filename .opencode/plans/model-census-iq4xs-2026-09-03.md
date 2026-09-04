# Model census + IQ4_XS calibration — 2026-09-03

Owner request: review new ~/Downloads models → "which quant at how much
context". Deletions first (owner-directed, disk space): 9B pair
(Q8_0 9.2G, Q6_K 7.1G) + UD-Q3_K_XL (13G) removed. Remaining: 27B quads
(Q3_K_M, UD-IQ4_XS, UD-Q4_K_S) + Mellum2-12B-A2.5B MoE.

## Hardware envelope

RTX 3060 12G (sm_86) + GTX 1070 Ti 8G (sm_61) = 20,480 MiB raw,
~15.9 GiB usable after CUDA overhead. Split ts 26,10.

## Calibrator census (GGUF-tensor-derived, vitriol-calibrate)

| model | file | weights | arch |
|---|---|---|---|
| Qwen3.8-27B-Q3_K_M | 13G | 9,350 MiB | qwen35, 65L, dense |
| Qwen3.8-27B-UD-IQ4_XS | 14G | 13,804 MiB | qwen35, 65L, dense |
| Qwen3.8-27B-UD-Q4_K_S | 15G | 14,620 MiB | qwen35, 65L, dense |
| Mellum2-12B-A2.5B-Q5_K_M | 8.6G | 7,340 MiB | mellum, 28L, 64-expert MoE |

Q4_K_S estimated 19,238 MiB total at ctx 8192 (93.9% of usable) —
no headroom for depth on this pair; not benched further.

## IQ4_XS pin sweep (sweep_controller, port 8290)

ctx 32768, ubatch 128, ts 26,10, tq3_0/tq3_0 KV, MTP 0, stream mode.
1 warmup + 3 measured 64-token rounds per config, mid-rounds dropped.

| pin | t/s |
|---|---|
| 0 | 14.90 |
| 4 | 14.85 |
| 8 | 14.91 |
| 12 | 14.87 |
| 16 | 14.89 |

**Flat — pin is irrelevant for dense models** (pinning exists for MoE
expert streaming; qwen3.8-27B has 0 expert tensors).

**Sweep-controller bug fixed en route** (bc57245): benchmark URL was
hardcoded to port 8280 while servers start on SWEEP_PORT 8290 — every
"connection refused" sweep failure traced here. KV flags also moved
q4_0-K-only → tq3_0 K+V to match production.

## IQ4_XS depth probe (prefill_probe methodology, ctx 49152, ub 64, resident)

- load → ready: ~30 s
- prefill 16,193 tok: 254.4 tok/s
- prefill 32,385 tok: 215.9 tok/s
- decode 3×64 at 32,385 filled: **8.07 t/s median** (8.04–8.08)
- VRAM after fills: dev0 10,073/12,288 (2,215 free), dev1 4,936/8,192

## Verdict vs the incumbent

| metric | Q3_K_M (certified 08-24) | UD-IQ4_XS (this session) |
|---|---|---|
| shallow decode | 14.05 t/s | 14.9 t/s |
| decode @ depth | 9.21 t/s @ 54.7K | 8.07 t/s @ 32.4K |
| certified depth | 54,692 tok | ≥32.4K (wall not probed) |
| weight quality | ~3.9 bpw K-quants | ~4.5 bpw i-quants |

Reading: IQ4_XS trades ~12% depth decode and a shallower wall for one
weight-quality tier. For deep-context Officina driving, Q3_K_M stays
the daily driver. IQ4_XS is the pick for quality-sensitive work that
stays under ~30-35K tokens. Q4_K_S doesn't fit this pair at useful
depth. Mellum remains the lightweight/MoE outlier (unbenched).

Follow-up if wanted: full depth certification of IQ4_XS (chunked fill
toward the wall, like lull-phase0) — expect the wall below Q3_K_M's
54.7K given 4.4 GiB less weight headroom.
