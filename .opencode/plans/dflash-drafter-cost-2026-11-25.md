# DFlash2 drafter: why 58 ms/row, and why the draft context reserves 5.4 GB

Research notes, 2026-11-25. Read-only investigation for track A item A3 and open task 11 in
`kintsugi-speed-and-capacity-2026-11-25.md`. No code was changed to produce this.

## 1. What the drafter actually is

`dflash-Qwen3.8-27B-Q4_0.gguf`, 1,094,346,080 B, `dflash` architecture, **81 tensors = 6 special +
5 blocks × 15**. Per block: `attn_q`, `attn_k`, `attn_v`, `attn_output`, `ffn_gate/up/down`,
`attn_conv_base`/`attn_conv_proj`, `ffn_conv_base`/`ffn_conv_proj`, `attn_k_norm`, `attn_q_norm`,
`attn_norm`, `ffn_norm`. Special tensors: `fc.weight` (73.7 MB encoder projection),
`enc.output_norm`, `output_norm`, `selector_hidden.weight` (0.74 MB),
`selector_predecessor.weight` and `selector_successor.weight` (35.8 MB each).

Weight geometry, from the tensor byte sizes: `ffn_*.weight` 50,135,040 B = 17,408 × 5,120 at Q4_0;
`attn_q`/`attn_output` 11,796,480 B = 5,120 × 5,120 at Q4_0. So **embd 5,120, ffn 17,408, 5 layers,
Q4_0**. Convolution tensors (`conv_kernel_size`, `conv_group_size`) make each block a
convolution-augmented attention block, not a plain transformer block.

Total ≈ 1.07 GB. A full pass therefore streams ~1.07 GB: at the ~90 GB/s this host achieves that is
**~12 ms per decode batch, independent of how many rows it carries**. Measured cost is ~58 ms *per
row*. The gap is ~46 ms/row of something that is not weight streaming.

Metadata keys present but with values not yet dumped: `dflash.block_count`,
`dflash.context_length`, `dflash.embedding_length`, `dflash.feed_forward_length`,
`dflash.attention.{head_count,head_count_kv,key_length,value_length}`,
`dflash.block_size`, `dflash.conv_{kernel,group}_size`, `dflash.selector_{rank,top_k}`,
`dflash.target_layers`, `dflash.attention.sliding_window{,_pattern}`,
`dflash.rope.{freq_base,dimension_sections}`. `llama-gguf <f> r` prints keys and the tensor table
but not values; the next step is `vitriol-calibrate` or a `llama_model`-side dump.

## 2. The configuration is not the difference

`common_base_params_to_speculative()` is **line-for-line identical** in both engines
(ours `common/speculative.cpp:2613`, theirs `common/speculative.cpp:3367`): the draft context inherits
the target's `common_params` wholesale (`common_params result = params`), overriding only
`devices`, `model`, `n_gpu_layers`, `tensor_buft_overrides`, thread counts, and the KV cache types.
Block drafts then set `n_outputs_max = n_parallel × (n_max + 1)` and `n_outputs_max_per_seq = per_seq`.

Theirs differs in exactly one respect: it clears `kv_mean_center_path` because the K-cache
mean-centring bias is calibrated for the target's K geometry. Irrelevant here (we pass no bias).

So the 4x is not a spec-config difference. It is either the graph the drafter builds, or where the
time is actually spent — see §4.

## 3. The 5.4 GB reservation is vocab-proportional, and the code is right there

Failing allocations, single card, both with and without tiered KV:

```
-ub 128: 5,564,565,504 B   -> /4 = 1,391,141,376 floats -> /200,704 = 6,931.9
-ub  32: 5,442,884,096 B   -> /4 = 1,360,721,024 floats -> /200,704 = 6,780.0
```

Both land on an integer multiple of the vocabulary. `src/models/dflash.cpp` materialises exactly that
kind of tensor, twice, in the `model.d2t` (draft→target vocabulary reduction) path:

- `dflash.cpp:485` — `ggml_fill(ggml_new_tensor_3d(ctx0, GGML_TYPE_F32, 1, n_vocab, n_blocks), 0.0f)`
  then a `ggml_set_rows` scatter of the reduced draft vocabulary into the full one.
- `dflash.cpp:936` — the same pattern on the normal decoder path:
  `ggml_fill(..., ggml_new_tensor_3d(F32, 1, n_vocab, n_outputs), -INFINITY)`, `ggml_set_rows`,
  `ggml_reshape_2d`.

A dense `f32 [n_vocab, n_rows]` tensor is 4 bytes × 200,704 per row = 803 KB per row. So ~6,780–6,932
rows would produce the observed numbers. Note that neither 4 (n_max+1) nor 32/128 (ubatch) explains
that row count, so either the tensor is being sized from something larger than the decode batch, or a
second, differently-shaped tensor is the 5.4 GB one. **This must be measured, not deduced.**
The cheap measurement already exists in the fork: `VITRIOL_GRAPH_DUMP=N` (commit `8356473`), which
dumps graph tensors and their shapes — running one dflash load with it set will name the 5.4 GB
allocation outright.

Why this matters beyond memory: their engine's `LLAMA_HEAD2` (two-stage output head) and
`--kv-vram-cells` exist precisely because full-vocabulary tensors are the expensive thing in this
model class. If our dflash path materialises a full-vocab f32 tensor *per decode step*, that is a
6x-102x cost depending on rows, and it would also explain §4.

## 4. The measurement is currently ambiguous — this is the honest finding

Two instruments disagree, and the disagreement is informative:

| instrument | says |
|---|---|
| `VITRIOL_SPEC_TIMING` (host wall clock around `common_speculative_draft`) | 234.69 ms/step at n_max 3 |
| `GGML_CUDA_OP_TIMING` (per-node GPU events, graphs off) | ~0.96 ms/graph, top node `MUL_MAT q4_0 5120x17408 cols=4` at 0.199 ms |

The op histogram's shapes are unmistakably the drafter (Q4_0, ffn 5120×17408, `cols=4` = the four
noise rows, and a `TOP_K f32 cols=4` = the selector). Yet it totals one millisecond against a
235 ms wall clock. Three candidate explanations, none yet excluded:

1. **`draft()` blocks on the target.** It touches target-side state first —
   `llama_get_embeddings_nextn/layer_inp` (`speculative.cpp:647-661`, `:821`) — and the target's
   verify decode of `n_max+1` rows may still be in flight. A sync there charges the target's verify
   time to `spec_draft`. This fits the numbers well: ~235 ms ≈ a 4-row verify of a 10.5 GB model.
2. **The events miss work on secondary streams.** Our fork forks work through the
   concurrent-event machinery; the A1 timer was already fixed once for this and still under-reports.
3. **The host spends the time**, e.g. copying a large `embd_nextn` lattice back per step, or the
   `d2t` scatter forcing an extra full-vocab materialisation per row.

**The decisive test is small and read-only in spirit**: time `llama_decode(ctx_dft, batch)` alone
(not the whole of `draft()`) and separately the lattice fetch, for one 200-token generation. If
`llama_decode(ctx_dft)` is ~10 ms and the lattice fetch is ~220 ms, hypothesis 1 or 3 is confirmed
and the "58 ms/row" framing in the plan is wrong — the drafter would be nearly free and the number
would be an artefact of where the timer sits.

## 5. What their engine has that we do not (from their argv and README)

`--spec-draft-window`, `--spec-draft-n-max-tail` (their README: "the model's own MTP head at every
depth, and lookup drafting from the prompt"), `LLAMA_HEAD2` two-stage output head,
`--kv-vram-cells`, `--kq-mask-packed`, `--reasoning-*`. We have implemented the third of those
(tiered KV). The other four are unported.

Given §3, **`LLAMA_HEAD2` is the most interesting**: it exists to avoid reading a 200k-row output
head from tiered storage by keeping a small q4_0 copy in VRAM and re-scoring 512 exact candidates per
row from system RAM. That is the same class of problem as our full-vocab f32 tensors.

## 6. Recommended next steps, in order

1. **`VITRIOL_GRAPH_DUMP=1` on one dflash load.** Names the 5.4 GB tensor and its shape. No code.
2. **Split the `draft()` timer** into (a) `llama_decode(ctx_dft, batch)`, (b) the lattice fetch, (c)
   the post-decode argmax. Decides whether §4's hypothesis 1/3 holds. Small code, one build.
3. If the lattice fetch or the `d2t` scatter is the cost: avoid the padded full-vocab materialisation
   (the `d2t` scatter is the obvious target — it exists only because the draft vocabulary is
   reduced).
4. Only after 1-3: consider porting `LLAMA_HEAD2`, which is a capacity/bandwidth feature for the
   same 200k-row head.

## 7. Confidence and gaps

High confidence: the drafter's geometry (§1), the config parity (§2), the vocab-proportional
arithmetic (§3), the instrument disagreement (§4).

**Not** established: which of §4's three explanations is true, and therefore whether A3's premise
("58 ms per drafter row") is even correct. That is the single most important open question — the
plan's framing of A3 should be treated as provisional until step 2 above runs.

Gap I did not close: I did not search the wider literature on EAGLE-3/DFlash-style drafters. The
strongest available evidence here is comparative rather than bibliographic — the same code on the
same host producing 14.55 t/s (theirs) and 8.80 t/s (ours) undrafted, and 72.8 vs 17.5 with the same
drafter, which localises the difference to something our build does that theirs does not. If the
owner wants the literature angle, that is a separate pass.
