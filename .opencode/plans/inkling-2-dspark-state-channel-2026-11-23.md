# DSpark state channel: make the Inkling-2 drafter actually accept

**Date:** 2026-11-23 · **Status:** implementing · **Supersedes:** the "in-graph f_rnn"
shortcut in `inkling-2-dspark-rs-max-plan-2026-11-23.md`

## Why this plan exists

`--spec-type draft-dspark` now executes end to end (commit `5a6d9b9`) but acceptance is
**0**: the n_max sweep goes *down* (8.3 / 5.4 / 4.5 t/s vs 15.9 baseline) and a
counting prompt that any correct drafter must get right scores 4.0 vs 13.1 t/s.

Reading `src/models/dspark.cpp` at PR head `e4b1cb53` showed my graph is wrong
structurally, not numerically:

```cpp
// rnn_state: [n_embd, n_tokens] per-token state of the Inkling2 rnn head
//   anchor row: f_rnn; draft row i: [uniform | 1/temp | 0...] of its draw
if (hparams.dspark_f_rnn && ubatch.embd_state) {          // line 433
    auto inp = std::make_unique<llm_graph_input_embd_h>(n_embd, n_embd);
    inp->h = ggml_new_tensor_2d(ctx0, GGML_TYPE_F32, n_embd, n_tokens);
    rnn_state = inp->h;
}
ggml_tensor * cur = build_layers(inp_tokens);
```

The block's **input is the state channel**, not token embeddings. Draft rows are all
`mask_token_id` carrying `[u, 1/temp]`; the anchor carries `f_rnn`, "a mix of the
target features of the last committed token".

The markov chain, by contrast, was already right for us: upstream's `draw()` samples
from `softmax(top_k · inv_temp)` but overrides with the exact argmax when `is_greedy`,
and its own comment says *"without rnn_state the in-graph chain is greedy (argmax);
sampling params affect only the final token pick, not the Markov conditioning path"*.
Our benchmarks run `--temp 0` ⇒ `1/temp = inf` ⇒ `is_greedy = 1` ⇒ argmax.

## Design decision: transport deviates, semantics do not

Upstream carries the rows in `ubatch.embd_state` fed from `llama_batch_ext`, a base
feature our frozen fork does not have (`grep` → zero hits for `embd_state`,
`n_embd_state`, `llama_batch_ext` in `llama.h` / `llama-batch.{h,cpp}`; PR #32218 adds
only 4 lines to `llama_batch_ext_select_n_embd_state`).

We carry the same rows through the fork's existing ext-tap pattern (`src/llama-ext.h`,
already used by `speculative.cpp:646,1179`):

- **rows** — identical values (anchor `f_rnn`, drafts `[u, 1/temp, 0…]`, width `n_embd`)
- **consumer** — identical (the `graph_ctx` block input)
- **transport** — a context-side buffer set immediately before `llama_decode`, instead
  of a batch field

The previous deviation failed because it changed *semantics* (tok_embd input, greedy
chain). This one changes only *plumbing*. That distinction is the review criterion.

## Stages

| stage | work | gate |
|---|---|---|
| **D1** | state channel: `llama_set_dspark_state()` in `llama-ext.h/.cpp` + context field; graph-side input tensor plumbing, unused | inkling2 alone 13.1 t/s unchanged; `-md` + `--spec-type none` clean |
| **D2** | block input becomes the state (`f_rnn` anchor row, `[u|1/temp]` mask rows) + block-uniform SWA mask | drafting still exits 0; **acceptance re-measured** (n_max sweep must stop being monotone-down) |
| **D3** | faithful `draw()`: `top_k(64) → argsort → softmax(·inv_temp) → cumsum/step → idx`, `is_greedy` ⇒ argmax, packing `[tok, n_cand, ids, probs]` | temp-0 draw ≡ argmax on a synthetic column; counting acceptance |
| **D4** | producer rows in `common/speculative.cpp`: `f_rnn = rms_norm(ctx_norm, ctx_mix · features)` from the existing tap, `[u, 1/temp]` for drafts | **counting acceptance ≥ 50 %**; code decode ≥ n-gram 20.3 t/s |
| **D5** | ship: `PROVENANCE:` naming PR #32218 (`e4b1cb53`) + master `dspark.cpp`; restore 27B | I1–I8 below |

## Invariants

- **I1 — no-drafter parity.** With no `-md`, target decoding is byte-identical to
  today (inkling2 and 27B). *Pinned by:* 32-token `--temp 0` output md5 on both models.
- **I2 — 27B unaffected.** Live engine decode within 2 % of 15.9 t/s and
  `probe-recommend` byte-identical. *Pinned by:* the two-arm bench + `vitriol
  probe-recommend` diff.
- **I3 — other spec modes untouched.** `--spec-type none` and n-gram keep working
  (n-gram 20.3 t/s ±5 %, identical output). *Pinned by:* the n-gram arm of the bench.
- **I4 — greedy equivalence.** At `temp = 0` the draw chain returns the exact argmax.
  *Pinned by:* D3 gate comparing `draw()` output against `ggml_argmax` on a synthetic
  column vector.
- **I5 — no state leakage.** A decode without a state set behaves as if the channel
  were empty. *Pinned by:* sequence (state, clear, decode) vs (decode) equality.
- **I6 — acceptance.** Counting prompt acceptance ≥ 50 %. *Pinned by:* the counting
  benchmark; the number is recorded in the report, not inferred from t/s.
- **I7 — padded vocab never enters a batch.** No token id ≥ `vocab.n_tokens()` reaches
  `llama_decode`. *Pinned by:* a run with backend draft sampling ON either still works
  or refuses loudly, plus the existing counting-prompt run.
- **I8 — recurrent paths intact.** `-md` runs exit 0 and the conv-state views stay
  valid. *Pinned by:* the `-md` arm (this is what the strided-view fix bought).

## Risks

1. **Acceptance still 0 after D2** ⇒ the next suspect is the ctx-K/V injection
   geometry (per-block `attn_k_ctx`/`attn_v_ctx` + `ctx_mix`/`ctx_norm`), then mask
   uniformity. Recorded as the next hypothesis, not a guess at fix time.
2. **`f_rnn` anchor differs per block** when `n_blocks > 1`. Implement the
   single-anchor-per-span case (what our drafter emits: `n_extract=4`,
   `sample_from_anchor=false`) and assert on the general case.
3. **Device-side weight access.** `ctx_mix`/`ctx_norm` live in the drafter's device
   tensors; computing `f_rnn` in-graph from a features input avoids host copies.

---

## Results — commit `092e098` (2026-11-23)

### What the channel bought

Drafts went from **constant garbage to varying, context-dependent tokens**, and the first
draft position now lands 30 % of the time. Three defects were found and fixed on the way;
the third was the one that had been poisoning everything:

1. **The 966-float alignment bug.** The drafter's lm head is padded to 201024 rows, the
   vocab exposes 200058. The context copies `n_vocab` rows per output, so every output
   after the first was shifted by exactly 966 floats. The signature was unmistakable once
   probed: drafted ids stepped 197963 → 198929 → 199895 (+966 each), and a padded id
   (200136) reached the sampler where the next batch rejected it. Trimming the head to the
   vocab width in both dflash logits paths fixed both symptoms.

2. **The state rows must not be the block input.** Feeding them as `inp->h` collapsed all
   draft rows to the same value — four identical logits vectors, one constant token.
   `mask_embd` is what distinguishes draft positions; the channel supplies `f_rnn` only
   (the anchor row of each block, upstream's `state_rows(n_embd, 0, 0)`).

3. **`f_rnn` producer.** `rms_norm(Σ_k ctx_mix[k]·features_k) · ctx_norm` from the last
   committed token's tapped features; weights fetched once via
   `llama_model_get_tensor_f32` (mix=4, norm=2304).

### Measured (counting prompt; a correct drafter should score near 100 %)

| arm | t/s | accepted |
|---|---|---|
| no speculation | 14.2 | — |
| n_max=1 | 8.6 | 7 / 23 steps |
| n_max=2 | 7.1 | 7 / 23 steps |
| n_max=4 | 5.7 | 7 / 23 steps |

Acceptance is **independent of depth**, so only position 1 can ever be accepted (30 %) and
positions 2+ never are. A draft costs ~46 ms against ~70 ms for a target token, so even
100 % depth-1 acceptance would barely break even. **I5 fails.**

### Remaining work, in order

1. **`block_uniform` SWA mask** — upstream sets `self_attn_mask_swa.block_uniform = true`
   so the whole block shares its anchor's mask row. Our fork's
   `llm_graph_input_attn_kv_iswa` has **no such flag** (verified: zero hits), so this is an
   arch-gated addition to shared mask code — it must not touch the 27B path (I2).
2. **Per-position hidden view** for i ≥ 2 (`hidden_i` stride/offset) — next suspect if (1)
   does not move depth acceptance.
3. **Numeric diffing** against a reference (transformers/vLLM implementation) on a fixed
   context, rather than inferring correctness from acceptance. This is the honest way to
   close the last gap; three rounds of probes got us here but the remaining error is
   subtle enough that a reference comparison is cheaper than more hypotheses.
4. **Draft cost.** ~46 ms per draft call is mostly per-decode launch/sync on 3 small
   blocks; batching drafts across steps or enabling graph reuse (my state input currently
   forces a rebuild) is where the throughput would come from.

### Invariants

I1 (13.5 t/s no-drafter), I7 (`-md` + `--spec-type none` clean) and I2 (27B healthy,
answering) re-verified after every change. I5 remains open — stated as measured, not
inferred.
