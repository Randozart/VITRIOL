# Kintsugi parity plan — HEAD2 first, then replay, then gates (2026-11-26)

## Goal

Kintsugi-27B on vitriol-server at **or above** the speed proven on their engine:
**≥ 14.55 t/s no-draft** and **≥ their DFlash2 numbers** (72.8 repeat / 45.1 code class) before the
live engine is restarted. The owner has stopped vitriol-server; it stays stopped until parity.
Research: `kintsugi-what-works-research-2026-11-26.md` (four stacked features, complete inventory).

## Success criterion (measurable)

Same prompts, same machine, same measurement harness (`/tmp/kt.py` class): our build ≥ their binary's
numbers with launcher defaults (HEAD2=512, DFlash2 n7, UB 1024). Not "close" — at or above.

## Stages

### Stage 0 — ablation on THEIR binary (attribute the 14.55; no code)
Runs of `/scratch/kintsugi-ref/Kintsugi-27B/bin/llama-server` on the same single card:
(a) launcher defaults (baseline, HEAD2 on); (b) `LLAMA_HEAD2=0`; (c) `LLAMA_HEAD2=0` + UB 256.
Output: t/s per arm → attributes the gap among HEAD2 / batch effects. Sizes everything after.
*Pin*: numbers recorded in this plan's executed-section before Stage 1 starts.

### Stage 1 — port HEAD2 (the largest, most isolated lever)
Files (PROVENANCE: kintsugi-engine 8f12baacf, MIT):
1. `ggml/src/ggml-cuda/head2.cu` + `head2.cuh` — copy verbatim.
2. `ggml/src/ggml-cuda/ggml-cuda.cu` — 19 head2 hunks: MUL_MAT interception, buft factory, bypass.
3. `src/llama-impl.h` — `llama_head2_candidates()` / `llama_head2_type()`.
4. `src/llama-model-loader.cpp` — OUTPUT Q8_0 → pinned `nostage_head2` buft.
5. `src/llama-model.cpp` — stage-1 Q4_0 copy + pair registration + check mode.
6. `src/models/qwen35.cpp` — optional `LLAMA_HEAD2_DRAFT` stage-1 logits (port but default off).
No graph changes (their invariant: "the graph does not change").
*Tests*: `test-backend-ops` MUL_MAT suite unchanged; new `tests/test-head2.cpp` — two-stage output
vs plain exact MUL_MAT (max rel diff ≤ stage-1 tolerance, plus a negative control: wrong q4 copy
must diverge); golden corpus ≥ 6/8 byte-identical; greedy 96-token reply identical to their binary's
same-prompt reply at temperature 0 is the stretch goal (may differ from OUR pre-port output only in
the documented stage-1 approximation — assert against THEIR output, not ours).

### Stage 2 — decode A/B at parity configs
c 8192 no-draft (HEAD2 on/off) vs their binary same flags. DFlash2 n7 at 4k. Two runs per arm.
*Pin*: the ≥ 14.55 gate. If met without Stages 3-4, they become optional.

### Stage 3 — replay rollback (only if the gate is not met, or DFlash2 lags)
server (`spec_is_replay` path) + recurrent memory pending plane + `ggml_gated_delta_net_replay` op
+ delta-net-base replay call site. Depends on the GDN raw path being correct → the
`gdn-raw-gate-reconciliation-2026-11-26.md` Stage-1 unit test is a PREREQUISITE here.
*Tests*: forced-rejection equivalence (replayed next-forward == cold re-prefill output);
acceptance-rate counters unchanged; golden corpus.

### Stage 4 — launcher parity knobs (cheap A/Bs on OUR binary)
`--kq-mask-packed`, `--fit off`, UB 1024, `[lookup]` composition. Adopt what measures faster.

## Invariants

- **I1 (engine stays down until parity).** vitriol-server.service is NOT restarted until the Stage-2
  gate is met or the owner says otherwise. *Pinned by*: `systemctl is-active` checks in the log.
- **I2 (HEAD2 opt-out).** `LLAMA_HEAD2=0` reproduces pre-port behavior byte-for-byte (same greedy
  reply, same speed class). *Pinned by*: I2 run + golden corpus.
- **I3 (stage-1 correctness).** Two-stage logits select the same top-N as the exact head on random
  and adversarial inputs (ties, near-ties via err2); gathered exact rows are bitwise equal to a plain
  MUL_MAT with the exact head. *Pinned by*: `test-head2.cpp`.
- **I4 (the test cannot lie).** Corrupting the registered Q4 copy or n_cand changes the output.
  *Pinned by*: `test-head2.cpp` negative control.
- **I5 (no silent capability loss).** HEAD2 must not disable or degrade any existing path (Escha,
  tiered KV, chunked GDN, DFlash2): all existing test suites pass unchanged. *Pinned by*:
  `test-escha-mul-mat`, `test-kv-tier`, `test-backend-ops -o GATED_DELTA_NET`.
- **I6 (provenance).** Every ported region carries the kintsugi PROVENANCE header. *Pinned by*: grep.
- **I7 (measurements are paired).** Every A/B: same prompt set, two runs per arm, fingerprint line
  logged. *Pinned by*: log excerpts in the executed section.

## Execution notes

- Engine patches via python3 exact-match scripts (count==1 asserts) — never the edit tool on engine/.
- Build at `engine/build` (→ /scratch symlink); targets: `llama-server`, `test-head2` needs a
  `cmake -B build` reconfigure first (GLOB).
- Their binary for Stage 0/2: `/scratch/kintsugi-ref/Kintsugi-27B/bin/llama-server` port 8298.
- VRAM discipline: single card at a time; `pkill -9 -x llama-server` between arms and verify
  nvidia-smi is desktop-only before the next load (both engines' leftovers poison the next arm).
