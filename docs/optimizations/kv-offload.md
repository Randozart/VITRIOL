# Optimization: KV offload paths

Status: **refuted on this box** (generic path) / **recorded** (VITRIOL Layer 1a, untested).
Lever: `--no-kv-offload`, env `VITRIOL_KV_MODE` → profile key `[kv] mode`.

## What was tested

Two ways to move KV out of the weight-vs-context VRAM competition:

- **Generic `--no-kv-offload`** (`offload_kqv=false`): KV + *attention compute*
  move to CPU.
- **VITRIOL Layer 1a `VITRIOL_KV_MODE=offload`**: KV storage in page-locked host
  RAM (CUDA_Host buffer, `llama-kv-cache.cpp:214-220`), **attention stays on
  GPU** (PCIe DMA for KV reads). This is the designed host-RAM path — NOT the
  generic flag.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| path | result |
|---|---|
| Default (f16 KV in VRAM) | decode 58–60 t/s; p=8@c4096 working path |
| `--no-kv-offload` | **15 t/s** collapse (CPU-attention bottleneck); p=1→17.6, p=8→15.5 aggregate, no recovery — **refuted** |
| Prior Qwen 35B (Layer 1a, custom) | 5.80 vs 6.21 t/s = **−6.6% PCIe penalty**, freed ~470 MiB VRAM, 2 graph splits vs 17 — recorded, per-model |

The generic path is refuted on this hardware: CPU attention is the bottleneck.
Layer 1a is the *designed* escape hatch and is **untested** on this box
(deferred behind the stream thread). Its penalty is a PCIe KV-read tax that
buys ~470 MiB — worth it only for a larger-context budget.

Sources: `.opencode/plans/2026-08-06-spagyric-layer1a-kv-offload-investigation.md`,
`.opencode/plans/2026-08-06-spagyric-kv-context-levers.md`, `docs/TEST_REPORT_2026-05-17.md` §2.2.

## Config

```ini
[kv]
mode = standard      ; offload (Layer 1a) recorded, untested on CC 6.1
```

## Undo

`[kv] mode = standard` (KV in VRAM) is the working path. If Layer 1a is enabled
later and the PCIe penalty hurts, revert here.
