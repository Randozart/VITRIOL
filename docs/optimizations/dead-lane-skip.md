# Optimization: dead-lane skip — RECORDED (not an autotune knob)

Status: **recorded** — measured, but NOT an autotune knob.
Lever: none exposed.

## What it is

Skip compute for lanes whose activations are measurably zero.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| metric | measured |
|---|---|
| zero lanes | 39–47% across real tensors |
| classification | a **compute** lever on a **memory-bound** box |

Dead-lane skip is a compute lever, and this box is memory-bound — skipping dead
lanes does not relieve the weight-fetch bottleneck. Recorded, not wired into the
autotuner's knob set.

Source: `docs/spagyric-autotuner.md` §3.

## Undo

Nothing to undo — it was never an autotune knob. If a workload becomes
compute-bound (e.g. larger models with sparse activations), re-measure before
considering it.
