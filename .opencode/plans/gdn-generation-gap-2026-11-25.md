# Why their fork is faster: the diff, not the measurement

2026-11-25. Research answer to "we have their code — why the struggle?".
Method note first, because the criticism was correct and the failure was mine.

## 0. The mistake I made

I spent three sessions instrumenting **our** binary: op histograms, spec phase timers, per-row
costs, draft depths. All of that produced true numbers and no answer, because our engine is not
"their engine with a bug" — it is **an older generation of specific components** and the *diff* names
them directly. Porting a codepath and then profiling the port, while the reference implementation sits
on disk, is backwards. The right order is: diff the file inventory, diff the hot file, then measure
only what the diff cannot explain.

## 1. What the inventory diff says (ours vs kintsugi-engine 8f12baacf)

Files that exist in theirs and not in ours:

| file | lines | what it is |
|---|---|---|
| `ggml/src/ggml-cuda/gated-delta-net-mma.cu` (+`.cuh`) | 450 | MMA/tensor-core GDN — dispatched only for `blackwell_mma_available` (their `ggml-cuda.cu:7602`), so **not usable on sm_86** |
| `ggml/src/ggml-cuda/head2.cu` (+`.cuh`) | 643 | two-stage output head (`LLAMA_HEAD2`) |
| `ggml/src/ggml-cuda/mirai-s.cu` (+`.cuh`) | — | a KV/quant variant |
| `ggml/src/ggml-cuda/wddm.cu` (+`.cuh`) | — | Windows/WDDM memory handling (irrelevant here) |
| `src/models/dspark.cpp` | — | the DSpark drafter arch |

`src/` itself has **no** file present only in theirs, so the divergence is inside shared files.

## 2. The hot file: `gated_delta_net.cu`

Ours 667 lines, theirs 758, **1,425 diff lines**. This is the op that dominates our graph — in
`VITRIOL_GRAPH_DUMP` the largest nodes of a Kintsugi reserve are `GATED_DELTA_NET [6144, 2052]` at
48-51 MiB — and it is exactly where the unexplained per-row cost (27-51 ms/row, both models) lives.

Their kernel is a **later generation of the same op**, not a tuned copy:

- Template signature `gated_delta_net_cuda<S_v, KDA, keep_rs_t, RAW, G_PRECOMPUTED>`; ours is
  `<S_v, KDA, keep_rs_t>`. So theirs has two axes we do not have at all:
  - `RAW` — beta and g arrive **pre-activation** (`ggml_gated_delta_net_set_raw_gates`), and the
    kernel applies the activation itself, which avoids a separate graph op and its memory traffic.
  - `G_PRECOMPUTED` — `g` already holds `exp(g)`, so the sequential scan does not recompute it per
    token.
- Three precompute kernels we have none of: `gdn_precompute_exp`, `gdn_gather_state`,
  `gdn_activate_raw_gates` (`gated_delta_net.cu:17`, `:25`, `:36`). These hoist per-token gate/state
  work **out of the sequential scan and across all tokens at once** — which is precisely the shape of
  a fix for a per-row sequential cost.
- Extra kernel arguments we do not pass: `rb_d`, `ra_d` (raw gates), `s_ids` + `s_row_stride` (state
  gather by id), and `const gdn_replay_args & rp` (rollback replay).
- Arch-aware launch geometry: `gdn_cols_per_warp(highest_compiled_arch, S_v, KDA)` decides
  `cols_per_warp`; ours hardcodes `num_warps = 4` with grid `(S_v + num_warps - 1) / num_warps`.
- New op entry points: `ggml_gated_delta_net_rows` (`ggml.h:2624`, decode graphs with snapshot
  slots), `ggml_gated_delta_net_set_raw_gates`, and a replay-rollback raw-gate op (`ggml.h:2635`).

**The public `ggml_gated_delta_net(q,k,v,g,beta,state,K)` signature is unchanged** (`ggml.h:2612`
ours, `:2608` theirs), so the extra inputs ride optional extra tensor slots. That keeps the port
bounded: the graph side stays compatible and we do not have to re-plumb `llama-graph.cpp`.

Ours also has something theirs does not: an upstream small-batch **scan path**
(`gdn_scan_enabled`/`gdn_scan_supported`, our `gated_delta_net.cu:480-492`) gated to
`n_tokens ∈ {2,3,4,6,8}` — the speculation batch sizes. It came in with the upstream import
(`git log -S` → `90416d1`), so it is not a VITRIOL optimisation, and it evidently does not deliver
what their precompute generation does.

## 3. What this does and does not explain

Explains, plausibly and specifically, the per-row term: our scan recomputes gate activation and
`exp(g)` per token inside a dependent chain; theirs computes them once across the token block in
separate kernels and feeds the scan precomputed values, with arch-tuned column grouping. A cost that
scales per row and is nearly model-size independent is what that design would remove.

**Does not** explain everything. Their argv also uses `-ub 1024`, `--kq-mask-packed`, `LLAMA_HEAD2`,
and tiered KV (which we now have). Part of their prefill advantage could be `-ub 1024`; part of their
decode advantage could be `head2`. Those are separable and testable afterwards.

**Not yet proven**: that porting their GDN generation recovers the 4x. It is the strongest hypothesis
on the table, it is falsifiable in one stage (port → re-measure per-row cost with the existing timer),
and it targets the exact op the graph dump implicates.

## 4. Port surface, if we proceed

1. `ggml/src/ggml-cuda/gated_delta_net.cu` + `.cuh` — take their generation: the four template axes,
   the three precompute kernels, the extra arguments, the arch-aware launch geometry. Their MMA file
   is Blackwell-only and can be skipped (sm_86).
2. `ggml/src/ggml-cuda/ggml-cuda.cu` — the GDN caller: pass raw gates, `s_ids`/`s_row_stride`, replay
   args; adopt their dispatch predicate.
3. `ggml/src/ggml.c` + `ggml/include/ggml.h` — optional extra src slots and
   `ggml_gated_delta_net_set_raw_gates`; `ggml_gated_delta_net_rows` only if we take snapshot
   decode graphs.
4. Replay/snapshot features (`LLAMA_RS_REPLAY`, `paragraph` rollback) are **optional** — they are
   correctness/rollback machinery, not speed, and can be left out of a first port.

Licence: theirs is MIT (ggml authors), so this is a compatible inbound port with a `PROVENANCE:`
header, exactly as the Escha and tiered-KV ports were done.

## 5. What I would measure first, cheaply, before any port

One experiment that partly separates "GDN kernel" from "everything else", with no code:
run their engine and ours at the **same** `-ub` (say 32 and 128) and compare **decode** only. If the
gap is ~1.6x at every ub, it is per-token work (the GDN hypothesis). If the gap shrinks with ub, part
of it is batching and `-ub 1024` is a separate lever. Their launcher defaults differ, so this needs
their `KINTSUGI_UBATCH` equivalent set explicitly.

## 6. Honest summary

We have their code, and we have had it — but I kept treating the difference as a defect to be found by
profiling our build rather than as a **version gap to be read off a diff**. That is the "struggle"
you're pointing at, and it was a method error, not a missing file. The diff now names the gap: our
GDN is upstream's generation, theirs is a rewrite with precomputed gates, raw-gate inputs, state
gather and arch-aware launch — in the one op the graph dump implicates.
