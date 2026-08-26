# Example profiles

Generic starting points, not tuned configs. The personal production profiles
in `profiles/` are this-box diary (specific GPUs, splits, thresholds).

| profile | for | derive |
|---|---|---|
| `dual-gpu-12-8` | 12 GiB + 8 GiB pair, resident MoE | tensor-split ratio, model path |
| `single-gpu-8gib` | one 8 GiB card, small quantized model | model size budget, context |

Usage:

```bash
cp examples/dual-gpu-12-8/config ~/.vitriol/profiles/mine/config
$EDITOR ~/.vitriol/profiles/mine/config   # replace TUNE markers
vitriol config load mine
```

Every marked `# <-- TUNE` line is a value that depends on your hardware.
Server-side knobs with global effect (`--cache-ram`) are documented in
[../../docs/OPERATIONS.md](../../docs/OPERATIONS.md).
