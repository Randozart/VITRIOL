# Mellum2 higher-quant — Q8_0 / Q5_K_M dual-GPU test (what worked)

Status: complete — both quants verified, tool calling fixed
Date: 2026-08-19

## Question
Would Q8_0 / Q6_K / higher Q4 of Mellum2-12B-A2.5B-Instruct fix the tool-calling
failures seen at low quants (IQ2_M, IQ4_NL, MXFP4)?

## Setup
Hardware: RTX 3060 (12GB) + GTX 1070 Ti (8GB), layer split, ts 24,12, flash-attn
on, full offload (29/29 layers). Context 131072 (native yarn 16×).

| quant | GGUF size | VRAM load (3060/1070) | KV @131K |
|---|---|---|---|
| Q8_0 | 12.93 GB | 8474 / 3619 MiB | 1280 / 512 MiB |
| Q5_K_M | 9.29 GB | 5117 / 3588 MiB | 512 / 384 MiB (at 65536 test) |

Both loaded clean at 131K on the dual-GPU pair. Q8_0 fits dual-GPU (impossible
on the 3060 alone, 12.9 > 12 GB).

## Results

| config | t/s (200 tok) | tool call (single) | tool call (parallel multi) |
|---|---|---|---|
| Q8_0 @ 131K dual | 68.29 | ✅ get_weather `{"city":"Paris","unit":"c"}` | ✅ 2 calls, valid JSON, correct args |
| Q5_K_M @ 131K dual | 69.88 | ✅ get_weather `{"city":"Paris"}` | ✅ 2 calls, valid JSON |

## What WORKED
- **Both Q8_0 and Q5_K_M fix tool calling.** Single-tool and parallel multi-tool
  calls return proper `finish_reason: tool_calls` with valid, correctly-populated
  JSON arguments. This is the exact scenario that failed at low quants.
- **Speed is essentially unchanged by quant** (68.3 vs 69.9 t/s). Mellum2 is a
  2.5B-active MoE — only active experts stream per token, so even the serial
  dual-GPU chain is fast. My earlier serial-penalty fear (Qwen lesson) does NOT
  apply to this small model.
- **Q8_0 fits dual-GPU at full 131K** — the structural win: dual-GPU unlocks
  Q8_0 quality + 131K context that the single 3060 cannot hold.

## Verdict
**Q8_0 dual-GPU @ 131K is the pick**: best quality, 68.3 t/s, full context,
tool calling verified. Q5_K_M is a near-identical fallback (69.9 t/s, slightly
smaller). Either is a massive step up from the Qwen dual-GPU config (12.4 t/s)
for agentic/tool use.

## Profiles (committed)
- `mellum2-q8-131k` — Q8_0, ts 24,12, 131K, flash-attn, ~68 t/s
- `mellum2-q5-131k` — Q5_K_M, ts 24,12, 131K, flash-attn, ~70 t/s

Load: `vitriol config load mellum2-q8-131k`

## Next (optional)
- Test Q6_K (10.99 GB) if Q8 VRAM margin ever tightens.
- Confirm tool-call robustness across many varied prompts (only 2 scenarios
  tested here, both passed).
