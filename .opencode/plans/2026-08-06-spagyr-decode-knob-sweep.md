# Spagyr S2+S3 — Decode-Knob Sweep (ubatch / threads / parallel)

Date: 2026-08-06.

## 1. Goal

Measure the decode-knob knee on the real VITRIOL runtime (llama-server) to decide what
`--spagyr-tune` should autotune. Two sweep shapes, both models. Baseline reference:
Spagyr Phase 0 report (`2026-08-06-spagyr-phase0-baseline-report.md`) — DeepSeek
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

`VITRIOL/libvitriol/spagyr_sweep.py` — starts/waits/kills llama-server per config,
mode A (3 rounds) or mode B (N concurrent via threads), writes CSV
(`/tmp/opencode/spagyr_sweep_<model>.csv`). Reuses the sweep_controller methodology
(health poll, warmup, t/s from timings).

## 5. Results table (fill on execution)

| model | knob | value | decode t/s | eval t/s | concurrent t/s (wall) | correct |
| --- | --- | --- | --- | --- | --- | --- |
| DeepSeek | ubatch | 512 (stock) | 58.1-58.3 | 56.7-58.4 | — | PASS |
| DeepSeek | ubatch | 64 | TBD | TBD | — | TBD |
| DeepSeek | ubatch | 128 | TBD | TBD | — | TBD |
| DeepSeek | ubatch | 256 | TBD | TBD | — | TBD |
| DeepSeek | threads | 2/8 | TBD | TBD | — | TBD |
| DeepSeek | parallel | 2/4/8 | — | — | TBD | TBD |
| Mellum | ubatch | 512 (stock) | 30.9-34.3 | ~49 | — | PASS |
| Mellum | ubatch | 64/128/256 | TBD | TBD | — | TBD |
| Mellum | threads | 2/8 | TBD | TBD | — | TBD |
| Mellum | parallel | 2/4 | — | — | TBD | TBD |

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
