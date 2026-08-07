# Optimization: activation-delta execution — REFUTED

Status: **refuted**.
Lever: none (removed); an activation-delta execution scheme.

## What it is

Track per-layer activation *deltas* and compute only the changed part, skipping
recompute where activations are stable.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| metric | measured |
|---|---|
| speed | **14.5× worse** |
| delta stability | unstable — median \|Δ\|/rms 0.52–0.72 |

The deltas are not stable enough to skip work; the added delta-tracking cost
makes execution 14.5× slower than the baseline. Refuted.

Source: `docs/spagyric-autotuner.md` §3.

## Undo

Already removed. Would only be reconsidered with a demonstrated stable-delta
signal per layer (median \|Δ\|/rms near 0 across a real workload).
