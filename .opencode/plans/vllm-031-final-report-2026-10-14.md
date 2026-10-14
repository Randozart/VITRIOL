# vLLM 0.31 -> VITRIOL integration: final report (2026-10-14)

Branch `vllm-031` (outer + inner), merged to `main` same day.
Survey source: vLLM 0.31.0 at
`~/.cache/uv/archive-v0/9InspFUO35OuRAn_/vllm` (commit `gdb9527a46`,
Apache-2.0 - nothing copied verbatim; ideas only, per the licensing table).
Plan doc: `vllm-031-integration-plan-2026-10-14.md` (stages + results log).
This file is the exhaustive did/didn't ledger.

## Merge manifest

Inner `main` fast-forwarded `f5fbc4e44 -> 4da6708ce` (8 commits). Outer
`main` gained the launcher commit + 7 docs commits (`af9699e..4aa854e`).

| commit | what | status after validation |
|---|---|---|
| `589443873` | acceptance-breakdown metrics (offered per position + verify histogram + labeled families) | WORKED as shipped; 10/10 + 10/10 unit suites |
| `f95a61881` | fail-closed draft-ctx coverage guard | worked only after `d7f4b5cb7`; see UB below |
| `ab8d1b374` | /metrics refresh at task boundary | WORKED; fixes a PRE-EXISTING main bug (proven: failed on the 10-12 production binary too) |
| `eafd15ca1` | TQ3_0 fattn port | WORKED; correctness restored (was silently degraded since 09-02) |
| `17c2e74cb` | quantized-KV small-batch VEC path | worked only in-regime; needed `4da6708ce` |
| `4da6708ce` | gqa<=2 gate for the VEC path | WORKED; live-validated |
| `d7f4b5cb7` | set_draft_ready UB fix | WORKED; MTP crashes gone on real model |
| `86137a81d` + outer `af9699e` | adaptive draft-K + launcher key | WORKED; opt-in, default off, validated inert |

NOT merged / NOT attempted (deliberate):

- Stage 4 (content-addressed prefix cache, eviction, disk hardening) -
  not started. The intended win is restart warmth + cache thrash, not tps.
- Stage 5 (per-shape CUDA-graph capture cache + capture-hit counter) -
  not started. Analysis (see plan doc) suggests the graph map keyed by
  first-node pointer thrashes when verify batch shapes alternate; fix
  would be shape-keyed graph instances in ggml-cuda.
- vLLM 1c (startup plan / flag gate): NO-OP - `scripts/vitriol`
  fingerprint journal already exceeds it.
- 3.1 residual-mass sampling: NO-OP by source analysis - the sampler
  accepts iff y == draft (exact by construction).
- 3.3 multi-module MTP rotation: NO-OP - live checkpoint reads
  `qwen35.nextn_predict_layers = 1`.

## What worked (with evidence)

1. **/metrics staleness fix.** The 1s throttle on the post-update_slots
   refresh left served counters at boot zeros forever after the last
   completion (queue only runs update_slots while tasks flow). Symptom
   reproduced on the production binary; unit `test_metrics_prometheus_format`
   went red -> green. Every dashboard consumer benefits daily.
2. **Acceptance telemetry.** Per-position offer counts + draft_len x
   accepted histogram as labeled metrics; denominator fixed (offered
   counted at drafting, not verify). Contract test
   `test_metrics_acceptance_breakdown` enforces offered >= hist.
3. **TQ3_0 flash-attention port.** Root cause chain established by
   source archaeology: GPU tq3_0 KV is raw-domain end to end (set_rows
   quantize / convert dequant / MMVQ+MMQ centroid-direct); fattn
   rejected TQ3 -> FA op fell back to CPU whose rotated-domain dequant
   clashed with GPU bytes = the 09-02 "TQ3 V-cache KNOWN-BROKEN"
   corruption. Port needs no WHT anywhere. Verified: set_rows-based
   harness max|FA(tq3) - CPU-ref-over-device-dequant| <= 0.0023 (5
   shapes x 2 devices); tinygemma3 e2e identical text across
   f16/q4_0/tq3_0 KV; FLASH_ATTN_EXT suite 5921 OK / 0 FAIL. Value =
   correctness restore + 22% KV-memory option (not speed).
4. **Fail-closed coverage guard + UB fix.** Guard intent (disable
   drafting when the draft ctx cannot cover the prompt) survived; the
   first implementation had a real bug (see didn't-work). Live 27B gate
   green after fix: f16/q4_0/tq3_0 all 200 with drafting live.
5. **Adaptive draft-K.** `--spec-adaptive-k` / `[spec] adaptive_k`:
   trim K where per-position acceptance < 0.125 (32-offer gate, 128-round
   re-exploration). Validated inert on real workloads (acceptance
   0.56-0.69) - by design; it is insurance for low-acceptance regimes.
   Rides the fingerprint as conditional `specak=1`.
6. **VEC quantized-KV path, gated gqa<=2.** Microbench at its firing
   regime (nk=40960, D=128): nq=2 beats MMA+conversion by +58% (gqa1)
   / +46% (gqa2); nq=4 ties. Never fires on qwen3.8 (gqa 4). Kept as
   option value for small-kv-head models.

## What did not work / was disproven

1. **The +19-20% tps claim.** Ladder sessions showed 25.4 vs 21.4
   control at 40k. The clean same-tree control (A/B/C: HEAD 21.98, base
   21.57, production 21.49) killed it - all within 2.3%. Re-confirmed on
   the live profile (daily-driver triangle within ~2% at 20k/40k/80k).
   The commits are PERF-NEUTRAL on the daily driver. There is no
   performance-based merge argument.
2. **VEC at gqa>=4.** First version routed MTP verify (nq=4) to the vec
   kernel unconditionally: tq3_0 collapsed to 6.3 t/s at 40k (each
   q-head block re-reads its kv stream through the scalar kernel). The
   microbench that motivated it ran gqa=1 and over-generalized. Fixed
   by the gqa<=2 gate; kept only where it wins.
3. **set_draft_ready UB (self-inflicted).** The coverage guard's first
   implementation called `erase(end())` on the empty seq_draft_bad
   vector on every HEALTHY request - UB, libstdc++ decremented
   _M_finish to nullptr-4, and the next draft_ready walked {0x0,-4}
   into SIGSEGV. Every real MTP request died; the unit suite stayed
   green because draft-simple never calls the helper. Caught by the
   live 27B gate. Lesson: spec unit tests had ZERO MTP/eagle3 coverage
   (still true for eagle3 - test debt noted below).
4. **tq3_0 40k cliff (open).** tq3_0 KV is correct everywhere and fast
   to 20k (25.4 t/s), but falls to 13.6 t/s at 40k filled on the
   MMA+conversion path (q4_0: 25.4). Also prefill lag (268 vs 330 t/s).
   Open follow-up: conversion-buffer scaling or per-tile dequant for
   the tq3 f16 path at large prefixes.
5. **ts retune for 2x3060: no effect.** Live ts 24,24 vs 50,50 on
   identical cards: within noise at every depth. Attention-bound.
6. **The 80k > 40k inversion: corpus artifact.** Every arm sped up at
   80k because the repeated archive text drives draft acceptance to
   100% (47/47) - free tokens, not a depth effect. Do not quote it as
   a speedup.
7. **Microbench->production transfer.** The gqa miss is the case study.
   Microbench run-to-run variance is +/-40% at these scales; only
   large margins are signal, and selection heuristics need the real
   model's GQA topology.

## Methodology lessons (keep for next time)

- `ggml_cpy` returns a NEW op node; expanding the dst leaf builds an
  EMPTY graph (zeros in, zeros out, identical "errors" - the first
  harness validated nothing). Build graphs from the returned op nodes.
- f32->tq3_0 device quantize is SET_ROWS, not CPY (cpy.cu:609 aborts
  both directions; the KV write path is set_rows).
- `timings.prompt_n` is the DELTA prefill count, not cumulative depth.
- The sparse probe gates on fill >= score_min_fill x window (106k on
  the 208k profile) - silently idle below; verify engagement with the
  "probe active" stderr line, not just env presence.
- Clean same-tree controls before any perf claim. The A/B/C triangle
  (file-revert in-place, rebuild, restore) cost ~30 min and killed a
  wrong +19% before it could reach main as a claim.
- Depth ladders on repetitive corpora confound depth with acceptance;
  use mixed text or fix acceptance per arm.
- Hardware note: this box is now 2x RTX 3060 (sm_86, 23.9 GiB total) -
  NOT the 3060+1070Ti duo in AGENTS.md. Pascal/TILE paths untested
  here; live profile ts 24,24 measured fine.

## Open follow-ups

1. eagle3 + MTP unit-test coverage for the spec path (the UB class of
   bug must be catchable in CI, not only on a 27B live gate).
2. tq3_0 40k cliff (above).
3. Stage 4 (warm-cache) + Stage 5 (graph-shape cache) from the original
   plan - the intended restart/cache wins, unattempted.
4. Sparse+fattn under real eviction pressure (window-edge fill or KV
   floor; interacts with the governor RELIEVE path).
5. Production binary rebuild + engine restart with the full
   fingerprint ritual (rebuild kicked 2026-10-14 ~22:40; restart at the
   owner's next natural window - the GPU goes to another agent).
