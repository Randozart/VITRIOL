# Reasoning Flag Discovery

**Date:** 2026-05-20
**Model:** Qwen3.6-35B-A3B-UD-IQ2_M.gguf (Unsloth)

## Problem

When serving Qwen3.6-35B-A3B-UD-IQ2_M via `vitriol serve`, all output consisted of `?` characters — the Unicode REPLACEMENT CHARACTER (U+FFFD). This affected every prompt and configuration. The model generated tokens at the expected throughput, but the tokenizer could not decode them to readable text.

## Root Cause

The GGUF file's chat template metadata has `thinking = 1` (thinking/reasoning mode). Qwen3.6 is a reasoning model that outputs internal reasoning tokens before the actual response. The server's tokenizer lacks proper mappings for these reasoning delimiters, rendering them as `?`.

The relevant metadata in the GGUF:
```
tokenizer.chat_template: ... {% if enable_thinking is defined and enable_thinking is false %}
                           {{- '<think>\n\n</think>\n\n' }}
                         {% else %}
                           {{- '<think>\n' }}
                         {% endif %} ...
```

When `enable_thinking` is `true` (default), the template injects `<think>` tags and the model outputs reasoning tokens. When `false`, the model outputs the response directly.

## Fix

The `--reasoning off` server flag disables thinking mode. This sets `enable_thinking = false` in the chat template, causing the model to skip reasoning and output the final answer directly.

```bash
# Before (broken):
llama-server -m Qwen3.6-35B-A3B-UD-IQ2_M.gguf ...
# → Output: "???"

# After (fixed):
llama-server -m Qwen3.6-35B-A3B-UD-IQ2_M.gguf ... --reasoning off
# → Output: "Paris, a city renowned for its iconic landmarks..."
```

## Impact

With `--reasoning off` and MTP N=2 enabled:

| Metric | Value |
|--------|-------|
| Throughput | **17.62 t/s** |
| MTP acceptance rate | 98.5% (65/66) |
| Quality | ✅ Clean output |
| vs no-VITRIOL baseline | +209% |

The flag is safe for Qwen3.6 models and has no effect on models without thinking support. It is now the default in `vitriol serve`.

## Configurability

The reasoning flag can be controlled via:
- **CLI:** `--reasoning on|off|auto`
- **Env var:** `LLAMA_ARG_REASONING`
- **Config file:** `vitriol.reasoning = on|off` (default: `on`)
- **TUI:** VITRIOL Mode Settings → option 6
