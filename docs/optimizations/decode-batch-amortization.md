# Optimization: decode batch amortization (ubatch)

Status: **validated** — the autotune target.
Lever: `--ubatch-size` → profile key `[engine] ubatch_size`.

## What it is

Batch the batched-forward pass so the weight fetch (the bottleneck on this
class of hardware) is amortized across more tokens. In a server, parallel slots
share one forward pass — the batch amortization expressed in native llama.cpp.

## Why it works here

The GTX 1070 Ti (Pascal CC 6.1, 2 MB L2, PCIe 3.0 x16, no AVX2) is
**memory-bandwidth-bound** at decode: the model weights are the fetch bottleneck,
so amortizing that fetch across the batch is the highest-leverage knob measured.

## Measured (bitshaper-ai, 2026-08-06)

Decode-knob sweep, single-request mode A (`ubatch` at t=4, parallel=1):

| knob | value | decode t/s |
|---|---|---|
| ubatch | 64 | 60.19 |
| ubatch | 128 | 59.76 |
| ubatch | 256 | 59.95 |
| ubatch | 512 | 59.83 |

**ubatch is NOT a decode lever at single-request** — flat 59.8–60.2 on DeepSeek,
28.3–31.1 on Mellum. The batch win appears only as **parallel slots** (see
`threads-and-parallel.md`): 78.5 → 87.9 → 135.8 t/s at p=2/4/8 = **2.3×**
single-slot. Source: `.opencode/plans/2026-08-06-spagyric-decode-knob-sweep.md`.

## Undo / tuning

Leave `ubatch_size` at the server default. If a different model class shows a
non-flat ubatch curve, sweep again and set the knee. The real lever is
`--parallel`.

## Config

```ini
[engine]
ubatch_size = 512   ; default; not a decode lever on this box
```
