# Inkling-2 DSpark drafter: port research (feasibility established)

**Date:** 2026-11-23
**Question:** can we port upstream's Inkling DSpark drafter support onto the frozen
engine fork, and how big is it really?
**Verdict: YES — tractable in one session.** My earlier "multi-day" estimate was
wrong, and the evidence below says why.

## 1. My earlier assessment was wrong on the gap

I previously reported the port as a forward-port of "a month of upstream evolution"
because PR #32218 modifies a `src/models/dspark.cpp` our fork lacks. That framing
was misleading. Measured facts:

| claim | reality |
|---|---|
| fork has no `dspark.cpp` | true — DSpark is **inline** in `dflash.cpp` (1001 lines) |
| our DSpark is a month behind | **false** — our `build_dspark_markov_head` (dflash.cpp:292) is **byte-identical to upstream master** through the anchor logic, including `g.ubatch.n_seqs_unq`, `ggml_cont_1d`, `model.gguf_kv.find("dflash.block_size")`, `sample_from_anchor`, the `d2t` path |
| PR is large | **690 additions / 49 deletions across 16 files** (GitHub API), of which only 5 files matter to the engine |
| master cannot be used | true — but irrelevant: we do not need master, only two *small* deltas |

The single real delta in the markov head is master's `is_rnn` path (a `hidden`
parameter + a 3-chunk `joint_proj` GRU-style state update), plus PR #32218's
`f_rnn` extension of it.

## 2. The infrastructure I thought was missing already exists

This was the decisive finding. Our fork already has the **complete generic
target-feature tap mechanism**:

- `src/llama-ext.h:126-129` — `llama_model_target_layer_ids()` / `_n()`
- `src/llama-context.cpp` — `llama_set_embeddings_layer_inp(ctx_tgt, layer, true)` to
  switch extraction on, `llama_get_embeddings_layer_inp(ctx_tgt, layer)` to read back
- `src/models/dflash.cpp:27` — `hparams.n_embd_inp_enc_impl = target_layer_ids.size() * n_embd`,
  i.e. **the draft's input is already defined as concatenated target-layer features**
- `common/speculative.cpp:993-999` — the `draft-dspark` impl already resolves
  `target_layer_ids` from the **draft** model, computes `n_embd_enc = n * n_embd_tgt`,
  and interleaves the tapped features into the draft batch

So the target (`inkling2`) needs **no model-specific code at all**. The spec
machinery drives it generically. The port is confined to *the draft model's own
architecture*.

## 3. Every graph API the upstream code needs is present in our fork

Checked individually against our tree (not assumed):

| API | where |
|---|---|
| `g.ubatch.n_seqs_unq` | `llama-batch.{h,cpp}`, `llama-graph.cpp` |
| `g.build_lora_mm` | `llama-graph.{h,cpp}` |
| `ggml_set_rows` | `llama-graph.cpp`, `llama-kv-cache.h`, ggml |
| `ggml_cont_1d` | already used **in our own `dflash.cpp`** |
| `model.gguf_kv` | `llama-adapter.{h,cpp}`, `llama-impl.cpp` |
| `dspark_markov_w2_s` | **already in our `dflash.cpp` + `llama-model.h`** |
| `llama_set_embeddings_layer_inp` | `llama-ext.h`, `llama-context.cpp` |

## 4. The one genuinely new plumbing layer, and a fork-native way around it

PR #32218 adds `ubatch.embd_state` / `n_embd_state` to carry the *last committed
token's* target features into the graph as `f_rnn`. Our fork lacks this (it exists
only inside `llama-kv-cache-dsv4`), and `llama-batch.cpp` has 735 lines of drift —
so importing that machinery is the most invasive part of the PR.

**Fork-native alternative (this is the plan's key design decision):** the graph
already receives the full target-feature tensor as `inp->embd`
(`build_inp_embd_enc`, dflash.cpp:259, `n_embd_inp = target_layer_ids.size()*n_embd`
= 4·2304 = 9216). The anchor row of each block *is* the last committed token, so
`f_rnn` can be computed **in-graph** from the anchor's own feature rows:

```
ctx = view(inp->embd @ anchor_row)      // [4*n_embd, n_blocks]
ctx = sum_k markov_ctx_mix[k] * ctx_k   // [n_embd, n_blocks]
f_rnn = rms_norm(ctx, markov_ctx_norm)  // [n_embd, n_blocks]
```

This is the same operation the per-block `ctx_mix`/`ctx_norm` already performs, so
it reuses one helper. It removes `llama-batch.cpp` from the change set entirely.
Risk: if the anchor row's feature rows are not populated in every draft batch the
result degrades (not crashes) — pinned by a test.

## 5. The drafter's actual architecture, from the GGUF

`dspark-inkling-mainline.gguf`, 58 tensors, `general.architecture=dflash`:

**Per block ×3:** `attn_norm[2304]`, `attn_q[2304,3072]`, `attn_k[2304,1024]`,
`attn_v[2304,1024]`, `attn_q_norm[128]`, `attn_k_norm[128]`,
**`attn_k_ctx[2304,1024]`**, **`attn_v_ctx[2304,1024]`**, **`attn_k_ctx_norm[128]`**,
**`ctx_mix[4]`**, **`ctx_norm[2304]`**, `attn_output[3072,2304]`,
`ffn_norm[2304]`, `ffn_gate[2304,9216]`, `ffn_up[2304,9216]`, `ffn_down[9216,2304]`

**Top level:** `joint_proj[5120,768]`+`bias[768]`, `markov_ctx_mix[4]`,
`markov_ctx_norm[2304]`, `markov_w1[256,200058]`, `markov_w2[256,200058]`+`bias[200058]`,
`mask_embd[2304,7]`, `token_embd_norm[2304]`, `output_norm[2304]`.
No `token_embd`, no `output` — both borrowed from the target (dflash already creates
`TOKEN_EMBD` with `TENSOR_NOT_REQUIRED`).

**Shapes are self-consistent and prove the `f_rnn` variant:** `joint_proj` input
`5120 = 2·2304 + 2·256 = 2·n_embd + 2·rank`; output `768 = 3·rank`;
`mask_embd [2304, 7] = [n_embd, block_size-1]` with `block_size = 8`;
`markov_ctx_mix [4] = |target_layer_ids|`.

**The design, read off the shapes:** each block mixes the 4 tapped target features
with `ctx_mix` → normalises with `ctx_norm` → projects to *context K/V*
(`attn_k_ctx`/`attn_v_ctx`, 8 KV heads × 128) which are concatenated to the block's
own K/V. So the drafter attends over [target context ; its own drafts] — it borrows
the target's representation without copying it.

## 6. Remaining risks (named, not hidden)

1. **R1 — loop-expanded tap indices.** `target_layers = [5,23,41,48]` but inkling2
   has 24 *physical* layers / 47 cache slots. `llama_set_embeddings_layer_inp`
   aborts when `id > n_layer`, and has a separate branch for `id == n_layer` (the
   post-final-norm tap). `5/23/41 < 47` and `48` plausibly equals the final tap — the
   index space matches the loop-expanded enumeration, which is what the hook uses.
   *Verified only by reading; must be proven by stage T1's dump.*
2. **R2 — `attn_k_ctx` attention layout.** Whether the ctx K/V are concatenated
   along the sequence axis (one shared context prefix) or per-position is inferred
   from shapes, not from source. Stage T2 pins it by checking output finiteness and
   then correctness.
3. **R3 — `mask_embd` usage** is gated upstream on `use_iswa` (`hparams.dspark_f_rnn`
   branch at PR line 210); our fork's iswa plumbing differs.
4. **R4 — zero acceptance looks like success.** A wrong graph still loads and still
   emits tokens; only acceptance rate exposes it. Stage T4's gate is mandatory.
5. **R5 — drafter VRAM.** 0.50 GB drafter + q4_0 draft KV on a card at 10.0/11.5 GiB.

## 7. Proposed plan (for confirmation)

| stage | work | gate |
|---|---|---|
| **T1** | `llama-arch.{h,cpp}`: 9 tensor enums + names + infos. `llama-hparams.h`: `dspark_f_rnn`. `llama-model.h`: `dspark_markov_ctx_mix/norm`, `dspark_joint_proj(_b)`, `dspark_mask_embd`, per-layer `ctx_*`/`attn_*_ctx`. Load them in `dflash.cpp` under a variant branch. | drafter **loads**, 58/58 tensors, no `check_tensor_dims` error |
| **T2** | Prove the taps: enable `llama_set_embeddings_layer_inp` for [5,23,41,48] and dump each tapped tensor's `ne[]` through the draft context. | 4 taps materialise as `[n_embd, n_tokens]` at `-c 256` and `-c 8192`; the `48` case resolves to the final tap |
| **T3** | Graph: per-block `ctx_mix`/`ctx_norm` → `attn_k_ctx`/`attn_v_ctx` + `attn_k_ctx_norm`, concatenated K/V; `mask_embd` per block position; `joint_proj` widened to `2·n_embd+2·rank` with in-graph `f_rnn`; markov state chain from master. | forward runs, logits finite, `1 2 3 4 5`-style prompt not degenerate |
| **T4** | Measure with `--spec-type draft-dspark --spec-draft-n-max 2 -ngd 99`, draft KV q4_0 vs f16. | **acceptance ≥ 50 %** on a fixed prompt, else the port is wrong; then decode t/s + fingerprint |
| **T5** | Commit with `PROVENANCE:` naming PR #32218 and master's `dspark.cpp`; restore the 27B engine. | engine `active`, `/health ok`, live completion OK |

Expected change size: ~350-550 lines net across
`llama-arch.{h,cpp}`, `llama-hparams.h`, `llama-model.h`, `src/models/dflash.cpp`.
**`llama-batch.cpp` is not touched.**

## 8. Invariants (preview; each gets its pinning test in the plan file)

- **I1** 27B path byte-identical: with no drafter, `probe-recommend` and a 27B bench
  match the current baseline. *(pin: bench signature + probe-recommend diff)*
- **I2** DSpark code is unreachable for non-`dflash` arches. *(pin: a load test for
  27B + inkling2 asserting no new tensors are created)*
- **I3** Inkling-2 target numerics unchanged: `The capital of France is` produces
  the same first N tokens as `9ce106a`. *(pin: greedy text md5)*
- **I4** Drafter acceptance must be measured, never assumed; a near-zero rate fails
  the stage. *(pin: T4 gate asserts acceptance ≥ 50 %)*
- **I5** Every measured result carries its `VITRIOL-FINGERPRINT:` including draft KV type.
- **I6** `mix()` S3 result preserved: prefill ≥ 153 t/s at 400 tokens. *(pin: bench)*

Awaiting owner confirmation before any code is written.
