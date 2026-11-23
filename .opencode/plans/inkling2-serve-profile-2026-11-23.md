# Inkling 2 Mini on the live engine — profile + measured envelope (2026-11-23 17:4x)

**What shipped:** `~/.vitriol/profiles/inkling2-mini-200k` loaded into `~/.vitriol/config`,
engine restarted. Backup of the previous config: `~/.vitriol/config.bak-pre-inkling`.

```
VITRIOL-FINGERPRINT model=Inkling-2-Mini-IQ4_NL.gguf ts=1,1 c=204800 kv=f16/f16 fa=on
                    ub=32 mode=off engine=native spec=none:0 par=1 cram=2048 cxr=1
```

**Restore the 27B:** `vitriol config load qwen38-buffedmod-262k-cache8-mtp2 && systemctl restart vitriol-server.service`.

## Sizing (measured, not estimated)

| quantity | value |
|---|---|
| model file | 17.6 GB IQ4_NL (~31B total params, ~1/16 active per token) |
| `27,21` split | **fails to load** — dev0 has no room for prefill buffers |
| `1,1` + ub 64 | fails (3.0 GB cudaMalloc on dev0) |
| `1,1` + ub 32 | **loads** — the shipped point |
| pp buffer | scales as ub × ctx (3031 MB @ ub64/c200k → 1551 MB @ ub32) — the dense relative-bias materialisation |

KV is not the constraint: 20 SWA layers (1024 window) + 4 full layers ≈ 3.3 GB at 200k.

## Measured envelope

| prompt depth | prefill | decode |
|---|---|---|
| 31 tok | 66 t/s | 16.2 t/s |
| 3,863 tok | **19.7 t/s** | **5.5 t/s** (3m20s for the turn) |

This is the known depth wall (dense relative-position bias materialisation, needs the
upstream #32808-style kv_rows per-head mask). It is a *depth* effect, not a
window-allocation effect, so no profile variant avoids it — 16k would crawl too.

## Model-protocol notes

- The GGUF carries a channel template (`<|message_user|>`, `<|content_thinking|>`,
  `<|content_text|>`, `<|end_message|>`); llama-server applies it automatically (props
  reports the 6867-char template).
- Thinking is on by default and lands in `reasoning_content` (template advertises
  reasoning preservation; the launcher passes `--no-reasoning-preserve`).
- `<|end_message|>` (id 200010, CONTROL) is **not** an EOG id. Consequence: the final
  answer text ends with a literal `<|end_message|>`.
  - Registering it in the EOG table (`llama-vocab.cpp`) is **wrong**: it closes the
    *thinking* segment, so generation stops before the answer (measured: empty content).
    Tried and reverted.
  - A per-request stop on it does the same thing (measured: empty content).
  - Correct fix is channel-aware output parsing that treats it as a message separator.
    Not attempted; cosmetic only.
