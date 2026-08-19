# Mellum2 5 t/s slowdown — culprit isolated

Status: fixed
Date: 2026-08-19

## Symptom
`vitriol serve` with Mellum2-12B-A2.5B Q5_K_M ran at ~5.5 t/s, but a direct
`llama-server` run of the same model/config measured 68-88 t/s.

## Bisection (Q5_K_M, dual-GPU, 131K, -fa on, -ts 24,12)

| test | t/s |
|---|---|
| plain llama-server (baseline) | 84.0 |
| + `--no-mmap` | 85.8 |
| + `--kv-unified --cache-idle-slots --ubatch-size 256` | 85.6 |
| + `--checkpoint-every-n-tokens 2048` | 85.1 |
| + `VITRIOL_MODEL_PATH` env | 88.1 |
| + `VITRIOL_ENGINE_MODE=vitriol-dma` | 4.66 |
| + `VITRIOL_ENGINE_MODE=native` | 5.04 |
| **+ `VITRIOL_MODE=stream`** | **5.04** |
| + `VITRIOL_MODE=off` | **88.6** |

## Culprit
**`VITRIOL_MODE=stream`** (set by `[vitriol] mode = stream` in the config, the
default). It engages the VITRIOL streaming/DMA integration path which cripples
decode for a small fully-offloaded MoE like Mellum2 (~18× slower). All other
flags are innocent.

## Fix
Set `[vitriol] mode = off` in the Mellum2 profile. Verified 88.6 t/s.
`VITRIOL_MODE=off` short-circuits the VITRIOL path regardless of
`engine.mode`.

Applied to `mellum2-q5-131k` and `mellum2-q8-131k` profiles; `mellum2-q5-131k`
reloaded as active.

## Note
The VITRIOL stream mode was presumably tuned for the Qwen models (13GB+, needs
DMA/RAM-shot). Mellum2 (9-13GB, fits VRAM) does not need it and should run
native (`mode = off`).
