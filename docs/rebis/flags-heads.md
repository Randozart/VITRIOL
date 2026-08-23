# REBIS flags — head servers (Sol & Luna)

Every flag on the two llama-server heads, what it does, and why this value
on this rig. Full reference: `docs/REBIS_FLAGS.md`.

## Placement

- `-ngl 99` puts every layer in VRAM — both heads are fully resident; there
  is no CPU offload to manage.
- `-c 65536` gives the 64k window hermes requires. Measured KV cost at q4_0:
  ~0.7 GiB (Luna, thanks to SWA) to ~1.9 GiB (Sol).

## Cache quantization

`--cache-type-k q4_0 --cache-type-v q4_0 -fa on`

The KV cache stores per-token attention vectors. Quantizing K and V to 4-bit
shrinks it ~4× with negligible quality loss for code work. Flash Attention
(`-fa on`) is required for V quantization and speeds prefill.

Failure mode: drop `-fa` and the V setting silently stops applying upstream.

## Rolling windows

`--context-shift --cache-reuse 256`

When a prompt would exceed the window, shift drops the oldest span instead
of erroring; cache-reuse re-evaluates shifted regions in ≥256-token chunks.
Safe only since the min-LCP restore gate landed (H1). Day-long sessions
require both.

## Prompt cache RAM

`--cache-ram 2048` (Sol) / `1024` (Luna)

Bounds the semantic prompt cache that stores whole conversation states in
host RAM. The default (8192 MiB) contributed to real OOM kills on this
15 GB box. Set `0` to disable entirely.

## Checkpoints

`--ctx-checkpoints 12 --checkpoint-every-n-tokens 8192`

VITRIOL saves mid-prefill checkpoints (~150 MB each at large contexts).
Defaults (32 slots / every 2048 tokens) multiply into multi-GB host-RAM
creep across day-long sessions. Fewer + sparser bounds it.

## Endpoint surface

`--slots --metrics --jinja`

`--slots` feeds TUI progress bars; `--metrics` feeds token totals;
`--jinja` applies each model's chat template (required for Thinking models)
and enables `/apply-template`, which anticipatio warming depends on.

PROVENANCE: flags registered in llama.cpp common/arg.cpp; measurements in
EXPERIMENT_LOG.md (2026-08-21/22 entries); launcher scripts/rebis-servers.sh.
