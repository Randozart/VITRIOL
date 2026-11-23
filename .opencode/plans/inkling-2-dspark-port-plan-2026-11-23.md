# Inkling-2 DSpark drafter: forward-port plan

**Date:** 2026-11-23
**Status:** READY TO EXECUTE (needs authorisation — this is a model-support addition
to the FROZEN engine, resolved upstream by open PR #32218)
**Predecessor:** `inkling-2-dspark-2026-11-22.md` (S0 proved the drafter cannot load)

## Why there is no shortcut

Exhausted and ruled out by measurement/inspection:

| shortcut | why it fails |
|---|---|
| Just add `--spec-type draft-dspark` flags | drafter arch ≠ our `dflash` (86 missing tensors) |
| Rename/map tensors | different *model* (MLA+MoE+hc vs plain attn+dense FFN) |
| `draft-simple` (plain autoregressive) | `draft-simple` feeds **tokens**; the drafter has no `token_embd` — it consumes **target features** |
| n-gram speculation instead | **DELIVERED anyway**: +27.7 % on code, no drafter needed |

## What exists in our fork vs what is needed

Our fork has DSpark *inline* in `src/models/dflash.cpp:292`
(`build_dspark_markov_head(g, model, tokens)` — 3 params). Upstream has since split it
into `src/models/dspark.cpp` and added a 4th `hidden` param, then built #32218 on top.
So the port is: **bring the drafter feature path forward**, not cherry-pick.

### Tensors to add (7 new + 2 upstream ones we lack)

| enum | name | shapes from the real drafter |
|---|---|---|
| `DSPARK_JOINT_PROJ` | `joint_proj` | w `[2*n_embd + 2*rank, 3*rank]` = `[5120, 768]` ✓, b `[768]` |
| `DSPARK_MASK_EMBD` | `mask_embd` | `[n_embd, block_size-1]` = `[2304, 7]` ✓ |
| `DSPARK_MARKOV_CTX_MIX` | `markov_ctx_mix` | `[len(target_layer_ids)]` = `[4]` ✓ |
| `DSPARK_MARKOV_CTX_NORM` | `markov_ctx_norm` | `[n_embd]` ✓ |
| `DFLASH_CTX_MIX` | `blk.%d.ctx_mix` | `[4]` ✓ |
| `DFLASH_CTX_NORM` | `blk.%d.ctx_norm` | `[2304]` ✓ |
| `DFLASH_ATTN_K_CTX` | `blk.%d.attn_k_ctx` | `[2304, 1024]` ✓ |
| `DFLASH_ATTN_V_CTX` | `blk.%d.attn_v_ctx` | `[2304, 1024]` ✓ |
| `DFLASH_ATTN_K_CTX_NORM` | `blk.%d.attn_k_ctx_norm` | `[128]` ✓ |

Every shape is confirmed against `~/Models/inkling-2-mini/dspark-inkling-mainline.gguf`.
`joint_proj = 2*n_embd + 2*rank` is what proves this is the **`f_rnn` variant**.

### Hparams / fields

- `llama_hparams::dspark_f_rnn` (bool) — read `dflash.has_markov_context`.
- `llama_layer`: `dflash_ctx_mix`, `dflash_ctx_norm`, `dflash_wk_ctx`, `dflash_wv_ctx`,
  `dflash_attn_k_ctx_norm`.
- `llama_model`: `dspark_markov_w2_b`, `dspark_markov_ctx_mix`, `dspark_markov_ctx_norm`,
  `dspark_joint_proj`(+`_b`), `dspark_mask_embd`.
- `llama-batch.cpp:llama_batch_ext_select_n_embd_state` — return `hparams.n_embd` when
  `dspark_f_rnn` (the rnn head reads `f_rnn` from `embd_state`).

## Stages

- **P1 — load.** Register the 9 tensors, hparams, fields; create them in the loader.
  *Gate:* `llama-cli -m dspark-inkling-mainline.gguf` loads with no
  `check_tensor_dims` error and reports 58/58 tensors. No GPU work needed.
- **P2 — target feature taps.** Wire `target_layer_ids` `[5,23,41,48]` through the
  target's graph. **This is the delicate stage**: 41 and 48 are *loop-expanded*
  indices in a 24-physical / 47-slot model — the same index space that produced the
  `n_layer_phys()` placement bug. Verify by dumping the tapped tensor's `ne[]` and
  checking it equals `[n_embd, n_tokens]` for each of the 4 ids.
  *Gate:* 4 tap tensors materialise with correct shapes at both `-c 256` and `-c 8192`.
- **P3 — block graph.** Per block: `ctx_norm`+`ctx_mix` mix the 4 tapped features →
  `attn_k_ctx`/`attn_v_ctx` produce context K/V; plain `attn_q/attn_k/attn_v` with
  `attn_q_norm`/`attn_k_norm`; rope (dim 128, base 1e6); sliding-window pattern
  `[T,F,T]`, `causal=false`; dense FFN 9216.
  *Gate:* forward pass runs; logits finite; `mask_embd` selected per block position.
- **P4 — markov head.** `joint_proj` on `cat(state, w1_prev, hidden_i, f_rnn)`,
  `markov_w1/w2`(+bias) bias, `softcap_after_bias` with softcap 30.
  *Gate:* drafted tokens are *plausible* — greedy argmax of the head equals the
  target's argmax on ≥ 50 % of positions on a fixed prompt (a broken head gives ~0 %).
- **P5 — measure.** `--spec-type draft-dspark --spec-draft-n-max 2 -ngd 99
  --draft-cache-type-k q4_0 --draft-cache-type-v q4_0` (f16 draft KV is a measured 3×
  regression). Report acceptance rate, decode t/s, and text diff vs no-spec.

## Invariants

- **P-I1** 27B + live engine untouched; `probe-recommend` byte-identical.
- **P-I2** with `--spec-type` absent, behaviour is byte-identical to today
  (drafter code is additive and only reachable for arch `dflash`).
- **P-I3** drafter loading must not change the *target* model's tensor set — verify
  the target still reports its original tensor count.
- **P-I4** acceptance rate must be measured, not assumed; a near-zero rate means the
  port is wrong, and the flags must not ship.
- **P-I5** every stage carries the `VITRIOL-FINGERPRINT:` line including draft KV type.
- **P-I6** `PROVENANCE:` note in `src/models/dspark.cpp` naming upstream PR #32218.

## Failure modes

1. **P2 is the risk.** If loop-expanded taps cannot be expressed, the drafter is
   unusable regardless of P3–P4 — stop there and report.
2. A drafter that loads but drafts garbage scores **0 % acceptance**, which is slower
   than no speculation. P4's ≥50 % argmax gate exists to catch this before shipping.
3. 0.50 GB drafter + draft KV on top of 10.8/10.1 GiB — watch `memory.free`.

## Already delivered in this session (no drafter required)

| change | effect | commit |
|---|---|---|
| `mix()` operand swap (#32855) | prefill **+8.0 %**, decode flat | `dbe6b89` |
| `ngram-simple,ngram-map-k` n_max 4 | decode **+27.7 %** on code (+6.8 % on long output), textually lossless at short lengths | (flags only) |

Net on short code output: **15.1 → 20.3 t/s (+34 %)**.
