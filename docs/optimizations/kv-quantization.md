# Optimization: KV cache quantization

Status: **refuted on this box** (K+V q4_0) / **recorded** (K-only split).
Lever: `--cache-type-k/v` → profile keys `[kv] quant_mode`, `[kv] quant_mode_v`.

## What it is

Quantize the KV cache to 4× density (`q4_0`) to fit more context per slot.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| config | result |
|---|---|
| `--cache-type-k/v q4_0` | decode **13.9 t/s** + server instability (threads=4 config crashed) — **refuted** |

KV quantization is slow and unstable in this fork on this box, and is **not a
usable escape** from the VRAM context budget. The working config keeps KV in
VRAM at f16 (`[kv] mode = standard`, `quant_mode = f16`).

## Config schema note (2026-08-06)

The config split K and V:

- `quant_mode = q4_0` — **K cache only**. Quantizing V causes garbage output
  with VITRIOL.
- `quant_mode_v = f16` — **V cache**; the config menu warns before allowing
  q8_0/q4_0 for V.

Map: `--cache-type-k` → K quant, `--cache-type-v` → V quant.

Sources: `.opencode/plans/KV_QUANT_SESSION.md`,
`.opencode/plans/2026-08-06-spagyric-kv-context-levers.md`.

## Config

```ini
[kv]
mode = standard
quant_mode = f16      ; q4_0 K-only is an option, not a decode win here
quant_mode_v = f16    ; V must stay f16 for correct output
```

## Undo

Return `quant_mode` and `quant_mode_v` to `f16`. If a future fork fixes the
13.9 t/s bottleneck, re-sweep before trusting it.
