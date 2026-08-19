# Mellum2-12B-A2.5B — higher quant for tool calling (consolidated findings)

Status: complete — Q8_0/Q5_K_M verified, tool calling fixed, slowdown culprit found
Date: 2026-08-19
Hardware: RTX 3060 (12GB) + GTX 1070 Ti (8GB), dual-GPU layer split (ts 24,12)
Related: `mellum2-quant-toolcall-2026-08-19.md`, `mellum2-5tps-culprit-2026-08-19.md`

## Background
Mellum2-12B-A2.5B-Instruct (2.5B-active MoE, 64 experts/8 active, 28 layers,
GQA-4 KV, SWA 1024, yarn 16× → 131072 native) failed **tool calling** at low
quants (IQ2_M, IQ4_NL, MXFP4). Proposal: a higher quant (Q8/Q6/Q5) should fix it.

## What we tested
Two new quants downloaded from bartowski HF repo, run on the dual-GPU pair at
full 131K context, flash-attn on, full offload.

| quant | GGUF size | fits 3060 alone (12GB)? | fits dual-GPU @131K? |
|---|---|---|---|
| Q4_K_M | 8.17 GB | yes | yes |
| Q5_K_M | 9.29 GB | yes | yes |
| Q6_K | 10.99 GB | borderline | yes |
| Q8_0 | 12.93 GB | **no** | **yes** (the reason to use dual-GPU) |

## Results

### Speed
| config | t/s @131K |
|---|---|
| Q8_0 dual-GPU | 68.3 |
| Q5_K_M dual-GPU | 84-88 (native) |
| Q5_K_M dual-GPU (stream mode) | ~5 (see culprit below) |

Speed is essentially quant-independent once running native — Mellum2's small
active-MoE only streams active experts per token, so even the serial dual-GPU
chain is fast (the Qwen serial-penalty lesson does NOT apply here).

### Tool calling (the actual fix)
Both Q8_0 and Q5_K_M produce **valid structured tool calls** in the scenario
that failed at low quants:
- Single tool → `finish_reason: tool_calls`, `get_weather {"city":"Paris","unit":"c"}`
- Parallel multi-tool → 2 valid JSON calls with correct args extracted.
- Verified across both quants.

Verdict: **Q8_0 is the pick** (best quality, negligible speed cost vs Q5);
Q5_K_M is a near-identical, marginally-faster fallback.

## The 5 t/s slowdown — culprit isolated
`vitriol serve` gave ~5.5 t/s instead of the expected ~85. Full bisection:

| test | t/s |
|---|---|
| plain llama-server | 84.0 |
| `--no-mmap` | 85.8 |
| `--kv-unified --cache-idle-slots --ubatch-size 256` | 85.6 |
| `--checkpoint-every-n-tokens 2048` | 85.1 |
| `VITRIOL_MODEL_PATH` env | 88.1 |
| `VITRIOL_ENGINE_MODE=vitriol-dma` | 4.66 |
| `VITRIOL_ENGINE_MODE=native` | 5.04 |
| **`VITRIOL_MODE=stream`** | **5.04** |
| **`VITRIOL_MODE=off`** | **88.6** |

**Culprit: `VITRIOL_MODE=stream`** (config `[vitriol] mode = stream`, the
default) — it engages the VITRIOL streaming/DMA path, ~18× slower for a small
fully-offloaded MoE. All other flags and `VITRIOL_ENGINE_MODE` values are
innocent. `VITRIOL_MODE=off` short-circuits the VITRIOL path entirely.
The stream mode was tuned for the big Qwen models (needs DMA/RAM-shot);
Mellum2 fits VRAM and should run native (`mode = off`).

## Also learned
- **Q8_0 requires dual-GPU** (12.93GB > 3060's 12GB) — this is the structural
  win of spreading Mellum2 across both GPUs.
- **Higher quant fixes tool calling** (low-quant error corrupts the precise
  formatting tool calls need).

## Profiles (all committed, `[vitriol] mode = off`)
- `mellum2-q8-131k` — Q8_0, ts 24,12, 131K, ~68 t/s, best quality (ACTIVE)
- `mellum2-q5-131k` — Q5_K_M, ts 24,12, 131K, ~85-88 t/s, near-identical tool calling

Load: `vitriol config load mellum2-q8-131k` (or `mellum2-q5-131k`).

## Run
```sh
vitriol serve
```

## Next (optional)
- Q6_K (10.99GB) as a RAM-safe middle option if Q8 VRAM margin tightens.
- Broader tool-call robustness pass across many varied prompts (only 2
  scenarios tested; both passed).
