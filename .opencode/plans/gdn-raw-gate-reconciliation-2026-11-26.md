# GDN raw-gate reconciliation — plan (2026-11-26)

Goal: make the fused GDN op's raw-gate path **correct** in our fork, keeping the asset their file
would have deleted, then measure whether it buys the decode parity we are chasing (ours 7.95-8.80 t/s
vs their 14.55 on Kintsugi-27B).

Research: `gdn-raw-gate-research-2026-11-26.md`. Predecessor: `gdn-port-plan-2026-11-25.md` §7-§8.

## Premise (from research)

The graph half is correct by construction — it is textually identical to theirs. The defect is
below it, and reading cannot localise it further (§3-§4 eliminated every gross-failure hypothesis
that static analysis can reach). Two rounds failed by editing before measuring. **This plan inverts
that: instrument and isolate first, edit only what the isolation names.**

Second premise, equally load-bearing: **the port is raw INTO our file.** Their file lacks our chunked
kernel (`n_tokens ∈ {2,3,4,6,8}`), and importing it wholesale deleted that kernel and installed an
MMA path that cannot run on sm_86 — plausibly the whole of the previous round's slowdown.

## Stages

### Stage 1 — op-level isolation (no engine change)

New test `tests/test-gdn-raw-gates.cpp`, modelled on `tests/test-kv-tier.cpp` (same CMake GLOB, no new
target needed). Two runs of `GGML_OP_GATED_DELTA_NET` over identical q/k/v/state:

- arm A: activated gates — `g = a[h]*softplus(alpha + dt_bias[h])`, `b = sigmoid(beta_raw)`;
- arm B: raw gates — same `alpha`/`beta_raw` handed in, then
  `ggml_gated_delta_net_set_raw_gates(op, dt_bias, a)`;
- require max |A − B| ≤ 1e-4 relative on both the attention output and the final state;
- **negative control**: perturb `a` by 10 % in arm B ⇒ outputs must differ (proves arm B actually
  reads `src[7]`/`src[8]` rather than silently ignoring them — the failure mode that makes a test
  like this lie);
- shapes: `S_v=128, H=48, n_tokens ∈ {1, 8}, n_seqs=1, K ∈ {1, 4}` — the Kintsugi geometry
  (`v = [S_v, H, T, S]`, GDN reserve tensor is `[6144, 2052]` ⇒ `S_v*H = 6144`) and both snapshot
  cases;
- the CPU backend is exercised only to *document* that it lacks raw (expect skip), since the port
  keeps raw CUDA-only.

Output: a number, not an opinion. Op correct ⇒ Stage 2. Op wrong ⇒ Stage 2 is skipped and the fix is
in the kernel's raw branch.

### Stage 2 — graph-level localisation (only if Stage 1 passes)

Env-gated comparison: run the same prompt with `GGML_GDN_RAW_GATES_DISABLE=1` and unset, dumping each
`GATED_DELTA_NET` output (reuse the existing `VITRIOL_TENSOR_DEBUG` machinery, no new switch unless it
proves insufficient). Print the first layer index and max-abs delta where they diverge. That names the
offending tensor directly instead of guessing which of `alpha`/`beta`/`dt_bias`/`a` is wrong.

### Stage 3 — the port, raw into our file

Re-apply the import **as a merge, not a replacement**:

1. keep our `gdn_scan_enabled`/`gdn_scan_supported`/`launch_gated_delta_net_chunk` and their dispatch
   decision for it;
2. add their `RAW`/`G_PRECOMPUTED` template parameters and `rb_d`/`ra_d` args to the per-token kernel;
3. give the chunk kernel raw support the way their MMA branch does it — activate into temporaries with
   their `gdn_activate_raw_gates` before the chunk kernel runs, so the recurrence is untouched;
4. keep their `gdn_precompute_exp` (DGX-Spark-gated, inert here) and the replay struct as unused
   scaffolding only if they cost nothing; do not port replay itself.

Plus the graph half exactly as ported last round (it was correct): `set_raw_gates`, the four
`gdn_raw_*` members, both call sites, `beta_raw` capture — and the device guard restricted to CUDA,
since our CPU GDN has no raw path.

### Stage 4 — measurement and verdict

Two runs per arm at the same config (baseline spread is 8.07-8.80 t/s, so a single run is not a
verdict): no-draft `c 8192`, no-draft `c 131072`, and `kintsugi-27b-262k` with DFlash2 n_max 7.
Keep only if decode improves beyond that band **and** the golden reply is unchanged.

## Invariants

- **I1 (parity).** With raw disabled, argv and output are byte-identical to `3355896`.
  *Pinned by*: greedy 96-token golden reply comparison, and `GGML_GDN_RAW_GATES_DISABLE=1` reproducing
  the baseline decode number within the run band.
- **I2 (op equivalence).** The raw path computes the same function as the activated path: max relative
  difference ≤ 1e-4 on attention output and final state.
  *Pinned by*: `test-gdn-raw-gates` arms A/B.
- **I3 (the test cannot lie).** A 10 % perturbation of `raw_a` must move the output.
  *Pinned by*: `test-gdn-raw-gates` negative control.
- **I4 (chunk path preserved).** `n_tokens ∈ {2,3,4,6,8}` still reaches `launch_gated_delta_net_chunk`,
  with raw either supported there or bypassed deliberately — but never silently dropped.
  *Pinned by*: a launch-path assertion (a `GGML_CUDA_GDN_SCAN` log or counter) at `n_tokens=8`, plus
  the existing MTP verify path still producing correct output.
- **I5 (device safety).** Raw gates engage only on the CUDA backend; a partly-offloaded or `-ngl 0`
  model keeps activated gates and correct output.
  *Pinned by*: an `-ngl 0` run of the test model producing sane text, and `test-gdn-raw-gates`
  asserting the CPU arm skips rather than computes.
- **I6 (no silent capability loss).** No imported change may delete an existing kernel or path without
  that deletion being named in the commit message and covered by a test.
  *Pinned by*: I4's assertions — this invariant exists because the previous round violated it.
- **I7 (provenance).** Every ported file/region keeps a `PROVENANCE: kintsugi-engine 8f12baacf (MIT)`
  header naming what was learned.
  *Pinned by*: review + `grep -c PROVENANCE` on the touched files.

## Named tests

| test | proves |
|---|---|
| `test-gdn-raw-gates` (new) | I2, I3, I5 (CPU arm skips) |
| `test-backend-ops -o GATED_DELTA_NET` (existing) | no regression in the op's documented cases (40 cases, 3/3 backends last run) |
| greedy golden reply (96 tokens, fixed prompt) | I1 |
| `n_tokens=8` launch-path assertion + MTP verify run | I4, I6 |
| `/tmp/kt.py` decode A/B, two runs per arm | I1, I7 verdict |
| `-ngl 0` text sanity | I5 |

## What this buys if it works

The generation gap has three named candidates: raw-gate precompute (this plan), `head2.cu` (643 L,
`LLAMA_HEAD2`), and their chunked MMA prefill (dead on sm_86, so not it for us). If raw gates reconcile
and the decode gap does not move, `head2.cu` is the next and last candidate — and we will know it in
one measurement rather than three reverts.

## Operational notes

- The engine is stopped only for a measurement window and restarted before a stage is called done.
- `systemctl stop vitriol-server.service` before any VRAM work — never `pkill` (the unit is
  `Restart=always`; this cost a cycle on 2026-11-25).
- Stage 1 needs no model load and no VRAM beyond a few MB, so it can run with the 27B live — that is
  the first stage on purpose.
