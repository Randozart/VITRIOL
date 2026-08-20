# Qwen3.8 — is VITRIOL throttling it? (A/B result)

Status: hypothesis disproven
Date: 2026-08-19

## Question
The user's hypothesis: VITRIOL's stream path (which throttled Mellum2 88->5
t/s) might also be throttling Qwen3.8, and `mode=off` could unlock "high tps".

## Test
Qwen3.8-27B-Q3_K_M, ts 27,9, MTP n1, ctx 49152, q4_0 KV, lean wrapper (ub 128),
150-token gen, varying `VITRIOL_MODE`.

| mode | t/s |
|---|---|
| native (no env) | 13.02 |
| `VITRIOL_MODE=stream` | 13.97 / 12.39 |
| `VITRIOL_MODE=off` | 13.68 |

## Result
All three are within run-to-run variance (12.4-14.0 t/s). **`VITRIOL_MODE` does
NOT throttle Qwen3.8.** This is the opposite of Mellum2 (where stream = 5 vs
off = 88 t/s).

## Why the difference
- **Mellum2** is small / fits VRAM → the VITRIOL DMA/streaming machinery is
  pure overhead → 18x regression.
- **Qwen3.8** is 13.8GB, does NOT fit on either GPU alone, uses dual-GPU
  offload → VITRIOL's stream/DMA path is engaged productively for the offloaded
  portions → roughly neutral.

## Conclusion
Qwen3.8's ~12.4-14 t/s is the **genuine native serial-dual-GPU ceiling**
(Phase E: context-invariant weight-streaming, ~54 ms/token). Turning VITRIOL
off does NOT unlock higher Qwen t/s. No `mode` toggle changes the serial
layer-chain bottleneck.

**Action:** leave Qwen profiles at `mode=stream` (neutral, no need to change to
off as with Mellum2). For "high tps" local coding, the hardware-fitted answer
remains Mellum2-Claude-Thinking (69 t/s) — Qwen3.8 is hardware-capped at ~13.
