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
