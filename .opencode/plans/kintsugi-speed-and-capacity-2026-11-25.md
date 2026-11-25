# Kintsugi on our engine: speed (A) and capacity (B) — combined plan, 2026-11-25

Status: PLAN. Two independent tracks, one shared measurement harness, explicit sequencing.
Related: `kintsugi-vitriol-port-2026-11-24.md` (§4c for the measured baseline),
`kv-tier-vram-cells-2026-11-25.md` (§B detail — this document supersedes its §4 stage numbering).

## 0. Why two tracks, and the honest split

| | track A — speed | track B — capacity |
|---|---|---|
| goal | close the 1.66x no-draft / 4x dflash gap | 262k native on **one** card, DFlash2 on one card |
| lever | per-token cost of a speculative step | KV placement (VRAM head + host tail, CUDA VMM) |
| measured today | 8.80 t/s plain, 17.52 best (dflash n_max 7, 2 cards) | 2 cards only; single card cannot load 262k |
| their reference | 14.55 plain, 72.82 repeat / 45.11 code with dflash | `-c 262144` on one card |
| touches | `escha-gemv.cu`, `common/speculative.cpp`, `ggml-cuda.cu` (graph loop) | `ggml-cuda.cu` (allocator), `llama-kv-cache.cpp`, launcher |
| risk to frozen fork | medium (hot path, but contained) | medium (new allocator; **no** attention change) |

They are orthogonal: B does not raise t/s (at depth it lowers it, physics not implementation —
`.opencode/plans/kv-tier-vram-cells-2026-11-25.md` §5), and A does not add capacity.

## 1. Track A — where the step time actually goes

Baseline decomposition already measured (dual card, `-ub 128`, repetition prompt):

| n_max | tok/s | mean draft len | ms/step | rows verified |
|---|---|---|---|---|
| 1 | 7.60 | 2.00 | 263 | 3 |
| 2 | 8.28 | 3.00 | 362 | 4 |
| 3 | 11.65 | 3.97 | 341 | 5 |
| 5 | 15.23 | 6.00 | 394 | 7 |
| 7 | 17.52 | 4.73 | 270 | ~6 |

Marginal **~33 ms per verified row** against 114 ms for a standalone token; **step base ~165 ms**
against 114 ms for one token.

New evidence from source (this session):

1. **It is not the Escha projections.** `escha-gemv.cu:479` chunks rows at `EXL3_GEMV_MAX_M` with
   `mmode = m == 1 ? 0 : 1`, so a 4–8 row verify takes the `MMODE=1` tile. Forcing the MMA prefill
   path instead (`GGML_ESCHA_GEMV=0`) measured **324 ms/step vs 341 ms/step** — within noise of each
   other, so the codec accounts for ~17 ms of a ~200 ms excess. The cost scaling with rows lives
   elsewhere.
2. **The drafter is decoded once per drafted token.** `common/speculative.cpp:319-402`: a
   `while (n_drafting > 0)` loop with `llama_decode(ctx_dft, batch)` inside it. Each iteration is a
   full forward of the 1.09 GB DFlash2 model. At n_max 7 that is up to seven drafter passes per step.
3. **The drafter's encoder *and* decoder run on the full verify batch**, and the fork's own comment
   says so as waste: `speculative.cpp:452-458` — "process() runs encoder + decoder on the *full* verify
   batch including rows for ... in verify mode, have process() only stash features and let draft() seed
   run". So per-verified-row cost plausibly includes drafter work proportional to rows.
4. **Their engine has features around exactly this** that we lack: `--spec-draft-window`,
   `--spec-draft-n-max-tail` (README: "the model's own MTP head at every depth"). A windowed drafting
   policy reduces how many rows/iterations are spent per accepted token.

**Conclusion A**: the fork's own TODO at `speculative.cpp:452` names the suspected waste, and the
drafter-per-draft-token loop is measurable but unquantified. The next step is measurement, not code.

### A-stages

- **A1 — port the op timer (diagnostic, reusable).** Their engine prints per-node GPU time behind
  `GGML_CUDA_OP_TIMING=1` (`ggml-cuda.cu:5908`, ~20 lines: an event before each node in the graph
  launch loop, charged to that node, printed as a histogram at the end). **Our fork has no such
  switch** — this is the third time a counter would have settled a question that reasoning got wrong.
  Port it behind the same env name. MIT, small, and useful forever.
- **A2 — localise the step.** With graphs off (`GGML_CUDA_USE_GRAPHS=0`) run three arms:
  no-draft, dflash n_max 3, dflash n_max 7, and read the histogram. Attribute the 165 ms base and the
  33 ms/row between: target projections, attention, output head, drafter encoder/decoder, drafter
  draft loop, sampling/acceptance. **No fix is proposed until this table exists.**
- **A3 — fix what A2 names.** Likely one of:
  - draft-once-then-verify batching (seed the drafter with the accepted rows instead of a forward per
    drafted token);
  - windowed drafting (`--spec-draft-window` equivalent) so the draft chain is bounded;
  - stop the drafter's encoder+decoder running over verify rows (the fork's own TODO);
  - only if A2 says so: route 2–16 row verify to the MMA tile permanently.
- **A4 — re-certify**: acceptance must stay ≥ the current 1.00000 at n_max 3 (a faster step that
  accepts less is not faster), and the parity corpus must stay 6/8 byte-identical.

**Predicted ceiling**: if per-row cost went to zero, a 7-row step is ~165 ms ≈ 48 t/s; the step base
must also fall to reach their 72.8. State this as a prediction, not a promise — A2 decides whether the
base is even attackable.

## 2. Track B — tiered KV

Full detail in `kv-tier-vram-cells-2026-11-25.md`: mechanism read from their source (one
`cuMemAddressReserve` range, per-granule VRAM or pinned-host backing, `cuMemMap` + `cuMemSetAccess`,
kernels unchanged), our gap (their `fattn.cu` is 1714 lines vs our 766; their staging assumes a chunked
attention loop we lack), and the resulting decision: **stage 1 ports the memory half only, no
`fattn.cu` change**, because device-addressable host pages are correct by construction and staging is a
bandwidth optimisation.

Stage numbering (supersedes the earlier document's §4):

- **B0 — driver probe**, ~60 lines standalone, no repo change: does driver 620.73 accept
  `cuMemCreate` with host location types here. Their code requires driver 580+. **Gate: if B0 fails,
  track B stops and we say so** rather than building an allocator that cannot map host memory.
- **B1 — tier buffer type** in `ggml-cuda.cu` (host-memory pool helper, VA reserve, per-granule
  backing, `cuMemMap`/`cuMemSetAccess`, fault-safe undo, proc-address export, `GGML_CUDA_NO_VMM` →
  ordinary buffer).
- **B2 — plumbing**: `n_kv_vram_cells` cparams → `llama_kv_cache` ctor → the 24-line buffer-selection
  hook; `--kv-vram-cells` in `common/arg.cpp` and the server.
- **B3 — launcher**: `[kv] vram_cells`, `0` = flag omitted (byte-identical argv), positive rides the
  fingerprint as `kvc=N`; auto-size reuses `vitriol serve --plan` per-card free space rather than a
  second copy of their `cells_for` formula.
- **B4 — certification**: port `tests/test-kv-tier.cpp` (no-staging variant) + negative controls.
- **B5 — measure at depth** and decide whether their staging/chunked-FA half is ever needed (§5 there).

## 3. Sequencing

1. **B0 and A1 in parallel** — both are small, both are gates, neither touches the other's files.
2. **A2 → A3** while B0's verdict is fresh; A is the faster path to a usable daily driver because it
   multiplies every configuration rather than one of them.
3. **B1 → B2 → B3 → B4** as a block afterwards: B1 is a large single-file edit to `ggml-cuda.cu`, and
   doing it after A's graph-loop work avoids two people editing the same region of the same file.
4. **B5** last: it needs a quiet machine (single-card loads) and a restored daily driver afterwards.

Conflict management: A1 inserts an event pair inside the graph launch loop in `ggml-cuda.cu` (~line
4800); B1 appends a new buffer type near the pool code (~line 700) and the registration table
(~line 6181). Disjoint regions, but land A1 first so B1 rebases onto stable code.

## 4. Shared invariants

Track-specific invariants live in the tier document (I1–I9). Shared across both:

- **S1 (parity).** A profile that names neither `vram_cells` nor a new spec knob emits byte-identical
  argv and an unchanged fingerprint. *Pinned by*: launcher argv/fingerprint diff test against `HEAD`.
- **S2 (no quality regression).** Any speed or capacity change must keep the 8-prompt parity corpus at
  **≥ 6/8 byte-identical** and draft acceptance at n_max 3 at **≥ 0.99**.
  *Pinned by*: `/tmp/i7.py` corpus run + acceptance line grep, both re-run in the stage's evidence.
- **S3 (operator safety).** The live engine is stopped only for a load window and is restored and
  verified `/health ok` before a stage is called done. *Pinned by*: the stage's evidence block quoting
  `systemctl is-active` + `/health`.
- **S4 (attribution).** Every ported block carries `PROVENANCE: kintsugi-engine 8f12baacf (MIT)` with
  the source file, as the Escha port does. No silent transplants.
- **S5 (measurement over inference).** A claim of speedup or capacity in a report cites a counter or a
  log line, not arithmetic. This session produced two counter-examples to arithmetic — the gemv path
  question and the "staging is required" reading of their README.

## 5. What we will know that we do not know now

1. Whether host-location VMM mapping works on this driver (B0, hours).
2. Which phase owns the 165 ms step base and the 33 ms/row (A2, one instrumented run).
3. Whether single-card 262k is usable at depth or only at shallow depth (B5) — and therefore whether
   their chunked-FA rewrite is ever on the table (a decision for the owner, not an assumption).
4. Whether their 72.8 t/s is reachable with our drafter or depends on their drafting policy
   (`--spec-draft-window`), which we have not read yet.

Items 2 and 4 are the ones that change what Kintsugi *is* for this workstation: at 48–72 t/s it is a
credible daily driver at 16–25 t/s it competes with the existing 27B and mostly wins on capacity.

---

## 6. A2 EXECUTED: the step is the drafter, not the codec and not verify

Two instruments were built. The first (`GGML_CUDA_OP_TIMING`, ported from their engine) does not
work well here: it only arms when a drafter is active, and its totals under-report (0.96 ms/graph
against a 340 ms step, with no `ESCHA_MUL_MAT` in the top 30) because our fork forks work onto
secondary streams. It is kept as a diagnostic with that limitation documented.

The second is a host-side timer around the two speculative choke points,
`common_speculative_process()` and `common_speculative_draft()` (`VITRIOL_SPEC_TIMING=1`). It settles
the question outright:

| n_max | decode | `spec_process` | `spec_draft` | mean draft len |
|---|---|---|---|---|
| 3 | 6.16 t/s (timing on) | **2.95 ms** | **234.69 ms** | 3.98, acceptance **1.00000** |
| 1 | 7.17 t/s | — | **67.52 ms** | 2.00 |

A step at n_max 3 is ~340 ms. Of that, **235 ms is `draft()` and 3 ms is `process()`**. The target's
own multi-row verification is the small remainder.

And `draft()` is linear in draft depth: one drafted token costs 67.5 ms, ~3.5 drafted tokens cost
234.7 ms. **A single forward of the 1.09 GB DFlash2 model therefore takes ~67 ms — about 16 GB/s,
six times worse than the target's own ~90 GB/s streaming rate, and 5.5x worse than its weights
require (~12 ms).**

This kills the three hypotheses I had been carrying:

- not the Escha projections (both gemv and MMA prefill give the same step time),
- not the small-batch verify path (that was the earlier guess: it is ~100 ms at most),
- not the documented `process()` waste (2.95 ms — irrelevant), despite the fork's own TODO.

**A3 is therefore: make one drafter forward cost its weight-streaming time (~12 ms) instead of
67 ms.** If it did, a 4-token step would be ~48 + ~130 = ~178 ms ≈ 22 t/s, and at n_max 7 with mean
len 7.95 (already measured at 14.27 t/s with the timer on) the ceiling is higher still. Candidates,
in order of likelihood, none yet tested: the drafter's shared 200k output head being computed over a
dequantising path rather than the coded gemv; per-step graph re-reservation; the drafter's KV at
c 8192 being re-read cold each step.

**Acceptance is 1.00000 (149/149)**, so S2's quality half holds while we look at speed.

### 6.1 A3 refined by reading the draft loop

`draft()` (`common/speculative.cpp:1274`) does **not** run one forward per drafted token. It builds a
single batch holding the whole noise block (`n_draft + 1` masked rows) and issues **one**
`llama_decode(ctx_dft, batch)`. So the measurement reads as a per-row cost:

| draft rows | `spec_draft` | per row |
|---|---|---|
| 2 (n_max 1) | 67.52 ms | **33.8 ms** |
| 4 (n_max 3) | 234.69 ms | **58.7 ms** |

The drafter holds 1.09 GB of weights, so its weight streaming is ~12 ms; at 4 rows it costs 235 ms,
i.e. ~46 ms per row of *non-weight* work. The drafter also cannot load standalone — it refuses
without `ctx_other`, because it is architecturally a two-context model (encoder over target hidden
states + decoder). The per-row term is therefore almost certainly the sequential per-row encoder /
`compute_f_rnn` path (`speculative.cpp:959`), not the projections.

**Why this matters more than it looks**: one drafter row costs ~58 ms and yields about one token; the
target produces a token in ~57 ms. Speculation here is close to break-even by construction, which is
exactly what the flat throughput shows (n_max 1 → 7.17, 3 → 11.65, 7 → 14.27-17.5 t/s, as against
8.73 undrafted). Their engine at 72.8 t/s must be running that per-row path far cheaper.

**A3 target, now specific**: reduce the drafter's per-row cost from ~58 ms toward its weight-streaming
floor. Not a kernel-tuning problem — a per-row sequential depth in the draft context. Next step is to
time the pieces inside `draft()` (encoder rows, `compute_f_rnn`, the single `llama_decode`, the
sampler) with the same `VITRIOL_SPEC_TIMING` pattern.
