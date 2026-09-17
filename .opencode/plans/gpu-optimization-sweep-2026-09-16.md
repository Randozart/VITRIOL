# GPU Optimization Sweep: Dual RTX 3060 12GB

Date: 2026-09-16 23:00 -> 2026-09-17 08:57
Status: DONE
Owner: randozart

## Hardware

- GPU 0: RTX 3060 12GB (sm_86, dev0, display)
- GPU 1: RTX 3060 12GB (sm_86)
- Host RAM: 32 GiB
- No NVLink (PCIe consumer cards)

## Baseline (ub=64, no env vars)

```
ts=24,24  ctx=262144  ubatch=64  parallel=1  ngl=99  fa=on  no-mmap
kv=q4_0/q4_0  ckpt_min_step=8192  ckpt_ring=4  cache_ram=2048
spec=mtp:1  mode=off  engine=vitriol-dma  reasoning=off
```

Baseline decode (3x64, `/completion`): **22.89 t/s** (22.24/23.24/23.20)

## Results

| Test | Env/Flag | t/s | vs baseline | VRAM GPU0/GPU1 (MiB) | Notes |
|---|---|---|---|---|---|
| Baseline (ub=64) | none | 22.89 | — | 10,502 / 11,690 | 221 MiB free GPU1 |
| GGML_CUDA_GRAPH_OPT=1 | env | 23.38 | +2.1% | 10,518 / 11,706 | Concurrent QKV in CUDA graphs |
| cuBLAS bf16 | env | 23.48 | +2.6% | 10,522 / 11,710 | bf16 tensor cores on sm_86 |
| GGML_CUDA_P2P=1 | env | 23.57 | +3.0% | 10,518 / 11,706 | Peer access; no help for layer-split |
| **ub=128** | **flag** | **23.67** | **+3.2%** | **10,660 / 11,776** | **Main lever. 135 MiB free GPU1.** |
| ub=256 | flag | OOM | — | — | GPU1 can't fit MTP draft (1024 MiB) |
| tq3_0 K + q4_0 V | flag | 16.55 | -27.7% | 10,126 / 11,448 | Hadamard transform overhead kills decode |
| GRAPH_OPT + bf16 + ub=128 | combined | 23.63 | +3.2% | 10,664 / 11,780 | Gains don't compound |
| All flags (GRAPH_OPT+bf16+P2P+ub=128) | combined | 23.28 | +1.7% | 10,664 / 11,780 | P2P hurts in combos |
| -sm tensor | flag | CRASH | — | — | Assert ggml-backend-meta.cpp:1704 + Vulkan OOM |
| No MTP + ub=128 | flag | 15.24 | -33.4% | 10,582 / 10,370 | MTP provides +55% speedup |

## Key Findings

1. **ubatch is the main lever**: ub 64->128 = +3.2% (22.89 -> 23.67 t/s). Simple, free, safe.
2. **CUDA env vars are marginal**: GRAPH_OPT +2.1%, bf16 +2.6% individually, but gains
   don't compound when combined with ub=128 (23.63 same as ub=128 alone).
3. **P2P hurts in combos**: fine alone (+3.0%) but degrades when stacked with other flags
   (+1.7% all-combo vs +3.2% ub=128 alone). Not worth the complexity.
4. **tq3_0 K is a net LOSS**: 16.55 t/s vs 22.89 baseline (-27.7%). The randomized
   Hadamard transform overhead in dequantization costs more than the VRAM savings buy.
   V-side tq3_0 is also KNOWN BROKEN on CUDA (fattn rewrite).
5. **-sm tensor is dead**: crashes with assert + Vulkan OOM fallback. No NVLink means
   PCIe allreduce would kill performance anyway.
6. **MTP is the dominant lever**: +55% speedup (15.24 -> 23.67 t/s). ubatch is +3% on top.
7. **ub=256 is the VRAM ceiling**: GPU1 has 135 MiB free at ub=128; ub=256 needs ~1 GiB
   more compute buffer which doesn't fit.

## Applied Changes

- `~/.vitriol/config`: `ubatch_size = 64` -> `ubatch_size = 128`
- `scripts/vitriol`: Added `GGML_CUDA_GRAPH_OPT=1` and `GGML_CUDA_CUBLAS_COMPUTE_TYPE=bf16`
  to both detached env blocks (memory-mode and direct paths)
- Blessed fingerprint (2026-09-17T08:56:54): `ub=128` (was ub=64)
- All other config unchanged (ts=24,24, kv=q4_0/q4_0, spec=mtp:1, etc.)

## What Was NOT Worth It

| Lever | Verdict | Reason |
|---|---|---|
| tq3_0 K | SKIP | -27.7% decode; VRAM savings don't compensate |
| P2P | SKIP | Marginal alone, harmful in combos |
| -sm tensor | DEAD | Crashes, no NVLink |
| -sm row | DEAD | Deleted upstream |
| ub>128 | OOM | VRAM ceiling at 128 |
| Unified memory | SKIP | Bad without NVLink |
| parallel>1 | SKIP | Would need ctx reduction for multiple KV pools |
