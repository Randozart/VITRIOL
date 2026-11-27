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

- 2026-11-27T10:35:33+01:00 — Stage 2 begins with bounded incremental engine-log collection. Publish completed prompt/decode timing only on a matching slot/task release. Associate cached-token counts only when observed on that same active slot/task. Reset records at engine PID changes and log rotation/start markers. Unavailable first-token and KV-transfer timings remain explicitly unsupported rather than inferred from poll intervals.

- 2026-11-27T10:45:00+01:00 — Stage 2 implementation results. The dashboard retains the most recently completed observed task with slot/task ID, new prompt tokens, observed exact cached tokens, prefill/decode durations and optional draft counts. The collector retains one completed group per slot, newest first, and publishes only after a matching release with both timings. A short request never observed while active has unknown cached tokens. Measurements begin after the dashboard establishes engine identity. Log history remains available independently. Engine PID/port changes, file rotation/truncation, startup markers and skipped log intervals clear request measurements. Publication requires the selected engine's stdout to resolve to the configured log file.

### Stage 2 backend capabilities and limitations

The reference source at `~/kintsugi/engine-src/kintsugi-engine-8f12baacf/tools/server/server-context.cpp` emits slot/task timing groups and draft acceptance before release. Its active slots expose cached-token counts, which reset at idle. Timing is the engine's reported phase duration and is not client wall latency. Sidecar `restored slotN.bin` events are shown only when the following baseline PID matches the served engine. Generation-change restores without a subsequent baseline record remain unavailable. First-token and KV-transfer timings are unavailable and are not inferred from prefill duration. The dashboard displays the newest completed task, while retaining the last group for each slot internally.

### Stage 2 verification and remaining work

Focused verification passed 69 Rust tests, including interleaved tasks, unknown versus zero cache, incomplete/malformed timing groups, wrong-task cache observations, rotation and unread log gaps. `cargo fmt --check`, `cargo test --locked` and `cargo build --release --locked` run from `vitriol-tui/`. The installed executable resolves to that release output. Terminal fixtures exercise the real poller against isolated loopback engine processes in Vitriolum at 125 columns and Aether at 100 columns, including request completion and active sampled cache retention. Captures are differential terminal streams and the fixture forces a resize to check a complete redraw. Fixture values are synthetic and make no production depth/performance claim. Scoped Praetor has no blocking findings. Full logs, fixture scripts/captures and final source-bound gates are under `/tmp/vitriol-completed-telemetry/` and its `verification/` directory.

The production service remained active with PID 816552. No generation request, slot mutation, profile change or restart was made against it in this stage. The existing 200K coding-session evidence is historical. New 200K reconciliation, throughput overhead below 1%, first-token/transfer instrumentation, richer per-slot detail and the other research plans remain pending. This stage adds observation only and claims no inference speedup. Final verification runs after the stage commit against base `feb108f` with source-bound review, secrets, documentation links, clippy, dependency check, build and tests. Praetor and installed terminal checks supply additional scoped evidence.

- 2026-11-27T12:12:15+01:00 — Stage 3 begins from d849a92. Read existing density-bench streamed timing implementation and both server state-save paths before editing. Expose native TTFT histogram sum/count as an explicitly engine-wide mean, absent on unsupported reference backend. Retain existing checkpoint API bytes and save/restore operation durations in PID-scoped sidecar telemetry without extra state operations. Add a stdlib streamed benchmark for client-observed first generated content latency, checkpoint API measurements and paired dashboard-off/on overhead against an isolated GPU0 small-model reference server. No production slot writes or global restart. GPU DMA-only timing stays unavailable because API times include serialization and I/O. Invariants I2/I3 require absent/zero and process/port identity separation; regression tests cover malformed metrics, stale identities and stream role/empty chunks. Falsify the below-1-percent overhead hypothesis if paired runtime results regress beyond threshold or noise cannot resolve it. Evidence will live under /tmp/vitriol-telemetry-stage3; run Python/Rust gates, Praetor and installed dark-theme fixtures before committing each completed stage.

### Stage 3 results — 2026-11-27 12:26 +01:00 Europe/Amsterdam

Native TTFT uses the existing in-tree histogram's unlabelled sum/count, shown
as a since-boot engine mean. Missing series, zero count and invalid values
stay unavailable. Reference Kintsugi has no native series. The new streamed
benchmark records client first-nonempty-content latency and final engine timings
from the same request. Empty/role-only events do not count as first content.
This distinction avoids treating prefill duration as client wait.

Autosave emits versioned PID/port-scoped checkpoint byte counts and save/restore
operation durations from existing responses. Pending saves and generation changes
are excluded. A port check fixes the previous first-matching-process lookup when
multiple servers run. The dashboard labels this `KV I/O`; it is the whole checkpoint
operation, not isolated DMA or continuous KV-tier traffic. Target-state bytes can
exclude draft sidecars while the duration can include restoring them. No CUDA
synchronization or per-token instrumentation was added.

The installed release passed 72 Rust tests, seven Python protocol/identity tests,
formatting, and Vitriolum/Aether fixtures. Scoped Praetor on the Rust source and
exact Python source copies has zero blocking findings. The fixture and benchmark
inputs are recorded under `/tmp/vitriol-telemetry-stage3/`, including binary and
benchmark SHA-256 checks. The original script has 20 unchanged broad-exception
lint findings; the previous executable source has 21 including import order.
The new code/tests pass Ruff, with no new findings in the touched autosave script.
The final generic gate will retain its full-file Ruff failure rather than weaken
that check. Source-bound review records and complete logs are in `verification/`.

The isolated reference engine ran Qwen2.5-0.5B Q4_K_M on GPU0, port 18781, context
8192, with all weights resident. Every measured request newly processed 448 prompt
tokens and generated 256 tokens. Full engine argv/environment fingerprint, PID,
benchmark argv, output hashes and timings are in `vitriol_gen.log` and
`overhead.json`. Sixteen off/on pairs alternate order, with equal five-second
settling and an untimed warmup in both arms. Median client first content was
84.91 ms. Paired mean decode-time change was -6.09%, with bootstrap 95% interval
[-13.85%, +0.638%]. The upper bound passes the below-1% criterion for this sample.
The negative mean is not evidence that the dashboard accelerates inference.
Large variation and shared display activity limit this estimate. An initial run
overlapping terminal fixtures and a second with unequal warmup were stopped and
excluded; their artifacts and reasons are retained separately.

The final isolated checkpoint contained 703 tokens / 4,601,052 bytes. Engine save
was 2.964 ms and restore 2.337 ms; HTTP wall durations were 3.720 and 2.970 ms.
These are a small-state observation, not a 200K transfer benchmark. The full
Kintsugi weights are 10,546,687,168 bytes, already beyond the measured free GPU0
budget before scratch; a second matching resident instance could not be tested
safely alongside the live GPU1 engine. No large-model overhead certification is
claimed. GPU-only copy timing, first-token support inside the reference backend,
200K reconciliation and the other research plans remain open.

Autosave alone was reloaded successfully without sudo after confirming its saved
boot/PID identity. It logged `engine unchanged ... skipping restore`; the engine
remained PID 816552. Its first normal save then reported 1,512 tokens /
191,166,028 bytes in 89.512 ms. No production generation request or engine restart
was issued. The isolated test engine was stopped after the benchmark. Operator
semantics and the reproducible command are in [inference telemetry](../../docs/inference-telemetry.md).

- 2026-11-27T12:27:32+01:00 — Live installed check revealed that journalctl --system -u returns systemd lifecycle messages but omits User=randozart sidecar stdout, which journalctl -u includes on this host. Remove the --system journal restriction while keeping explicit unit selection and system-scope service actions. The live record assertion pins this correction. Existing stage-3 benchmark results describe the previous journal selector; repeat the paired benchmark after rebuilding the corrected installed release before claiming its overhead.

### Corrected journal reader and final benchmark — 2026-11-27 12:34 +01:00 Europe/Amsterdam

The installed TUI now reads the selected unit's journal without restricting it
to the system journal file. System-scope service actions remain unchanged. The
real production terminal check matched the autosave record's 191,166,028 bytes /
89.512 ms (`182.3MiB`, `0.09s` on screen). Both theme fixtures passed again.
The first failed live-terminal capture is retained as the observation that led
to this correction, followed by the successful capture.

The final release and benchmark source hashes remained unchanged throughout a
new 16-pair run. This supersedes the previous build's overhead numbers above
for the delivered executable. Mean paired decode-time change was -1.485%, with
95% bootstrap interval [-3.889%, +0.892%]. The upper bound is below 1% for this
isolated 0.5B workload. Median client first content was 83.68 ms. All requests
processed 448 new prompt tokens and generated 256. The final 703-token /
4,601,052-byte checkpoint took 4.775 ms to save and 2.998 ms to restore in the
engine, with HTTP wall times 5.960 and 3.898 ms. These retain the small-model,
shared-display and statistical limitations already stated. The prior build's
records are named `pre-journal-fix-*`; the delivered build's results are
`overhead.json`, `benchmark.log` and `benchmark-inputs.sha256`.

Final source-bound checks use base `d849a92` after the stage commit. Python
protocol tests and Rust formatting run explicitly in addition to the generic
Ruff, Bandit, secrets, links, clippy, dependency, build/test and review gates.
The known 20 Ruff findings remain an explicit pre-existing failure, with no new
findings demonstrated by `ruff-comparison.json`. Scoped Praetor covers the Rust
source and hash-matching Python copies, including a previous-script baseline.
No full-repository or universally correct behavior claim is made. Checkpoints
and evidence stay private under `/tmp/vitriol-telemetry-stage3/` and
`/tmp/vitriol-resumption/`.
