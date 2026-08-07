# Optimization: threads and parallel slots

Status: **validated** (parallel) / **flat** (threads).
Lever: `--parallel` → `[server] parallel`; `--threads` → `[model] threads`.

## What it is

Two server knobs:
- **threads** — CPU threads for compute-heavy parts of the forward.
- **parallel** — concurrent decode slots sharing one batched forward pass.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

Decode-knob sweep, mode B (concurrent 64-token completions):

| knob | value | aggregate t/s |
|---|---|---|
| parallel | 2 | 78.5 |
| parallel | 4 | 87.9 |
| parallel | 8 | **135.8** (DeepSeek IQ2_M) |

- **parallel = the decode lever**: 2.3× single-slot (~60 t/s) at p=8 — the
  weight fetch amortizes across slots. Mellum (compute-bound, Q4_K_M):
  37.2 → 41.8 at p=2/4 = 1.4×.
- **threads: t=4 is floor-best on this 4C/8T box.** DeepSeek flat across
  2/4/8; t=8 was 25% worse in earlier runs (hyperthread contention). One
  measured outlier: 2.24 t/s at t=2 in a prior Mellum sweep (~13× slower —
  MoE/CPU-side contention, not the steady state).

## Conclusion

`--parallel` is the primary decode autotune axis; fix `threads=4`; leave ubatch
at default. Source: `.opencode/plans/2026-08-06-spagyric-decode-knob-sweep.md`.

## Config

```ini
[model]
threads = 4

[server]
parallel = 8
```
