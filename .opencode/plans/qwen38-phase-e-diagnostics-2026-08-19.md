# Qwen3.8-27B Phase E — decode diagnostics: what worked / what didn't

Status: complete — bottleneck fully characterized
Date: 2026-08-19
Related: `.opencode/plans/qwen38-phase-d-bottleneck-2026-08-19.md`

## TL;DR

The ~98-108 ms/decode on the dual-GPU (RTX 3060 + GTX 1070 Ti) layer split is
**genuine serial layer-chain GPU execution** — 81.5% of wall time (54 ms/token),
context-invariant, weight-streaming-bound. Host overhead is minor (~11%);
the ctx_mtp draft decode is negligible (1 ms). The only real lever is
**kernel/bandwidth efficiency** of the serial chain or **more tokens per decode**.
Everything else was measured and falsified.

## Instrumentation (what we built to measure)

Env-gated by `GGML_CUDA_GDN_PROFILE=1` (same toggle as the existing `[GDN]`/
`[DEC]` timers). Committed.

- **llama-context.cpp** — per-decode split of `build` (graph build+alloc+
  `set_inputs`) vs `compute` (`graph_compute`) vs `post` (extraction + MTP
  hook). Thread-local reentrancy depth guard so the nested `ctx_mtp` decode
  folds into the outer decode's `post` instead of emitting its own line.
- **ggml-cuda.cu** — CUDA-graph capture-vs-replay tallies + per-op node census;
  new `ggml_cuda_perf_reset/get` `GGML_BACKEND_API`; field-level diff logging
  in `ggml_cuda_graph_update_required` (`[CUGR]` lines) to name the exact
  node property that flips each decode.
- **MTP hook** — sync-stall census (the `synchronize()` in the ctx_mtp path).
- **TUI** — parses `[PERF]` and renders the breakdown live in the GEN card
  (build/compute/post + graph C/R + sync). 2 parser unit tests.

Output per decode:
```
[PERF] total=98.0ms build=0.3ms compute=87.5ms post=10.0ms graph=2C/1R sync=1(9.7ms) top_ops=MUL_MAT=189 ADD=57 GET_ROWS=32 CPY=31
```

## What DIDN'T work (hypotheses measured and falsified)

| hypothesis | result | evidence |
|---|---|---|
| graph *build* is the overhead | **no** — 0.3 ms | `build=` field |
| CUDA-graph *re-capture* per decode | **no** — reaches `0C/1R` stable replay | graph counter; total **invariant** to capture vs replay |
| *attention over 131K context* (16 full-attn layers) | **no** — `-c 8192` vs `-c 131072` identical ~98ms | [PERF] |
| `--split-mode row` rebalances GPUs | **regression** — 189→561 MUL_MAT, prompt 655ms, decode 178ms | A/B |
| `--tensor-split 30,6` rebalances toward 3060 | **no room** — `failed to fit params` + OOM device 0 | probe |
| delta-net (GDN) kernels | **no** | earlier Phase C |
| per-layer PCIe syncs | **no** — layer split = 2-3 graph splits | Phase D |

## What DID work (the measured bottleneck)

1. **Main verify decode ~98 ms / 2 tokens** — the serial layer chain alternates
   GPUs, so each is ~50% busy *by design*; the time is the serial sum of layers,
   context-invariant (weight-streaming ~153 GB/s over 13.8 GiB Q3_K, vs ~300+
   GB/s theoretical → kernel/bandwidth headroom exists). ~**20.9 t/s ceiling**.
2. **Inter-decode serial overhead** — the per-token ctx_mtp draft decode
   (~11 ms for a 13-MUL_MAT graph, direct-eval `0C/0R`) + sampling, sitting
   serially between main decodes. Drags 20.9 → **12 t/s** (~40% loss).

## CORRECTION (2026-08-19, precise per-graph accounting)

The earlier "host overhead ~40%" was **wrong**. A clean 500-token run
(41.45 s / 12.06 t/s) with exact per-pattern classification:

| graph | n | avg | sum | what |
|---|---|---|---|---|
| MUL_MAT=189 | 244 | 108.3ms | 26.4s | main verify (2 tok) |
| MUL_MAT=185 | 68 | 108.5ms | 7.4s | main verify (boundary/1 tok) |
| MUL_MAT=9 | 249 | **1.0ms** | 0.2s | **ctx_mtp draft — negligible!** |
| MUL_MAT=558/574 | 9 | 193-545ms | 2.5s | prompt processing |
| all decode | 572 | — | 36.8s | 88.7% of 41.45s wall |

Corrected split of the 41.45 s wall:
- **Main decode GPU: 33.8 s (81.5%)** → ~16.4 t/s ceiling (108 ms/2 tok = 54 ms/tok)
- Prompt processing: ~2.8 s (7%)
- **True non-decode host overhead (sampling + server): ~4.65 s (~11%) = ~9 ms/token**
- ctx_mtp draft: **1 ms/token, negligible** (not the 11 ms earlier mis-read)

So the draft decode is NOT a lever. The main decode GPU time is the single
dominant cost (81.5%). The only real ways forward are **kernel/bandwidth
efficiency of the serial layer chain** or **more tokens per decode**.

## What's left (real next directions)

1. **Kernel-side (the only real lever)**: close the ~108 ms/2-token serial GPU
   time toward the weight-stream floor. Q3_K stream efficiency, layer-split
   scheduling, or batching more tokens per decode.
2. ~~Host-side draft/sampling overlap~~ — **dropped**: draft is 1 ms, host
   overhead only ~11%.

## Actions log
- [x] build both archs; `sudo vitriol setup`
- [x] [PERF]/[CUGR] instrumentation; live TUI breakdown
- [x] re-capture cause isolated (`ne[1]` growing context) — then falsified as cost
- [x] context-invariance (8K vs 131K), split-mode row, split-ratio 30/6
- [x] precise per-graph accounting (draft=1ms, host=11%, main=81.5%) — draft/host levers dropped
- [ ] (next) kernel/bandwidth efficiency of the serial layer chain (Q3_K stream, split scheduling)
- [ ] (next) more tokens per decode (batch / spec depth) to amortize the 108ms