# What enables the Inkling-2 DSpark drafter: a fork-generation gap, not a model-file bug

**Date:** 2026-11-23
**Question:** the Inkling DSpark drafter loads and its graph is written, yet
`--spec-type draft-dspark` aborts in the *target's* conv-state store. Other people
apparently run this. What is actually different?
**Verdict:** the model file is from post-Nov-5 upstream, but the **recurrent-memory
core underneath it is a pre-fix generation**. Upstream tracks rollback snapshots
**per cell** (`cell.rs_max`, head-aware `state_clear`); our fork uses a single global
`n_rs_seq`. The ported model drives a rollback path the older core accounts for
differently, so the K-slot view overruns. This is the same defect family as the open
llama.cpp issue #31656 ("cells migrate between ubatches, `rs_max` doesn't follow").

## 1. Method

Read-only: local source and diffs, plus a four-way external fan-out
(PR branch, Unsloth docs, llama.cpp tracker, other engines).

## 2. What is *not* the cause — every drafter-side surface matches upstream

I had assumed fork drift. Measured, it is not:

| surface | ours vs upstream master |
|---|---|
| `build_conv_state_store` (incl. the K-slot `windows`/`slots` views) | byte-identical |
| `inkling2::convolve()` | byte-identical |
| `llama_hparams::n_embd_r()` (Inkling branch) | byte-identical |
| `inkling2.cpp` `n_channels_max` derivation | byte-identical |
| `llm_arch_supports_rs_rollback` (INKLING2 listed) | identical |
| `common.h` `need_n_rs_seq()` (DSPARK included) | identical |
| `ggml_set_rows` asserts | identical (ours lacks only BF16) |

PR #32218 itself confirms this: it does **not** touch `build_conv_state_store`,
`convolve()`, `n_embd_r()`, `n_sconv_channels`, `n_rs_seq` or
`llm_arch_supports_rs_rollback` at all, and `src/llama-graph.cpp` is not even in its
file list. Its only engine plumbing is `llama-batch.cpp:1214-1226` (widening
`llama_batch_ext_select_n_embd_state` to `n_embd` under `hparams.dspark_f_rnn`) and
the flag at `llama-hparams.h:307`.

**I was also wrong in my last report** that the two views are "mutually exclusive".
With `ggml_view_4d(ctx, a, ne0..ne3, nb1, nb2, nb3, offset)` read correctly
(`ggml/include/ggml.h:1655-1665`), the `windows` view is an *identity*:

```
offset + data_size = 4(n−Kw+1) + [4(n_cols+Kw−1) + (C−1)·4T + (S−1)·4TC] = 4·T·C·S = nbytes
```

It can never overflow for a contiguous `conv_x`. The `slots` view fits exactly at the
widest conv site (`n_channels_max = 2·n_embd_k_gqa + 2·n_embd = 6656`,
`n_embd_r() = 6656·3 = 19968`). So no arithmetic overrun exists in the *steady state* —
something must make the accounting inconsistent first. That points at state, not shape.

## 3. The actual gap: the recurrent-memory generation

`diff` of `src/llama-memory-recurrent.cpp` against upstream master: **103 differing
lines**; the header differs by **14**. Ours has neither `rs_max` nor `state_clear`
(`grep -c state_clear` → 0). Upstream:

```cpp
// upstream (master)
if (n_rs_seq > 0) { n_rs_seq = 0; /* disabling rollback snapshots because the memory
                                   module is empty */ }
if (!pending && rollback >= 1 && rollback <= (llama_pos) cell.rs_max) { ... }
cell.rs_max = n_seq_tokens <= n_rs_seq + 1 ? n_seq_tokens - 1 : 0;
// and head-aware clearing:
const uint32_t cell_head = head;
state_clear(seq_id, cell_head, meta_read ? cell_count : 0);

// ours
if (!pending && rollback >= 1 && rollback <= (llama_pos) n_rs_seq) { ... }   // global
```

Meaning: upstream's rollback bound is **per cell** and migrates with the cell; ours is
a single global. When speculative decoding turns `n_rs_seq = n_max` on
(`common/common.cpp:1751`) and the target's draft block advances several cells, our
accounting drifts from what the graph's `get_head()`/slot math assumes — and
`build_conv_state_store` is the first place that shows it. That is exactly the
mechanism llama.cpp issue **#31656** describes ("cells migrate between ubatches,
`rs_max` doesn't follow → `seq_rm` returns false"), whose stated fix (**#31669**,
"GDN: use replay instead of rollback") is **still open**.

Provenance explanation: `engine/` is a frozen import, but the Inkling-2 model files
were hand-ported from a *later* upstream (#32694 merged 2026-11-20 `29b52d398b`,
follow-up #32704 `690b87d881`; #31660 merged 2026-11-05 routed packing archs through
`build_conv_state_store`). We transplanted post-#31660 model code onto a pre-#31660
core. The new code assumes plumbing the old core does not have.

## 4. Does it work for others? Yes — on a build nobody here is running

- Unsloth's supported engine is **their own fork release `b12631-mix-acc0843`**, not
  upstream master (`unsloth.ai/docs/models/inkling-2-mini.md:162`). Their launch line
  (docs `:265-272`) uses `--model-draft … --spec-type draft-dspark --spec-draft-n-max 2
  --n-gpu-layers-draft 99` with **sampling enabled** (`--temp 1.0 --top-p 1.0
  --top-k 0 --min-p 0.0`), not greedy.
- For the **mainline** drafter (the one I have) their README states it needs merged
  #32694 + #32704 (b12611+) **and that DSpark is still the open PR #32218**
  (`Mainline_Support/README.md:67`). So on plain master, mainline-format DSpark
  drafting is not a supported configuration at all.
- The published numbers come from follow-up PR **#32773**: `inkling2_loop` target +
  `dspark-Inkling-2-Mini-Q8_0`, **80.2 vs 47.6 tok/s (+68 %)** on code with q4_0 draft
  KV; f16 draft KV collapses to 22.7. Mainline note: n_max 5 gives "about 1.1–1.2×",
  and "2 can be slower than no drafter" (`README.md:113`).
- The PR is not accepted: ggerganov requested changes ("a lot of model special-casing
  logic without explanation") despite 26/26 CI; users report a hang
  (`common_sampler_sample_and_accept_n_rejection`, no tokens for 60 s) and a
  `markov_w1` shape mismatch `256×200058` vs `256×201024`.

**Corroboration of my own fix:** that `markov_w1` 200058-vs-201024 mismatch is the same
vocab-padding class of bug I hit and fixed with zero-padding the markov bias
(commit `7b4356e`). Someone else is seeing it too — which is evidence the integration
is genuinely rough, not that I'm uniquely failing.

## 5. Why DSpark needs target rollback at all (so it can't be waved away)

From the reference implementations: the draft block is verified in one target forward,
advancing the shortconv state by up to 8 positions; rejection invalidates all of them,
so **attention KV is not the only casualty** — the sconv state must revert too. vLLM
solves it by making Inkling's sconv state *a paged sliding-window KV cache*, so generic
block-table truncation reverts it (`vllm/models/inkling/sconv_swa_attn.py:1-16,122`);
transformers keeps full per-position conv state and slices with `crop(-n)`
(`cache_utils.py:1169-1205`). llama.cpp's answer is the rs snapshot ring — the thing
whose accounting our core has not got.

## 6. Remaining unknowns

1. Whether porting the rs cache alone is sufficient, or whether #31669's
   replay-instead-of-rollback is also needed for multi-ubatch targets.
2. Whether our failure is the *snapshot* path or the *cell-migration* path (the two
   differ: the former mis-bounds a view; the latter returns `seq_rm == false` and
   falls back to a checkpoint in `server-context.cpp:4910-4913`).
3. Whether greedy (`--temp 0`) triggers a different code path than Unsloth's sampling
   config; the PR's known hang is in the *sampler* accept/reject path.

## 7. Recommended next step (for PLAN)

**Forward-port `src/llama-memory-recurrent.{h,cpp}` to the per-cell `rs_max`
generation** — a ~117-line, one-module change, not a redesign:

1. add `rs_max` to the cell and set it in `find_slot`/`init_batch`
   (`cell.rs_max = n_seq_tokens <= n_rs_seq + 1 ? n_seq_tokens - 1 : 0`),
2. bound `seq_rm` rollback by `cell.rs_max`, not `n_rs_seq`,
3. add head-aware `state_clear(seq_id, cell_head, cell_count)` and track the restored
   head through `state_read_meta`,
4. add upstream's empty-module clamp (`n_rs_seq > 0` → 0) — this may also be the guard
   that prevents the segfault I hit when trying to clamp from the arch side.

Gates: (i) inkling2 target alone unchanged (13.1 t/s) with and without `-md`;
(ii) `-md` + `--spec-type none` clean; (iii) `draft-dspark` produces tokens;
(iv) acceptance ≥ 50 %; (v) 27B untouched.

Risk: this touches the shared recurrent memory used by Mamba/GDN/LFM2 models, so it
needs a regression sweep beyond Inkling (27B hybrid layers use it too).

## 8. Citations

Local: `engine/ggml/include/ggml.h:1655-1665`; `engine/src/llama-graph.cpp`
`build_conv_state_store`; `engine/src/models/inkling2.cpp:70-92,191-215`;
`engine/src/llama-hparams.cpp:208-220`; `engine/src/llama-memory-recurrent.{h,cpp}`;
`engine/common/common.cpp:1751`, `engine/common/common.h:396-402`;
`engine/src/llama-arch.cpp:1191-1192`.
Web: PR #32218 (+ `/files`, reviews: ggerganov CHANGES_REQUESTED), PR #32773,
issue #31656, PR #31669 (open), PR #31660 (merged 2026-11-05), #32694 (merged
2026-11-20, `29b52d398b`), #32704 (`690b87d881`), #32016;
`unsloth.ai/docs/models/inkling-2-mini.md` (§ build `b12631-mix-acc0843`:162, launch
cmd :265-272, n_max note :297), `hf.co/unsloth/Inkling-2-Mini-GGUF`
(`Mainline_Support/README.md:67,78-79,113`);
`vllm/models/inkling/sconv_swa_attn.py:1-16,122`,
`vllm/models/inkling2/speculator.py:64-70`,
transformers `cache_utils.py:1169-1205`.
