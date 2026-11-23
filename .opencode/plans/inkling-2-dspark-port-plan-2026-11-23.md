# Inkling-2 DSpark drafter: port plan

**Date:** 2026-11-23
**Status:** APPROVED (owner confirmed; in-graph `f_rnn` authorised)
**Research:** `inkling-2-dspark-port-research-2026-11-23.md`
**Upstream reference:** PR #32218 (`feat: Inkling 2 DSpark support`, 690+/49-,
16 files) layered on master's `src/models/dspark.cpp`.

## Goal

Load and run `dspark-inkling-mainline.gguf` as a speculative draft model for
Inkling-2-Mini on the frozen engine fork, then measure whether it beats plain
n-gram speculation.

## What the port actually is

Our `build_dspark_markov_head` (`dflash.cpp:292`) is byte-identical to upstream
master through the anchor logic, so this is **not** a forward-port of a month of
work. Two small deltas are missing:

1. master's `is_rnn` path — a `hidden` parameter and a 3-chunk `joint_proj`
   GRU-style state carried across block positions;
2. PR #32218's `f_rnn` extension — `joint_proj` widened to
   `2·n_embd + 2·rank` with a **second** hidden stream mixed from the target features
   by `markov_ctx_mix` / `markov_ctx_norm`.

Plus a **new backbone** in the `dflash` family: the Inkling drafter is 3 plain
attention blocks that attend over *context K/V projected from the target features*
(`attn_k_ctx` / `attn_v_ctx` / `attn_k_ctx_norm`) concatenated to their own K/V.
It has **no `fc`** and **no `output_norm_enc`** (the ctx mix replaces the feature
fusion), and it borrows `token_embd` / `output` from the target via `ctx_other`
(already supported).

## Design

### Detection

`hparams.dspark_f_rnn = (ml->get_tensor_meta("joint_proj.weight") != nullptr)`.
Presence of `joint_proj` uniquely identifies the f_rnn/ctx variant; every other
dflash/dspark build lacks it. No GGUF key is needed.

### In-graph `f_rnn` (fork-native deviation, owner-authorised)

Upstream threads the last committed token's features through
`ubatch.embd_state` / `n_embd_state` in `llama-batch.cpp` (735 lines of drift —
the most invasive part of the PR). We avoid it: the graph already receives the
target features as `inp->embd [4·n_embd, n_tokens]`, and each block's anchor row is
the last committed token. `f_rnn` is therefore computed in-graph from the anchor's
own feature rows with the same helper the per-block ctx mix uses.
**`llama-batch.cpp` is not touched.**

### Graph (`graph_ctx`, new, in `dflash.cpp`)

```
inp : [4·n_embd, n_tokens]                       (already the draft input)
mix_ctx(inp, mix[w], norm[w]) -> [n_embd, n_tokens]
    = rms_norm( Σ_k mix[k] · view_k(inp), norm )
h   = mix_ctx(inp, blk0.ctx_mix, blk0.ctx_norm)  // block 0 input
per block b in 0..2:
    x      = rms(attn_norm_b) @ h
    q      = rope(q_norm(wq_b(x)))                [3072]
    k_self = rope(k_norm(wk_b(x)))                [1024]
    v_self = wv_b(x)                              [1024]
    ctx_b  = mix_ctx(inp, blk_b.ctx_mix, blk_b.ctx_norm)
    k_ctx  = k_ctx_norm(wk_ctx_b(ctx_b))          [1024]
    v_ctx  = wv_ctx_b(ctx_b)                      [1024]
    attn   = softmax(q · [k_ctx ; k_self]ᵀ) · [v_ctx ; v_self]   // seq axis concat
    h      = h + wo_b(attn)
    h      = h + ffn_b(rms(ffn_norm_b h))
logits = target_output(output_norm(h))            // borrowed lm_head
markov : build_dspark_markov_head(g, model, tokens, hidden, f_rnn)
```

`mask_embd [n_embd, block_size-1]` replaces the input embedding of masked draft
slots (positions 1..block_size-1 of each block).

## Stages

| stage | delivers | gate (must pass to proceed) |
|---|---|---|
| **T1 load** | 9 enums + names + infos; `dspark_f_rnn`; model/layer fields; `fc`-optional; ctx tensor creation | drafter loads: `llama-cli -m dspark-inkling-mainline.gguf` reports all 58 tensors, no `check_tensor_dims` error |
| **T2 taps** | prove `target_layers [5,23,41,48]` resolve | 4 taps materialise as `[n_embd, n_tokens]`; the `48` id resolves to the final tap, not an abort |
| **T3 graph** | `graph_ctx` + is_rnn/f_rnn markov head + `mask_embd` | forward runs, logits finite, greedy prompt not degenerate |
| **T4 measure** | `draft-dspark` end-to-end | **acceptance ≥ 50 %** on a fixed prompt; else the graph is wrong → iterate T3 |
| **T5 ship** | PROVENANCE commit, engine restored | 27B `active`, `/health ok`, live completion OK |

## Invariants

| # | property | pinned by |
|---|---|---|
| **I1** | The 27B path is unchanged: decode and prefill equal the current baseline, and `probe-recommend` output is byte-identical. | `bench_27b_unaffected` (bench signature vs `49c3660b7b9b`-era numbers) + `probe_recommend_golden` |
| **I2** | DSpark code is unreachable for non-`dflash` arches; no new tensors are created for 27B or inkling2. | `load_27b_no_dspark_tensors`, `load_inkling2_no_dspark_tensors` |
| **I3** | Inkling-2 target numerics unchanged: greedy output for `The capital of France is` matches the `9ce106a` reference. | `inkling2_greedy_golden` (text md5) |
| **I4** | A drafter that loads but drafts badly must fail the stage, not pass silently. | `dspark_acceptance_floor` asserts acceptance ≥ 50 % |
| **I5** | Every measured number carries a `VITRIOL-FINGERPRINT:` line including draft KV type. | `fingerprint_present` (grep of the run log) |
| **I6** | S3's prefill win is preserved: ≥ 153 t/s at 400 tokens. | `inkling2_prefill_floor` |
| **I7** | With `--spec-type` absent, Inkling-2 output is identical whether or not the drafter file exists on disk. | `no_spec_type_identical` (greedy text md5, with drafter present but unused) |

## Named tests

Engine absence of a unit-test harness means these are **scripted gates** recorded
in the stage logs; each is a command whose failure blocks the stage:

- `load_drafter` — `llama-cli -m dspark-inkling-mainline.gguf -ngl 0 -n 1 -p x`;
  asserts `n_tensors == 58` and no `check_tensor_dims`/`not found`.
- `load_27b_no_dspark_tensors` / `load_inkling2_no_dspark_tensors` — load and
  assert the tensor count equals the pre-change count (no new tensors created).
- `inkling2_greedy_golden` — `-p 'The capital of France is' -n 16 --temp 0`,
  md5 of the generated text vs the `9ce106a` reference.
- `dspark_acceptance_floor` — parse acceptance rate from the `draft-dspark` run
  log; fail under 50 %.
- `bench_27b_unaffected`, `probe_recommend_golden`, `inkling2_prefill_floor`,
  `fingerprint_present`, `no_spec_type_identical` — as named above.

## Risks

- **R1** `target_layers = [5,23,41,48]` live in the loop-expanded index space
  (24 physical / 47 slots). `48` should hit the post-final tap branch. Proven by T2.
- **R2** the ctx-K/V concatenation axis is inferred from tensor shapes, not read
  from upstream source. If T4 acceptance is near zero, this is the first knob.
- **R3** `mask_embd` and rope/sliding-window interaction on 3 plain blocks.
- **R4** drafter VRAM: 0.50 GB + draft KV on a card already at ~10.0/11.5 GiB.

## Change set

`src/llama-arch.h`, `src/llama-arch.cpp`, `src/llama-hparams.h`, `src/llama-model.h`,
`src/models/dflash.cpp`. Nothing else. `llama-batch.cpp` explicitly excluded.

---

## Execution results (2026-11-23)

Commit `7b4356e`. What landed, what broke, and exactly what remains.

### T1 — DONE, verified

The drafter **loads**: all 58 tensors, zero `check_tensor_dims` / `not found`
errors, advancing to the expected `dflash requires ctx_other to be set`. Before this
commit the load died at `fc.weight not found`.

Implemented: 10 tensor enums (9 + a dedicated global `token_embd_norm` alias) with
names/infos; `hparams.dspark_f_rnn` detected from `joint_proj.weight` presence;
`llama_layer` ctx fields; `llama_model` f_rnn fields; `fc`/`output_norm_enc` optional
for the variant; `mask_embd` shaped from the file's own tensor meta.

### T3 — written, not yet reached end-to-end

`graph_ctx<is_enc>`: the per-block feature mix (`ctx_mix`/`ctx_norm`) replaces `fc`;
context K/V are cached during the embd pass and the mask block attends over them;
the markov rnn head gained master's `is_rnn` state chain plus `f_rnn`.

Three bugs found and fixed while bringing it up:

1. **`ggml_mul` operand order** — `ggml_mul(a,b)` requires `b` repeatable into `a`;
   the mask broadcast had them reversed (`ggml_can_repeat` abort).
2. **Mask path on non-block batches** — a plain sequence batch (context warmup)
   was getting mask embeddings with out-of-range slot indices; now gated to
   `rows_per_block - 1 <= mask_embd->ne[1]`.
3. **Padded-vocab mismatch** — the markov bias has **200058** rows (unpadded) while
   the target's logits carry **201024**; the bias is now zero-padded before the add.

The in-graph `f_rnn` design was adopted as planned: `llama-batch.cpp` untouched.

### The remaining blocker (T4) — measured, not guessed

With `--spec-type draft-dspark`, the abort is **in the target**, not the drafter:

```
common.cpp:1751  cparams.n_rs_seq = params.speculative.need_n_rs_seq()   // = n_max = 2 for DSPARK
build_conv_state_store -> ggml_view_4d  GGML_ASSERT(data_size + view_offs <= ggml_nbytes)
```

Root cause, with numbers: inkling2's `convolve()` builds
`x = [n_tokens, n_channels, n_seqs]` and passes it as `conv_x`. The
`n_rs_seq == 0` path addresses that layout correctly, but the K-slot path
(`n_rs_seq > 0`) builds

```cpp
windows = ggml_view_4d(conv_x, n_cols, Kw, channels, n_seqs,
                       elemsize, conv_x->nb[1], conv_x->nb[2], row_size(type, n - (Kw-1)));
```

which requires the **conv window on dim0**. `ggml_ssm_conv` requires **tokens on
dim0**. The two are mutually exclusive, so inkling2 cannot satisfy the K-slot path
without building a second, differently-laid-out view (or reworking the conv state
store). Measured params at the abort: `n_cols=3 K=3 channels=2304 n_seqs=1
row_size=79872 get_size=1 nbytes_s=239616 cell_offs=52224 n_rs_seq=2 s_ne=[19968,3]`.

A second, unexplained target fault appears on the alternative lever: forcing
`n_rs_seq = 0` (by removing INKLING2 from `llm_arch_supports_rs_rollback`) makes the
*target alone* segfault in `llama_kv_cache_context::build_input_k_idxs` via
`build_inp_mem_hybrid_iswa_rel`. Reverted — that lever is not usable, and the target
alone is verified healthy again (13.1 t/s).

### What T4 needs next

1. Give inkling2's conv state store a rollback-compatible path: build a
   `[n_cols, Kw, channels, n_seqs]` view from `x` for `build_conv_state_store`
   only (keep `x` in token-major form for `ggml_ssm_conv`), or restructure the
   store to read token-major and write slot-major.
2. Then re-run T4's acceptance gate (≥ 50 %).
3. Then T5: measure, and sweep draft KV q4_0 vs f16.

**Unchanged invariant status**: I1/I3 hold (inkling2 alone 13.1 t/s, no regression;
enum/table additions verified safe by bisection), I2 holds (DSpark code is reachable
only when `dspark_f_rnn`), I7 holds (`-md` present + `--spec-type none` runs clean).
