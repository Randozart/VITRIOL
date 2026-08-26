# BENCHMARKS — certified numbers only

> Rule of evidence: every number below is a **filled-context** measurement
> (prefill to N tokens, decode at depth) with a `VITRIOL-FINGERPRINT` /
> full-argv trail. Shallow-bench figures are labeled as such and prove
> nothing about depth. Raw reports: `.opencode/plans/lull-certification-
> report-2026-08-24.md` and successors.

## Hardware baseline (the box this was tuned on)

| component | detail |
|---|---|
| CPU | i7-3770 (Ivy Bridge, AVX only — no AVX2) |
| RAM | 16 GiB DDR3 dual-channel + 7.8 GiB zram + 8 GiB swapfile |
| GPU 0 | RTX 3060 12 GiB (sm_86) |
| GPU 1 | GTX 1070 Ti 8 GiB (sm_61, Pascal) |
| Build | CUDA archs `61;86` both mandatory |

## Qwen3.8-27B (UD-IQ3_S / Q3_K_M, qwen35 arch + MTP head)

| config | depth reached | t/s at depth | notes |
|---|---|---|---|
| IQ3_S + tq3_0 KV, ts 26,10, ub64 | **96,836 tok** | 11.32 | deep-context champion |
| IQ3_S + q4_0 KV, ts 24,12 | 92,642 tok | 7.8–12.4 | certification matrix run |
| Q3_K_M + tq3_0 KV, ts 26,10, ub64 | 54,692 tok | 9.21 | beats historical 45–61k OOM zone |
| Q3_K_M + tq3_0 KV (shorter) | 43,890 tok | 9.47 | |
| IQ2_S @ 64,634 tok | 64,634 tok | 11.7–12.7 | quant headroom trade |

## Production profiles

| profile | context | split | notes |
|---|---|---|---|
| `qwen38-master` / `qwen38-ontic` | c=81,920 | slots 0=73,728 / 1=8,192 | dual-slot tenancy; hermes slot 0, ontic slot 1; `--cache-ram 1024` |
| hard cap found | c=98,304 OOM'd under host pressure | | memory exhaustion ends in bounce, not graceful degradation |

## Negative results (see VERDICTS.md)

| experiment | result |
|---|---|
| MTP sweep (5×5 pin × draft-n) | all configs 9.6–9.98 t/s — zero benefit; n≥2 regresses to 8.58 |
| tq3_0 KV in non-stream mode | −40% decode penalty for some configs; q4_0 KV is the fast path |
| streaming/DMA offload of fitting models | pessimization on DDR3/PCIe |

## Shallow-bench history (NOT depth-certified)

| profile | shallow t/s | caveat |
|---|---|---|
| `qwen38-mtp-131k` legacy | ~14.1 | window allocation ≠ usable depth |
| `qwen38-iq3s-131k`, ts 70,30 | — | user profile, preserved |

## VRAM creep observation

~23 KiB/token growth on device 0 during long prefills is the depth wall —
independent of KV bits, not fixed by `GGML_CUDA_NO_VMM=1`. Budget accordingly.
