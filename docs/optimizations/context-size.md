# Optimization: context size budgeting

Status: **validated** — a budget, not a speed knob.
Lever: `--ctx` / `-c` → profile key `[model] context`.

## What it is

The KV cache lives in VRAM on this box, so context is budget-limited by the
`weights + slots × KV + scratch <= VRAM` equation. Larger context per slot means
fewer parallel slots (and vice versa).

## Measured (bitshaper-ai, 2026-08-06)

| config | result |
|---|---|
| DeepSeek p=8 @ c=4096 | decode 58–60 t/s; **the measured knee** of the all-VRAM claim |
| DeepSeek p=8 @ c=32768 | VRAM ceiling — cannot hold both weights + 8 slots of 32K KV |

KV per slot (f16) on Mellum: 32,256 B ≈ 31.5 KB/token (2×28 layers × 4 kv_heads
× 72 head_dim × 2 B). Native context_length 131,072 (yarn ×16 of 8192) is not
achievable at full parallel on 8 GB — context and slots trade off 1:1.

Source: `.opencode/plans/2026-08-06-spagyric-kv-context-levers.md`.

## Rule

Context is budget-limited by the parallel×ctx product. To grow context, drop
`--parallel` first. KV quantization/offload paths on this box are refuted (see
`kv-offload.md`, `kv-quantization.md`).

## Config

```ini
[model]
context = 4096   ; DeepSeek profile; trade against [server] parallel
```

## Undo

Raise parallel → lower context, or lower context → raise parallel. No other
levers change this equation on CC 6.1.
