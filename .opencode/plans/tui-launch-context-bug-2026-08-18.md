# TUI Launch Bug: "Cannot create context" for Qwen3.8-27B

Date: 2026-08-18 20:45
Status: **FIXED 2026-08-18 21:05**

## Symptom

Launching from the TUI (via `scripts/launch_vitriol_full.sh`) fails to boot the
Qwen3.8-27B model: **"cannot create the context"**. `vitriol serve` on the same
config works fine.

## Root cause (two layers)

1. **Missing flags** — the launch script's gen-server command omitted the flags
   Qwen3.8-27B requires:
   - `-ts 24,12 --main-gpu 0` (tensor split — no split otherwise)
   - `--cache-type-k q4_0 --cache-type-v q4_0` (default f16 KV OOMs at 131K)
   - `--spec-type mtp --spec-draft-n-max 5` (MTP head)
   The `[kv] quant_mode` env `VITRIOL_KV_QUANT` does NOT apply (known — must pass
   `--cache-type-k/v` explicitly).

2. **Missing `-ub`** — default ubatch 512 makes the MTP context's pp (prefill)
   compute buffer need 505 MiB on device 1 (GTX 1070 Ti) → `cudaMalloc OOM` →
   `failed to allocate compute pp buffers` → "cannot create context". With
   `-ub 128` (the tuned optimum) the pp buffer fits.

## Confirmed (before fix)

- `vitriol serve` (passes all flags from config, `-ub` default 128-ish) works.
- Manual test without `--ubatch-size` reproduces the exact OOM (505 MiB on device 1).
- Manual test with `--ubatch-size 256` (or `-ub 128`) works.

## Fix (applied 2026-08-18 21:05)

`scripts/launch_vitriol_full.sh`:
- Resolve `[gpu] tensor_split`, `[kv] quant_mode/quant_mode_v`, `[spec]
  type/draft_n_max`, `[model] ubatch` (default 128) from `~/.vitriol/config`.
- Inject `TS_ARGS` (-ts + --main-gpu 0), `KV_ARGS` (--cache-type-k/v),
  `SPEC_ARGS` (--spec-type/--spec-draft-n-max), `-ub $UBATCH` into the gen-server CMD.
- Env overrides `VITRIOL_TENSOR_SPLIT`, `VITRIOL_KV_QUANT_K/V`,
  `VITRIOL_SPEC_TYPE`, `VITRIOL_SPEC_DRAFT_N_MAX`, `VITRIOL_UBATCH` win over config.

## Verified (after fix)

- TUI launch boots Qwen3.8-27B @ 131K ctx: KV 1584/720/144 MiB, `MTP draft head
  registered (n_ubatch=128)`, server listening on 8279.
- Real completion returns "OK".
- TUI `launch_flags()` in `vitriol-tui/src/control.rs` still only forwards
  model/ngl/ctx/threads/parallel — fine, since the launch script now reads config.

