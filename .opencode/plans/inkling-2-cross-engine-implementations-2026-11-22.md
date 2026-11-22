# How other engines implement Inkling-2-Mini — and what our port still gets wrong

Research note, 2026-11-22. Follows
`inkling-2-mini-numerical-fix-2026-11-22.md` (GRIND stopped at "output still
degenerate: `0. 0. 0. …`" after fixing the V-aliased-to-K bug).

Purpose: the model works in other stacks, so the remaining defect is in our
port. Find out how the architecture is *supposed* to be implemented by reading
the reference implementation and the engines that already support it, then
narrow our remaining bug against that.

---

## 1. What the model actually is

Thinking Machines Lab, **`thinkingmachines/Inkling-2-Mini`**, released
2026-11-20, Apache-2.0 — a 26.4 B-total / ~5.4 B-active **looped Mixture-of-
Experts** language model (the repo's GGUF is text-only; the full release is
multimodal).

- Model card: https://huggingface.co/thinkingmachines/Inkling-2-Mini
- Announcement: https://huggingface.co/blog/inkling2mini

Crucially, **the canonical forward pass ships natively in `transformers`** (no
`trust_remote_code`):
https://github.com/huggingface/transformers/blob/main/src/transformers/models/inkling2_loop/modeling_inkling2_loop.py
and
https://github.com/huggingface/transformers/blob/main/src/transformers/models/inkling2_loop/configuration_inkling2_loop.py

So there is an authoritative, readable specification — we do not have to infer
semantics from llama.cpp alone. That is the single most useful fact from this
research pass.

### The six unusual components, per the reference

Cited as (M:line) = `modeling_inkling2_loop.py`; these line numbers come from
the research pass and may drift as upstream moves.

| # | component | reference semantics |
|---|---|---|
| 1 | **shortconv** (`sconv_kernel_size=4`) | Depthwise **causal** conv, 4 taps. Tap index = **backward distance**; `static_weight[0]` = current token. Four sites per layer (`conv_idx`): 0 = Q, 1 = **K/V**, 2 = post-attention, 3 = post-MLP. Output is `(conv + residual)/√(K+1)`; for the *dynamic* K/V variant `/√(2K+1)`. State lives in `Cache.conv_state[layer_idx + cache_offset][conv_idx]`. |
| 2 | **attn_rel** (`d_rel=16`, `relative_extent=1024`) | Per-token hidden → 16 states → `proj[16,1024]` → one bias per backward distance, **gathered by distance**, and **zero outside `[0,1024)`**. Added to **scores**, not to the mask. `relative_clip=8.0`. |
| 3 | **attn_res** (`block_size=6`) | Every layer does an RMS-normed attention with a *fixed learned query* over the completed block summaries + the current partial state, and that becomes the layer input. Also applied once more at the very end (the `output_res_query`). |
| 4 | **log-scaled positions** (`alpha=0.1`, `floor=8192`) | `τ = 1 + 0.1·log(max((pos+1)/8192, 1))`, multiplying **q** (full-attention layers only) and the relative states. |
| 5 | **loop** (`count=3`, `start=6`, `end=18`, `gate_rank=64`) | 3 passes over the core layers, each restarting from the pre-loop state. Cross-pass `MessageGate`: `(1 − tanh f)·h + tanh g·msg`, applied per Q/K/V/MLP branch. |
| 6 | **sinks** | One extra zero logit per head absorbs probability mass; it is dropped after softmax (implemented by baking `−sink` into the bias table). |

### Order of operations for K/V (the part that matters most)

From the reference (`Inkling2LoopKeyValueProjection.forward`, ~L523-528, and the
attention call sites ~L612 / ~L703):

1. **Conv runs BEFORE caching, never after retrieval.** linear → sconv →
   reshape → norm; *then* `cache.update(key_states, value_states, …)`. Nothing
   touches K/V after `cache.update`.
2. **Dynamic taps are full-attention only, for K *and* V.**
   `Inkling2LoopFullAttention` passes `dynamic=True` for both `k_proj` and
   `v_proj` (~L655-660); `Inkling2LoopSlidingAttention` uses the static conv
   (~L495-507, ~L562-568).
3. **What is cached:** post-conv, **L2-clipped K**; post-conv,
   **unweighted-RMS-normalised V** (`Inkling2LoopUnweightedRMSNorm`) — V is
   normalised per head *before* caching (~L510-512, ~L67-76).
4. **Recurrent state:** the conv state keeps the last **4 raw (pre-conv)
   projection outputs** (~L1004-1008 `cache_utils.py`) — i.e. 3 history + the
   current token, per KV head. A 1-token decode uses `causal_conv1d_update`
   (~L374-377).

---

## 2. How the other engines do it

| engine | support | conv state | cached K/V | rel bias | attn_res |
|---|---|---|---|---|---|
| **vLLM** | ✅ `vllm/models/inkling2/nvidia/` | one paged `InklingConvState` **per layer per loop-pass**, 4 streams (K, V, attn, mlp) packed head-major (`sconv.py:3-10`), addressed by absolute position via `block_table` + `slot_mapping` | **convolved**: prologue L2-clips K, RMS-norms V, then stores to paged KV (`ops/sconv.py:3-25`, `attention.py:453-479`) | r·16 profiles → per-head distance table, **sink-subtracted**, gathered by distance (`ops/sconv.py:470-489`) | fp32 softmax mix of block states (`ops/attn_res.py:3-11`) |
| **SGLang** | ✅ | centralised `MambaPool` per request (`inkling_sconv_backend.py:17-28`) | convolved | pre-sheared rel bias fused into the prologue (`inkling2_attn.py:449-509`) | yes |
| **TensorRT-LLM** | ✅ | per-layer `[slots, 3, C]` × 4, *separate* from paged KV; **block reuse refused** (`cache.py:3-12, 83, 99-107`) | convolved | fp32 rel matmul **mandatory** (`modeling.py:306`) | `residual_mix.py` |
| **llama.cpp** (upstream `8a97fa98`) | ✅ | loop passes get their own KV + conv slots (`models/inkling2.cpp:194-237, 329-388`; `models/inkling2-loop.cpp:24-41`) | convolved (`cpy_k` + `cpy_v`, then `get_k`/`get_v`) | bias folded into the mask | `mix()` |
| **mlx-lm** | ❌ none — it lives in **mlx-vlm** `inkling2/language.py:80-205` | | | | |

**The unanimous pattern: every engine caches the *convolved* K and V, and every
engine keeps a per-layer, per-conv-site recurrent state.** That is exactly the
invariant our fork violated (V aliased to K) and exactly what our

`convolve()` now does correctly.

### Documented port bugs elsewhere (the failure classes to check ourselves against)

These are other people hitting the same class of problem — worth walking through
as a checklist:

- **SGLang #46572** — the conv prologue wrote CUDA-graph *padding rows* into KV
  slot 0 → NaN. (Our analogue: writing phantom/concat rows into a real cache
  slot.)
- **vLLM** — short-conv page size vs attention page size mismatch
  (`short_conv.py:74-81`). (Our analogue: conv state ring vs KV cell count.)
- **TensorRT-LLM** — refuses block reuse entirely for this arch because the conv
  state cannot be reconstructed from KV.
- **llama.cpp #32710 / #32785** — looped-slot KV locality bugs.
- **MLX** — speculative-decoding conv-state rollback.

---

## 3. Where our port actually stands

Verified **correct** by direct diff against upstream `8a97fa98` and/or against
the reference semantics above:

| piece | status |
|---|---|
| `models/inkling2.cpp`, `models/inkling2-loop.cpp` | semantically verbatim (only `no_build{}` plumbing and the dropped `prec_policy.set_acc`) |
| `convolve()` | verbatim, **including** `1/√((dynamic?2:1)·K+1)` — matches reference `(conv+res)/√(K+1)` and `/√(2K+1)` |
| `control()` (MessageGate) | verbatim `(1−tanh f)·h + tanh g·msg` |
| `build_inputs()` incl. `τ` log-scaled positions | verbatim |
| `mix()` (attn_res) | verbatim |
| `build_conv_state_store` | verbatim |
| `set_input_relative_pos` | line-identical (out-of-range → `extent`, padded bias row = zero ⇒ matches "zero outside [0,1024)") |
| `n_embd_r()` sconv branch | identical (`n_sconv_channels·(l_cache−1)`, checked **before** LFM2) |
| `build_moe_ffn` fused `gate_up_exps` + `LLM_FFN_SITU` | verbatim |
| `build_output` (incl. softcap + unpadded vocab slice) | verbatim |
| `create_memory` hybrid filters for inkling2 | both all-true, same as upstream |
| `cparams.n_rs_seq` | 0 by default ⇒ `build_conv_state_store` takes the simple `cpy` path, not the rollback ring |
| `hparams.is_mla()` | false for inkling2 ⇒ a real V cache **is** allocated (`llama-kv-cache.cpp:291 has_v = !is_mla`) |

Fixed this session (`dcc67dc`): the rel attention path now stores/reads a real V
(`cpy_k` + `cpy_v`, `get_k` + `get_v`) instead of aliasing V to K via the MLA
shortcut that `deepseek4`/`dflash` rely on.

Still failing: `I4` — greedy decode is degenerate (`0. 0. 0. …`).

---

## 4. Remaining hypotheses, ranked

Ordered by (evidence weight × cheapness to test). None of these is confirmed;
each names the exact observation that would settle it.

### H1 — the conv history read and the conv state write disagree (highest)

The reference and all four engines keep **4 raw pre-conv inputs per conv site
per layer**. Our `convolve()` reads `past` from the recurrent state via
`build_rs(recr, state, hparams.n_embd_r(), n_seqs)` and writes via
`build_conv_state_store(...)`. `n_embd_r()` and `build_conv_state_store` are
both verbatim — **but the fork's recurrent memory is not upstream's**: the fork
has its own `llama_memory_recurrent`, `s_copy_main` / `s_copy_extra` split, and
a rollback ring. Upstream's `build_rs(inp, …)` passes
`(state_copy, inp->s_copy_main)`; the fork passes
`(inp->s_copy_main, inp->s_copy_extra)` — same shape of call, different source
tensors.

*Test:* dump the recurrent state (`get_r_l(il)`) for one conv site across two
decode steps and assert the 3 history columns advance. If they stay zero, this
is the bug. **Prediction if true:** each step's conv sees only the current
token, so attention has no local context — which is exactly "echoes a word from
the prompt, then latches".

### H2 — SWA vs full-attention V index path

I use `inp->get_v_idxs_swa()` for SWA layers. The reference has SWA layers using
the **static** conv and full-attention layers the **dynamic** one, and I have
verified the model file chooses correctly — but the *index tensors* for V in the
SWA branch were never exercised by any other model in this fork
(`build_inp_mem_hybrid_iswa_rel` has exactly one user). If `v_idxs_swa` is null
or mis-sized, SWA layers write V to the wrong cells.

*Test:* assert `get_v_idxs_swa()` is non-null and equal to `get_k_idxs_swa()` at
the first SWA layer.

### H3 — `v_trans` layout mismatch between `cpy_v` and `build_attn_mha`

The fork was written expecting `v` to be a *view of k*, so the `v_trans`
assumption in `build_attn_mha`'s transpose branch may never have been exercised
with a real cached V. The cache is constructed with
`attn_v_trans = !cparams.flash_attn` — and we run `-fa off`, so `v_trans = true`.

*Test:* flip `-fa on` (which exercises the other branch) and see whether the
degeneracy changes character. Cheap, and informative either way.

### H4 — the looped phantom slots get a real KV but no correct hparams arrays

Upstream fills `n_head_arr`, `n_head_kv_arr`, `n_ff_arr`, `n_ff_exp_arr`,
`n_expert_used_arr`, `is_swa_impl`, `is_recr_impl` for `il >= n_phys` from
`layer_phys(il)`. If any of those arrays is short in our build, `hparams.is_swa(il)`
for a phantom slot could read out of bounds or default to false, sending a
looped pass down the full-attention path when it should be SWA.

*Test:* print `is_swa`/`n_head_kv` for `il` 0..47 and diff against
`layer_phys()` expectation.

### H5 — attn_res block bookkeeping (`blocks`)

`mix()` consumes the `blocks` vector; `build_layer` pushes into it. If the
push/pop boundary is off by one relative to `attn_res_block_size = 6`, the
residual mix uses the wrong set of summaries. It would corrupt output without
breaking shapes — the hardest class to see.

*Test:* assert `blocks.size()` at the start of each `il` equals
`il / block_size` (completed blocks) and log it.

### Ruled out (do not re-chase)

- chat template / prompt format — a fully templated greedy prompt still emits
  `message`;
- rel bias folded into the mask vs added to scores — byte-identical output
  (softmax-linearity), and the reference says "added to scores" anyway;
- V aliased to K — fixed (`dcc67dc`), behaviour changed;
- upstream `8a97fa98` as a differential reference — it cannot load this model on
  2×12 GB (single 15 861 MiB lump on CUDA0) even with our placement fix;
- `n_rs_seq` — 0 by default, so the rollback ring is not in play;
- `is_mla()` — false, so a real V cache exists;
- greedy vs sampling degeneracy — not the sole cause.

---

## 5. Recommendation

The single highest-value next step is **not** more flag experimentation: it is
**H1**, a state-inspection test on the conv recurrent state across two decode
steps. Every engine that supports this model treats that state as
load-bearing, our fork's implementation of it is the one piece we did *not*
port verbatim (it is fork-specific machinery), and the symptom matches its
failure mode precisely.

Cheap companion tests, each < 1 build: H2 (null-check the SWA V idxs), H3 (one
`-fa on` run), H4 (dump the phantom-slot hparams arrays), H5 (assert the blocks
bookkeeping).

## Sources

Local (this repo / the frozen upstream clone at
`/scratch/briev-builds/llama-upstream` @ `8a97fa98`):

- `engine/src/models/inkling2.cpp` — `convolve()`, `control()`, `mix()`,
  `build_inputs()`, `build_layer()`, `build_output()`
- `engine/src/llama-graph.cpp` — rel `build_attn`, `build_conv_state_store`,
  `build_inp_mem_hybrid_iswa_rel`, `build_attn_mha`
- `engine/src/llama-kv-cache.cpp:291` — `has_v = !is_mla`
- `engine/src/llama-hparams.cpp:208-222` — `n_embd_r()`
- `engine/src/llama-context.cpp:109-114` — `n_rs_seq` clamping
- `.opencode/plans/inkling-2-mini-numerical-fix-2026-11-22.md` — GRIND evidence

Web:

- https://huggingface.co/thinkingmachines/Inkling-2-Mini
- https://huggingface.co/blog/inkling2mini
- https://github.com/huggingface/transformers/blob/main/src/transformers/models/inkling2_loop/modeling_inkling2_loop.py
- https://github.com/huggingface/transformers/blob/main/src/transformers/models/inkling2_loop/configuration_inkling2_loop.py
