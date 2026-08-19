# Qwen3.8-27B Phase E — decode diagnostics: what worked / what didn't

Status: complete — bottleneck fully characterized
Date: 2026-08-19
Related: `.opencode/plans/qwen38-phase-d-bottleneck-2026-08-19.md`

## TL;DR

The ~98 ms/decode on the dual-GPU (RTX 3060 + GTX 1070 Ti) layer split is
**genuine serial layer-chain GPU execution** (~90 ms), plus **inter-decode host
overhead** (ctx_mtp draft decode + sampling) that drags the GPU's ~20.9 t/s
ceiling down to ~12 t/s. Everything else was measured and falsified.

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

Throughput accounting (winner config, 400 tok / 33.2 s / 12.1 t/s, draft
acceptance 100% 199/199):
- Main verify: 195 × ~98 ms = 19.1 s.
- Sum of ALL decode types = 29.9 s of the 33.2 s wall.
- The gap to 12 t/s = serial draft decodes + sampling.

## Measured reference rows

| config | total | build | compute | post | graph | sync | nodes |
|---|---|---|---|---|---|---|---|
| MTP n=1 (winner) | 98.0ms | 0.3ms | 87.5ms | 10.0ms | 2C/1R | 1×9.7ms | 189 MUL_MAT |
| no-MTP | 78.1ms | 0.1ms | 78.0ms | 0.0ms | 1C/1R | 0 | 112 MUL_MAT |
| ctx_mtp draft (per) | 11.2ms | — | 8.3ms | 2.5ms | 0C/0R | 2.4ms | 13 MUL_MAT |
| prompt chunk | 109.5ms | — | 73ms | 36ms | 1C/0R | 36ms | — |

## What's left (real next directions)

1. **Host-side (most achievable)**: overlap or shrink the per-token ctx_mtp
   draft decode + sampling that sits serially between main decodes — the
   20.9 → 12 t/s gap. Measure the draft/sampling split precisely, try overlap.
2. **Kernel-side**: close the ~90 ms serial GPU time toward the weight-stream
   floor — Q3_K stream efficiency / layer-split scheduling so the chain isn't
   purely serial (the two GPUs each idle ~50%).

## Actions log
- [x] build both archs; `sudo vitriol setup`
- [x] [PERF]/[CUGR] instrumentation; live TUI breakdown
- [x] re-capture cause isolated (`ne[1]` growing context) — then falsified as cost
- [x] context-invariance (8K vs 131K), split-mode row, split-ratio 30/6, throughput accounting
- [ ] (next) draft-decode/sampling split + overlap test
- [ ] (next) kernel/bandwidth efficiency of the serial layer chain