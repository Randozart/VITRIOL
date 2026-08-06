# Spagyric S2+S3 — Decode-Knob Sweep (ubatch / threads / parallel)

Date: 2026-08-06.

## 1. Goal

Measure the decode-knob knee on the real VITRIOL runtime (llama-server) to decide what
`--spagyric-tune` should autotune. Two sweep shapes, both models. Baseline reference:
Spagyric Phase 0 report (`2026-08-06-spagyric-phase0-baseline-report.md`) — DeepSeek
58.1-58.3 t/s, Mellum 30.9-34.3 t/s at stock decode knobs (default ubatch 512, t=4,
parallel 1).

## 2. Methodology

- **Mode A — single-request decode t/s.** One completion (merge-sort prompt, 64 tokens,
  temp 0), warmup + 3 measured rounds, per config. Correctness gate: output must contain
  `def merge_sort` (else the config is marked FAIL, no t/s claim).
- **Mode B — concurrent-request throughput.** Server started with `--parallel N`; send
  N concurrent 64-token completions, measure wall time; aggregate throughput =
  `N*64 / wall`. This is the slot-shared amortization test (native MoE analog of the
  measured dense batch win).
- Fresh server launch per config (knobs are startup flags). Server lifecycle managed by
  the sweep harness; stale servers killed with `killall -9 llama-server`.

## 3. Grid

| knob | values | models |
| --- | --- | --- |
| ubatch-size | 64, 128, 256, 512 | both (t=4, parallel=1, batch=2048) |
| threads | 2, 8 (at ubatch=256) + 4 (from ubatch sweep) | both |
| parallel (Mode B) | 2, 4, 8 (DeepSeek c=4096); 2, 4 (Mellum c=32768) | both, default ubatch, t=4 |

Base per model: DeepSeek ngl=99 c=4096; Mellum ngl=24 c=32768 (from Phase 0 + mellum2
profile). Mellum parallel capped at 4 to avoid KV OOM at c=32768.

## 4. Harness

`VITRIOL/libvitriol/spagyric_sweep.py` — starts/waits/kills llama-server per config,
mode A (3 rounds) or mode B (N concurrent via threads), writes CSV
(`/tmp/opencode/spagyric_sweep_<model>.csv`). Reuses the sweep_controller methodology
(health poll, warmup, t/s from timings).

## 5. Results (measured 2026-08-06)

**DeepSeek-Coder-V2-Lite IQ2_M** (ngl=99, c=4096), all correctness PASS:

| knob | value | decode t/s | eval t/s | concurrent t/s (aggregate) |
| --- | --- | --- | --- | --- |
| ubatch | 64 | 60.19 | 60.01 | — |
| ubatch | 128 | 59.76 | 59.46 | — |
| ubatch | 256 | 59.95 | 59.10 | — |
| ubatch | 512 | 59.83 | 59.38 | — |
| threads | 2 | 59.49 | 59.26 | — |
| threads | 8 | 59.61 | 59.12 | — |
| **parallel** | **2** | — | — | **78.50** |
| **parallel** | **4** | — | — | **87.86** |
| **parallel** | **8** | — | — | **135.83** |

**Mellum2-12B Q4_K_M** (ngl=24, c=32768), all correctness PASS:

| knob | value | decode t/s | eval t/s | concurrent t/s (aggregate) |
| --- | --- | --- | --- | --- |
| ubatch | 64 | 29.84 | 44.72 | — |
| ubatch | 128 | 28.31 | 39.03 | — |
| ubatch | 256 | 28.81 | 43.83 | — |
| ubatch | 512 | 31.05 | 43.91 | — |
| threads | 2 | 27.64 | 32.36 | — |
| threads | 8 | **2.24** | 8.78 | — |
| **parallel** | **2** | — | — | **37.17** |
| **parallel** | **4** | — | — | **41.84** |

## 5b. Reading (DeepSeek)

- **ubatch: NOT a decode lever** (flat 59.8–60.2). Decode is memory-bound; ubatch chunks
  the batched forward but at single-request it changes nothing.
- **threads: NOT a decode lever** (flat 59.5–59.8) on this GPU-bound decode.
- **parallel slots: THE lever.** Aggregate throughput 78.5 → 87.9 → 135.8 t/s for
  p=2/4/8. The slot-shared amortization is real in native llama.cpp: one forward pass
  serves all slots, weight fetch amortized. 8 slots ≈ 2.3× single-slot (~60 t/s).

So `--spagyric-tune`'s decode autotune axis is **`--parallel`**, not ubatch/threads.
Finer parallel sweep (p=6/12/16) + the real VITRIOL stream knobs (LRU/prefetch/pin)
are the next S4 candidates.

## 5c. Reading (both models, cross-check)

- **ubatch: not a decode lever on either model** (DeepSeek 59.8–60.2 flat; Mellum
  28.3–31.1 flat). Confirms decode is memory/bandwidth-bound, not ubatch-chunk-bound.
- **threads: t=4 is the floor-best on this 4C/8T box.** DeepSeek flat across 2/4/8
  (59.5–59.8, GPU-bound). Mellum: t=2 slightly worse (27.6), **t=8 catastrophic
  (2.24 t/s, ~13× slower)** — hyperthread contention on the shared MoE/CPU-side work.
  t=4 is the safe universal choice.
- **parallel slots: the only decode throughput lever.** DeepSeek (bandwidth-bound,
  IQ2_M): 78.5 → 87.9 → **135.8 t/s** at p=2/4/8 = **2.3× single-slot**. Mellum
  (compute-bound, Q4_K_M): 37.2 → 41.8 at p=2/4 = **1.4×**. The amortization is real
  in native llama.cpp; the win scales with how bandwidth-bound the model is.

Implication for `--spagyric-tune`: autotune `--parallel` (fine-grained: 1/2/4/6/8/12/16)
as the primary decode knob; fix threads=4; leave ubatch at default. Mellum-scale models
gain ~1.4×, bandwidth-bound models up to 2.3×+.

## 6. Expected

- Mode A: decode t/s roughly flat across ubatch (decode is memory-bound; ubatch chunks
  the batched forward, should matter little at single-request) — a flat result is itself
  informative (shows ubatch is not the decode lever; parallel/concurrency is).
- Mode B: aggregate throughput should rise with N while PCIe/DRAM-bound, flattening at
  the knee — the amortization win, expressed in native llama.cpp.
- threads: expect t=4 best on this 4C/8T box (documented earlier: t=8 is 25% worse on
  the Qwen ternary).

## 7. Cross-repo

Plan + results recorded in both repos (bitshaper-ai canonical, VITRIOL mirror). Harness
lives in VITRIOL (feature tooling).
