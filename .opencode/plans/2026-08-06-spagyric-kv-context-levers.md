# Spagyric S4b — KV/Context vs VRAM levers (measured on DeepSeek, GTX 1070 Ti)

Date: 2026-08-06.

## 1. Question

Weights + parallel slots claim all VRAM — where does context (KV cache) live? Tested
the three levers that move KV out of the weight-vs-context competition.

## 2. The three levers, measured

| lever | mechanism | measured result | verdict |
| --- | --- | --- | --- |
| Default (f16 KV in VRAM) | KV per slot in VRAM, GPU attention | decode 58-60 t/s; parallel ceiling p=8@c4096 (VRAM: model + 8x4096 KV) | **working path** |
| `--no-kv-offload` (KV to host RAM) | `offload_kqv=false` -> attention compute + KV on CPU | decode collapses to **15 t/s** (CPU-attention bottleneck on this box); p=1 -> 17.6, p=8 -> 15.5 aggregate (no recovery) | **refuted on this hardware** |
| `--cache-type-k/v q4_0` (quantized KV) | 4x KV density | decode **13.9 t/s** + server instability (threads=4 config crashed) | **refuted on this fork/box** |

## 3. Reading

- The generic llama.cpp KV-offload path (`--no-kv-offload`) moves **attention compute**
  to CPU, not just KV — on this 4C/8T box that is a 4x decode penalty, and the
  parallel aggregate never recovers (CPU attention serializes).
- KV quantization (`q4_0`) here is slow (13.9 t/s) and unstable. Not a usable escape
  hatch on this fork/build.
- **Therefore on this box: KV stays in VRAM, and context is budget-limited by the
  parallel x ctx product.** Bigger context per slot means fewer slots (drop `--parallel`);
  the p=8@c4096 (DeepSeek) config is the measured knee of the "all-VRAM" claim.
- VITRIOL's custom KV offload (AGENT_BRIEF Layer 1a, CUDA-graph-split based, ~470 MB
  VRAM freed for 20K+ ctx) is the *designed* host-RAM path — it is NOT the generic
  `--no-kv-offload`. It remains untested here (deferred with the stream thread).

## 4. Spagyric implication

The autotuner's VRAM budget is 3-way: `weights + slots x KV + scratch <= VRAM`, with
KV quantization/offload **not viable** on this box. So `--parallel` ceiling = floor(
(free_VRAM - scratch) / (KV_per_slot + weight_share)). The frozen DeepSeek profile
(p=8@c4096) stands. A future box with faster CPU attention or the VITRIOL Layer 1a KV
offload would change this — record as a fingerprint-conditional.

## 5. Commands (repro)

```bash
# baseline (VRAM KV): default flags -> 58-60 t/s, p ceiling 8@c4096
# KV in host RAM:
llama-server -m DeepSeek...gguf -ngl 99 -c 4096 --no-kv-offload --parallel 8
#   -> 15.5 t/s aggregate (CPU attention bottleneck)
# quantized KV:
llama-server -m DeepSeek...gguf -ngl 99 -c 4096 --cache-type-k q4_0 --cache-type-v q4_0 --parallel 8
#   -> 13.9 t/s + instability
```

## 6. Follow-up

- VITRIOL Layer 1a KV offload: test behind the stream thread (needs a working stream
  model + the custom kv-mode, not the generic flag).
- Harness fix: capture server stderr to a log per config (the devnull redirect hid the
  q4_0 crash; stderr to a file would have shown it immediately).
