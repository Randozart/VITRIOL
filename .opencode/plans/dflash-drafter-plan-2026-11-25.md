# DFlash2 drafter: decide before fixing (D1-D3)

Status: PLAN CONFIRMED, executing.
Research: `dflash-drafter-cost-2026-11-25.md` (findings, citations).
Parent: `kintsugi-speed-and-capacity-2026-11-25.md` §6/§6.1 (track A), open task 11.

## 1. What is actually unknown

Track A currently rests on two numbers that contradict each other:

| instrument | reading |
|---|---|
| `VITRIOL_SPEC_TIMING` (wall clock around `draft()`) | 234.69 ms/step at n_max 3 |
| `GGML_CUDA_OP_TIMING` (per-node GPU events) | 0.96 ms/graph, drafter shapes, `cols=4` |

And one arithmetic coincidence that is too exact to ignore: the single-card dflash load fails
allocating `5,564,565,504 B` at `-ub 128` and `5,442,884,096 B` at `-ub 32`, which divide by
`4 × n_vocab (200,704)` to **6,931.9** and **6,780.0** — an integer multiple of the vocabulary, in a
code path (`src/models/dflash.cpp:485`, `:936`) that materialises a dense
`f32 [1, n_vocab, n_blocks]` tensor for the `d2t` draft→target vocabulary scatter.

So this plan does **not** start with a fix. It starts by measuring which of the two numbers is real
and which tensor is 5.4 GB. Previous work in this area produced two refuted hypotheses (the Escha
gemv path, the `process()` waste) — the cost of one more measurement is far below the cost of
another refuted fix.

## 2. Stages

- **D1 — name the 5.4 GB tensor. No code.** One dflash load with `VITRIOL_GRAPH_DUMP=1` (already in
  the fork, `8356473`). Deliverable: tensor name, dtype, dims, and the op that consumes it. This
  also closes open task 11 by itself.
- **D2 — split the `draft()` timer.** Three timers inside `common/speculative.cpp:1274-1441`
  behind the existing `VITRIOL_SPEC_TIMING`: (a) `llama_decode(ctx_dft, batch)`, (b) the
  `embd_nextn` lattice fetch, (c) the post-decode argmax/truncation. Deliverable: a three-number
  split per step, plus the same for `process()`.
- **D3 — one fix, chosen by D1/D2.** Branch table, decided by evidence, not preference:
  - if `llama_decode(ctx_dft)` is ~12 ms and the lattice fetch or a sync owns the rest → the 58 ms/row
    claim was a measurement artefact; remove the per-step sync or the oversized host copy.
  - if `llama_decode(ctx_dft)` really is ~230 ms → the drafter's graph is the cost; D1's tensor
    tells us whether the `d2t` full-vocab materialisation is inside it, and that is the first thing
    to remove.
  - if both are small → the cost is elsewhere in `common_speculative_draft()` and the next stage is a
    finer split, not a fix.

## 3. Invariants

- **I1 (speculation does not change output).** For a fixed argv, the token stream of a greedy request
  is byte-identical before and after every stage in this plan. Timing instrumentation and any `d2t`
  change must not alter what the model produces.
  *Pinned by*: the 8-prompt parity corpus (`/tmp/i7.py`), which must stay at **6/8 byte-identical or
  better**, plus a greedy 96-token code prompt compared byte-for-byte against the pre-change build.
- **I2 (no OOM, honest failure).** Single-card dflash either loads, or refuses with the computed
  maximum rather than dying inside `cudaMalloc`. A stage may not convert a working dual-card load
  into a failing one.
  *Pinned by*: the single-card load attempt captured verbatim (`failed to allocate CUDA0 buffer of
  size N` vs a clean verdict), and a dual-card load still healthy in the same session.
- **I3 (acceptance preserved).** Draft acceptance at n_max 3 stays at **≥ 0.99** (today 1.00000,
  149/149) and mean draft length ≥ 3.9.
  *Pinned by*: `draft acceptance = …` line grepped from the run log, recorded per stage.
- **I4 (instruments are off by default).** `VITRIOL_SPEC_TIMING` and `VITRIOL_GRAPH_DUMP` unset must
  leave argv, fingerprint and token stream untouched.
  *Pinned by*: `vitriol serve --dry-run` argv/fingerprint diff against `HEAD`, and the corpus run
  with both variables unset.
- **I5 (no silent measurement claim).** Every number in the stage record cites the log line or counter
  that produced it. A stage that cannot show its number does not count as done.

## 4. Tests that prove the work

| test | proves |
|---|---|
| `/tmp/i7.py` corpus, both builds | I1: identical token streams |
| greedy 96-token prompt, byte diff | I1 at token level |
| `draft acceptance` line per run | I3 |
| `vitriol serve --dry-run` diff | I4 |
| single-card dflash load, verbatim log | I2 |
| D2's three-way split | the attribution itself (I5) |

## 5. Out of scope

`--spec-draft-window`, `--spec-draft-n-max-tail`, and `LLAMA_HEAD2`. D1 may make `LLAMA_HEAD2` the
obvious follow-on (it exists for exactly the 200k-row head problem); adopting it is a separate
decision, not part of this plan.

---

## 6. Executed (D1, D2)

### D1 — the 5.4 GB is the TARGET's prefill reserve, not the drafter

`VITRIOL_GRAPH_DUMP` (`src/llama-context.cpp:2549`) on the same model and flags:

| flags | pp reserve graph | largest node |
|---|---|---|
| no drafter, `-c 32768 -ub 128` | **4,055 nodes** | 18.0 MiB (`SET_ROWS cache_v_l15`) |
| + dflash n_max 3 | **4,295 nodes** (+240) | **48.1-51.0 MiB** `GATED_DELTA_NET [6144, 2052..2176]` |

So the failing allocation is not a single tensor and not the drafter's context: it is the **target's**
pp compute reserve, whose GDN activations grow to ~48-51 MiB each. 5.44 GB / 51 MiB ≈ **107 live GDN
activations**. With the drafter present the graph is built for `n_tokens = ubatch + n_max + 1` (2052
and 2176 observed across fit iterations), which both grows each tensor and adds 240 nodes.

**Task 11's answer: the single-card dflash blocker is the target's block-draft-inflated prefill
reserve, not the drafter's 5-layer context.** The fix shape is therefore "do not size the prefill
reserve for the verify batch" rather than anything in `dflash.cpp`.

### D2 — the split: the drafter's own decode is 110 ms, and the old 234 ms reading was wrong

`VITRIOL_SPEC_TIMING=1` with three timers, dual card, c 8192, `-ub 128`, n_max 3, 200 tokens:

| phase | ms/step |
|---|---|
| `draft_decode` — `llama_decode(ctx_dft, batch)` alone | **109.64** |
| `draft_lattice` — `llama_get_embeddings_nextn` | **4.70** |
| `spec_draft` — whole `draft()` | **114.35** |
| `spec_process` | 1.88 |

Decode 10.72 t/s, acceptance **1.00000**, mean len 3.98.

Two conclusions that change the plan:

1. **The previous 234.69 ms was wrong.** With the timers split, `draft()` is 114 ms. The earlier
   figure was measured before the helper moved above its first use — the honest statement is that the
   first number was not reproducible, and `spec_draft` is ~114 ms. A3's "58 ms per drafter row" is
   therefore also wrong: it is **~27 ms/row** (109.64 ms for 4 rows), against ~12 ms of weight
   streaming for the whole batch.
2. **The per-row cost is not specific to the drafter.** From the speed table: the target's verify is
   ~257 ms for 5 rows (~51 ms/row, marginal ~33 ms/row) and the drafter's decode is ~27 ms/row. A
   65-layer GDN model and a 5-layer conv model show the *same* per-row term. That points at a
   **per-row sequential dependency** (GDN recurrence / conv taps) rather than weight streaming or
   vocab work — a latency-bound kernel chain, not a bandwidth problem.

### D3 — no evidence-supported cheap fix (see §7)

## 7. D3 verdict: no evidence-supported cheap fix

The plan's branch table said "if `llama_decode(ctx_dft)` really is ~230 ms → the drafter's graph is
the cost". It is 110 ms, so that branch does not apply, and nothing in D1 or D2 names a defect. What
they do establish is a *property* of this fork:

| model | layers | per-row cost |
|---|---|---|
| Kintsugi target (verify) | 65, 48 of them GDN | ~33-51 ms/row |
| DFlash2 drafter (decode) | 5, conv-augmented | ~27 ms/row |

A 13x difference in layer count produces a ~1.2x difference in per-row cost. That is the signature
of a **per-row sequential dependency** — a latency-bound chain of small dependent kernels that both
architectures share (GDN recurrence, conv taps) — not of weight streaming (which the drafter would
put at ~12 ms per *batch*, not per row) and not of vocabulary work (which would scale with neither).

Consequences:

1. **A3's original target is void.** "Reduce the drafter's 58 ms/row to its weight floor" rests on a
   number that was not reproducible (114 ms, not 235) and on an attribution that D2 disproves.
2. **The ceiling is set by the per-row term, in both directions.** A step is
   `draft(≈27 ms × rows) + verify(≈33 ms × rows)`, and each row yields ~1 token (acceptance 1.00000),
   so speculation is structurally near break-even. That is why every configuration lands in the same
   7-17 t/s band regardless of n_max.
3. **The real fix is architectural**: make the per-row recurrent/conv path batch across rows. That is
   a project in the hottest part of a frozen fork, not a stage in this plan, and it would need its own
   research pass (how the GDN scan is built, whether it is row-unrolled, whether a chunked/tree scan
   is available in the fork's `ggml` ops).

What I deliberately did **not** do: hack the prefill-reserve allocator (D1's finding) blind at the end
of a session to make single-card dflash fit. It would touch `llama-context.cpp`'s reserve sizing,
which every model depends on, and the honest statement is that it needs its own plan with its own
invariants — not a late-session edit.

## 8. State after D1/D2

- Shipped: the three-way `VITRIOL_SPEC_TIMING` split and the graph-dump evidence (`3b94ba6`).
- Retracted: "234.69 ms draft / 58 ms per drafter row". Corrected to 114.35 ms / ~27 ms per row.
- Answered: open task 11 — single-card dflash is blocked by the **target's** block-draft-inflated
  prefill reserve (~107 live GDN activations), not by the drafter's context.
- Verified: I4 (fingerprint and argv carry no timer fields with the env unset), I3 (acceptance
  1.00000, mean len 3.98 reproduced), I2 (no change in load behaviour), I1 (no tokens affected —
  the timers only read a clock).
- Daily driver restored and verified after every window: `active`, `/health ok`, 10,164/10,106 MiB.
