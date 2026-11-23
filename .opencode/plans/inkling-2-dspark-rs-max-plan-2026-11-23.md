# Plan: forward-port the recurrent memory to per-cell rs_max (unblocks DSpark drafting)

**Date:** 2026-11-23
**Status:** APPROVED (owner's go-ahead given; research confirmed)
**Research:** `inkling-2-dspark-rs-cache-research-2026-11-23.md`

## Goal

Make `--spec-type draft-dspark` run on an Inkling-2 target by bringing
`src/llama-memory-recurrent.{h,cpp}` up to the generation that tracks rollback
snapshots **per cell**, without regressing every other arch that shares the module
(27B hybrid layers, Mamba, GDN, LFM2).

## Why this is the fix

Measured: every drafter-side and conv-state surface is byte-identical to upstream, and
PR #32218 touches none of the target's rs machinery — so the drafter is not the defect.
`llama-memory-recurrent.cpp` differs from master by 103 lines, `.h` by 14, and ours has
neither `rs_max` nor `state_clear` (grep → 0). Upstream bounds rollback by
`cell.rs_max` (which migrates with the cell); ours by one global `n_rs_seq`. With
`n_rs_seq = n_max` (common.cpp:1751) the accounting drifts and the conv-state K-slot
view overruns. Same family as llama.cpp issue #31656.

## Stages

| stage | delivers | gate |
|---|---|---|
| **R1** | `cell.rs_max` field + assignment at slot allocation (`n_seq_tokens <= n_rs_seq + 1 ? n_seq_tokens - 1 : 0`); `seq_rm` bounds by `cell.rs_max` | build green; **target-only** inkling2 decode ≥ 13 t/s; 27B unaffected |
| **R2** | head-aware `state_clear(seq_id, cell_head, cell_count)` + restored-head tracking through `state_read_meta` | 27B slot restore/warm-resume still works (state_read_meta path) |
| **R3** | empty-module clamp (`n_rs_seq > 0` → 0 when the module is empty) | `-md` + `--spec-type none` clean |
| **R4** | measure DSpark end-to-end | tokens emitted; **acceptance ≥ 50 %**; decode t/s vs the 20.3 t/s ngram baseline |

## Invariants

| # | property | pinned by |
|---|---|---|
| **I1** | With no draft model, Inkling-2 numerics are unchanged: `The capital of France is` greedy output matches the `9ce106a` reference and decode stays ≥ 13 t/s. | `inkling2_greedy_golden`, `inkling2_decode_floor` |
| **I2** | The 27B path is unchanged: decode ≥ its current baseline and `probe-recommend` byte-identical. | `bench_27b_unaffected`, `probe_recommend_golden` |
| **I3** | Slot persistence survives: save → restart → restore warms without re-prefill (the `state_read_meta`/head path R2 touches). | `slot_restore_warm` |
| **I4** | `n_rs_seq == 0` behaviour is byte-identical to today for every arch (the clamp and `rs_max` are inert without snapshots). | `rs_disabled_parity` (inkling2 + 27B, no spec) |
| **I5** | A drafter that loads but drafts badly must fail, not pass silently. | `dspark_acceptance_floor` ≥ 50 % |
| **I6** | Every measured number carries its `VITRIOL-FINGERPRINT:` including draft KV type. | `fingerprint_present` |

## Named tests (scripted gates, each blocks its stage)

- `build_green` — `cmake --build build -j8 --target llama-cli` produces no errors.
- `inkling2_decode_floor` — `-ngl 99 -ts 27,21 --no-mmap -ub 64 -c 1024 -fa on -n 8
  --temp 0 -p 'The capital of France is'` → exit 0, ≥ 13 t/s.
- `inkling2_greedy_golden` — md5 of the 16-token greedy continuation unchanged.
- `bench_27b_unaffected` — live engine `/health` ok + a completion returns; decode
  within noise of baseline.
- `probe_recommend_golden` — `libvitriol` probe-recommend output diff empty.
- `slot_restore_warm` — stop engine, restart, confirm warm resume (delta-only prefill).
- `rs_disabled_parity` — with no `-md`, n_rs_seq is 0 and output matches pre-change.
- `dspark_acceptance_floor` — parse acceptance from the `draft-dspark` log; fail < 50 %.

## Risk

The module is shared by Mamba/GDN/LFM2 **and the live 27B's hybrid layers**. A mistake
regresses the operator's daily driver. Mitigation: every stage gates on I1/I2/I4 before
proceeding; the port is additive (new field + new function + clamped bounds), and the
`n_rs_seq == 0` path stays untouched by construction.

## Change set

`engine/src/llama-memory-recurrent.h`, `engine/src/llama-memory-recurrent.cpp`.
Nothing else unless a caller requires it.

---

## Results (2026-11-23) — commit `5a6d9b9`

### Three blockers cleared; drafting now executes end to end

| # | fix | evidence it was the blocker |
|---|---|---|
| 1 | per-cell `rs_max` + head-aware `state_clear` + empty-module clamp | `-md` runs now reach drafting instead of aborting |
| 2 | inkling2 target feature taps (`t_layer_inp[il]`, `t_layer_inp[n_layer]`) | assert `layer input tensor is null` gone; `n_extract=4` reported |
| 3 | ggml: bound views by the **strided** extent, not the dense footprint | the conv-state K-slot view is an *overlapping* view; upstream's comment "a view stays inside its source, over the bytes its strides actually reach" |

Plus one configuration finding: the draft must sample on the CPU
(`--no-spec-draft-backend-sampling`). On-device top-k samples over the padded
201024 logit rows and emits ids like 200136, which the 200058-entry vocab rejects
(`invalid token[2] = 200136` → `Invalid input batch`).

**The drafter now runs:** coherent output, `block_size=8, n_extract=4,
sample_from_anchor=false`, exit 0.

### Acceptance is ~0 — proven by construction

| arm | decode t/s |
|---|---|
| no speculation | 15.9 |
| `-n-max 1` | 8.3 |
| `-n-max 2` | 5.4 |
| `-n-max 4` | 4.5 |

Monotone *decrease*. Healthy speculation is the opposite (more accepted tokens per
target step ⇒ faster). So every draft token is rejected and each one is pure cost.
Gate I5 fails, correctly.

### What that means

The plumbing (features, ctx injection, mask rows, markov head, rs rollback) is
functional; the **draft graph's numerics are a reconstruction**, and at least one
structural choice is wrong. Candidates, in order of likelihood:

1. **anchor-row input.** Row 0 of each block may need the mixed target feature (the
   PR's `ubatch.embd_state` exists for exactly this), not `tok_embd[anchor]`. My
   in-graph `f_rnn` covers only the markov head, not the block input.
2. **ctx-K/V placement.** Whether the per-block ctx K/V form a prefix the block
   attends over, and whether they take rope or the block's positions.
3. **mask_embd slot mapping** (row order ⇒ slot) and its interaction with
   `sample_from_anchor=false` (`i_draft_beg = 1`).
4. layer-wise vs single ctx mix (I mix per block).

Next step is a measurement loop, not a guess: instrument acceptance per position
(the impl's `accept()` is currently a noop) so each graph change is scored
immediately instead of inferred from t/s.
