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

# selective-historical-kv

## Purpose

Test whether selecting historical KV pages improves long-context coding latency without unacceptable quality loss.

## Approach and alternatives

Prototype opt-in page selection and coalesced transfers against the full-attention reference, retaining a dense fallback.

- Keep the measured current profile unless paired experiments meet the acceptance criteria.

## Scope and documentation

engine/ggml/src/ggml-cuda/fattn.cu, KV tier metadata, admission policy, quality fixtures and provenance.

## Verification

- Run the concrete acceptance and regression checks below with exact launch fingerprints and evidence outside the repository.

## Results and evidence

Planned. Record measured outcomes and verification evidence here during execution.

## Change history

- 2026-11-27T09:27:25+01:00 — Plan created before implementation.

## Baseline and hypothesis

The daily profile uses full attention with tiered KV. The reference source
contains experimental sparse attention mechanisms that must be inventoried
before adding a second selector. Prior needle success does not establish
coding quality when attention drops pages.

Hypothesis: query-relevant page selection and larger contiguous transfers can
reduce historical KV I/O enough to improve 200K coding response time.

## Research and source comparison

Read PulseInfer's page selection, I/O coalescing and admission design. Compare
its assumptions to the local reference selector, quantized KV, hybrid recurrent
state and PCIe topology. Inspect code and license availability before reuse.
The paper describes near-lossless accuracy; approximate selection still needs
its own evaluation on VITRIOL tasks.

- [PulseInfer](https://arxiv.org/abs/2609.34555)

## Implementation stages

1. Capture a full-attention quality and timing control at 64K and 200K.
   Inventory reference page bounds, selectors and existing experimental knobs.
2. Add or reuse a bounded selector for historical attention pages. Keep recent
   tokens and explicit sink pages, and report selected bytes per layer.
3. Coalesce selected pages into fewer transfers and overlap only where event
   ownership is already verified. Include index-building cost in timings.
4. Detect missing metadata, stale bounds and unsupported graph shapes and use
   dense attention. Rebuild selection metadata after slot edits or restore.
5. Evaluate an opt-in experimental profile with a recorded attention budget.

## Invariants

1. Disabled selection uses the current full-attention path. Pin with dispatch,
   numerical and lifecycle regression checks.
2. Selection respects causal masks and never reads another session's state.
   Pin with page-boundary and interleaved-slot fixtures.
3. Unknown or stale metadata selects the dense path. Pin with save/restore,
   earlier-turn edits, truncation and invalidation tests.
4. Every report exposes that approximate attention was active and its budget.
   Pin through fingerprints and emitted metrics.

## Acceptance and falsification

Run exact function copies, facts distributed across the entire context,
conflicting updates, system rules and complete Officina coding tasks with
independent tests. Use multiple placements and repeats, plus the full-attention
control. Provisional target: 20% lower 200K full-turn latency, zero additional
failures on exact-copy/instruction fixtures and a documented task-score
noninferiority margin set before evaluating the held-out tasks. Reject default
promotion if quality worsens beyond that margin or selection costs erase gains.

## Dependencies and rollback

Telemetry and a stable dense streaming baseline come first. Preserve complete
historical KV so a request can fall back to dense. Enable through a distinct
experimental profile and retain the tested daily profile for rollback.

## Evidence and delivery

Status is planned. Implementation and performance are unverified. Record per-stage commits, full argv, model/runtime identity and actual filled token counts. Use the repository verification gates, retain existing diagnostics and document unavailable checks. Save results before the final verification freeze. Public research code requires license review and attribution before incorporation. Live maintenance requires an idle or operator-authorized window.

## Related implementation plans

- [cache-continuity](2026-11-27-092725-cache-continuity.md)
- [streamed-kv-attention](2026-11-27-092725-streamed-kv-attention.md)
- [adaptive-speculation](2026-11-27-092725-adaptive-speculation.md)
- [inference-dashboard-telemetry](2026-11-27-092725-inference-dashboard-telemetry.md)
