---
schema: "codex-work-plan/v1"
created_at: "2026-11-27T09:27:25+01:00"
timezone: "Europe/Amsterdam"
status: "active"
baseline_commit: "169aa16b72a992eae86f66ad0d63c69630353203"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-27-081932-kintsugi-224k-daily-driver.md"]
---

# inference-dashboard-telemetry

## Purpose

Show the causes of coding latency and supply trustworthy measurements for cache and inference experiments.

## Approach and alternatives

Add per-request and per-slot cache, first-token, restore, speculation and transfer metrics using existing polling and runtime adapters.

- Keep the measured current profile unless paired experiments meet the acceptance criteria.

## Scope and documentation

vitriol-tui/src/model.rs, poller.rs, ui.rs, engine metrics and Officina engine telemetry.

## Verification

- Run the concrete acceptance and regression checks below with exact launch fingerprints and evidence outside the repository.

## Results and evidence

Planned. Record measured outcomes and verification evidence here during execution.

## Change history

- 2026-11-27T09:27:25+01:00 — Plan created before implementation.

## Baseline and hypothesis

The TUI already polls slots and server metrics and derives some live decode
speed from logs. The daily driver uses an external reference executable,
while VITRIOL also supports its in-tree server. Capability differences must be
represented explicitly. Historical request totals and live gauges must retain
their existing meanings.

Hypothesis: attributable request measurements reveal whether a slow turn spent
time in prefill, cache restore, transfers, drafting or generation, allowing the
other plans to choose experiments from observed bottlenecks.

## Research and source comparison

Use HiCache's exact-prefix hit and recovery distinctions to define cache
metrics. Use SpecStream's transfer/compute separation for diagnostic timings.
Read the existing server APIs and client timing path before defining fields.

- [HiCache](https://www.sglang.io/blog/hicache)
- [SpecStream](https://arxiv.org/abs/2609.33184)

## Implementation stages

1. Define a small versioned metric contract. Include request/slot/generation
   identity, filled prompt, cached/new tokens, first-token latency, restore
   duration, decode time, proposed/accepted drafts and KV transfer time/bytes.
2. Inventory which values each backend actually supplies. Use explicit absent
   values for unsupported metrics. Add engine instrumentation only for missing
   decision-relevant values and measure its overhead.
3. Extend the existing poller and shared telemetry representation. Reset
   deltas at engine restart and preserve request identity through routing.
4. Add a compact dashboard view and per-slot detail. Show cache and prefill
   status during waiting; put detailed transfer statistics in expanded detail.
5. Validate dark themes, narrow terminals, stale data and active tool turns.

## Invariants

1. Existing dashboard/service actions behave identically without new fields.
   Pin with legacy API fixtures and existing TUI checks.
2. Unsupported, stale and zero values remain distinguishable. Pin with
   missing-field, restart and counter-reset fixtures.
3. Request, slot and physical model identity match the actual served lane.
   Pin with local/cloud routing and overlapping request fixtures.
4. Timing definitions use monotonic intervals and avoid summing overlapping
   GPU work as wall time. Pin with controlled delayed-response fixtures.
5. Telemetry polling cannot mutate slot state. Pin with endpoint allowlists
   and isolated runtime observations.

## Acceptance and falsification

Reconcile displayed counters to API replies and logs from the established
200K Rust-session workload. Verify cache hits across restart, cancellation,
model switching and two slots. Require under 1% measured throughput overhead,
correct units and no invented values on either engine. Use release terminal
captures in Vitriolum and Aether. If an expensive metric changes scheduling,
retain it as opt-in profiling instead of continuous polling.

## Dependencies and rollout

Implement this first. The other four plans consume its metric definitions.
Run Rust formatting and appropriate existing tests, build and install the
operator's TUI binary, and confirm the display in the installed executable.

## Evidence and delivery

Status is planned. Implementation and performance are unverified. Record per-stage commits, full argv, model/runtime identity and actual filled token counts. Use the repository verification gates, retain existing diagnostics and document unavailable checks. Save results before the final verification freeze. Public research code requires license review and attribution before incorporation. Live maintenance requires an idle or operator-authorized window.

## Related implementation plans

- [cache-continuity](2026-11-27-092725-cache-continuity.md)
- [streamed-kv-attention](2026-11-27-092725-streamed-kv-attention.md)
- [adaptive-speculation](2026-11-27-092725-adaptive-speculation.md)
- [selective-historical-kv](2026-11-27-092725-selective-historical-kv.md)

- 2026-11-27T10:15:12+01:00 — Begin implementation with existing read-only slots and request timing logs. Reference slots expose processed/cache counters but reset them when idle. Display these as active-request values only, with explicit unknowns for unsupported fields. Keep completed request timing separate from live throughput. Measure instrumentation scope before adding backend-dependent transfer or first-token fields.

- 2026-11-27T10:18:33+01:00 — Stage 1 implemented active per-slot prompt counters with explicit unavailable values for legacy and idle responses. Corrected last-decode parsing for timestamped reference slot logs. All 62 Rust tests passed. The rebuilt installed release displayed exact new/cache counts in Vitriolum at 125 columns and Aether at 100 columns using an isolated HTTP fixture. Praetor reported no blocking findings. Remaining telemetry stages include request-completion timing identity, restore and unsupported-backend status, first-token and KV-transfer instrumentation, plus overhead measurement. Research-dependent engine stages remain planned.
