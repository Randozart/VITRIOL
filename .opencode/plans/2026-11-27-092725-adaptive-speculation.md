---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T09:27:25+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "169aa16b72a992eae86f66ad0d63c69630353203"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-27-081932-kintsugi-224k-daily-driver.md"]
---

# adaptive-speculation

## Purpose

Choose speculative work that reduces actual response time across context depths.

## Approach and alternatives

Build a bounded controller using accepted tokens, draft/verification time and memory headroom, starting with existing MTP controls.

- Keep the measured current profile unless paired experiments meet the acceptance criteria.

## Scope and documentation

engine/common/speculative.cpp, engine/tools/server/server-context.cpp, launch fingerprints and speculative metrics.

## Verification

- Run the concrete acceptance and regression checks below with exact launch fingerprints and evidence outside the repository.

## Results and evidence

Planned. Record measured outcomes and verification evidence here during execution.

## Change history

- 2026-11-27T09:27:25+01:00 — Plan created before implementation.

## Baseline and hypothesis

The current daily driver uses MTP draft length 3, tail length 4 and lookup.
The 65K rollback uses DFlash2 n7. Prior fixed-depth sweeps favored n7 for their
corpus, and putting the drafter on GPU0 reduced matched coding throughput.
A controller must beat those measured controls on the same workload.

Hypothesis: context and acceptance-sensitive choices reduce total time per
accepted token without oscillation or disproportionate memory use.

## Research and source comparison

Read the existing reference scheduling and length controls first. D3D
(November 2026) removes dependence on target hidden states to overlap drafting
and verification. Its trained drafter is a separate research branch; establish
whether a compatible checkpoint exists for Kintsugi before planning adoption.
SpecStream motivates target-priority overlap on constrained GPU memory.

- [D3D paper and code](https://arxiv.org/abs/2611.06425)
- [SpecStream](https://arxiv.org/abs/2609.33184)

## Implementation stages

1. Record per-round elapsed time, proposed/accepted tokens, verification size,
   GPU margin and output position for current MTP and fixed DFlash controls.
2. Offline replay a small policy choosing among supported draft lengths.
   Optimize elapsed time per committed token with bounded probing cost.
3. Add per-slot state, hysteresis, a minimum observation count and a maximum
   exploration budget. Begin with MTP length selection on the current model.
4. Test depth changes, low acceptance, interruptions and memory pressure.
   Treat loading/unloading an external drafter as a separate measured stage.
5. Investigate asynchronous drafting only if compatible model weights,
   runtime support and the timing trace justify it.

## Invariants

1. Disabled control reproduces the fixed-profile choices. Pin with a golden
   decision matrix and launch-fingerprint comparison.
2. Target verification determines committed output. Pin with rejection and
   sampling tests; require operational correctness rather than byte-identical
   prose where valid floating-point or stochastic variation applies.
3. Decisions stay within supported memory and draft bounds, including during
   cancellation and restore. Pin with pressure and malformed-metric fixtures.
4. Per-slot observations cannot leak into another slot's active state. Pin
   with alternating requests and restored sessions.

## Acceptance and falsification

Measure fresh code, edits and prose at filled 4K, 64K and 200K. Include time to
first token, full reply time and controller overhead. Provisional target:
10% lower median full-turn latency than the best fixed control per depth,
with no more than 5% regression on any representative workload. If choosing a
fixed setting wins once measurement and switching costs are counted, retain it.
D3D performance claims on other Qwen models remain unverified here.

## Dependencies and rollback

Telemetry first; streamed KV overlap is a later integration point. Keep the
fixed-profile override available and fingerprint all speed-bearing choices.

## Evidence and delivery

Status is planned. Implementation and performance are unverified. Record per-stage commits, full argv, model/runtime identity and actual filled token counts. Use the repository verification gates, retain existing diagnostics and document unavailable checks. Save results before the final verification freeze. Public research code requires license review and attribution before incorporation. Live maintenance requires an idle or operator-authorized window.

## Related implementation plans

- [cache-continuity](2026-11-27-092725-cache-continuity.md)
- [streamed-kv-attention](2026-11-27-092725-streamed-kv-attention.md)
- [selective-historical-kv](2026-11-27-092725-selective-historical-kv.md)
- [inference-dashboard-telemetry](2026-11-27-092725-inference-dashboard-telemetry.md)
