---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T13:27:20+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "6a3203df61bc7149e47a0fed30296b3ac50205e9"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-15-125010-officina-input-settings-chat.md"]
---

# officina-checker-warning-cleanup

## Purpose

Separate checker execution failures from invalid code, make child termination tests diagnostic and bounded, and eliminate Officina Rust warnings where behavior can be preserved

## Approach and alternatives

Stage checker outcome contracts and subprocess-test fixes first; remove dead code and apply reviewed compiler/Clippy repairs in coherent commits; retain all diagnostic checks and honest unavailable outcomes

- Suppressing lints or marking denied execution successful would hide real failures. A baseline records unresolved findings but does not eliminate them.

## Scope and documentation

Shared Officina diagnostics/subagent tests and canonical Rust TUI; no engine/model changes or live-session restart

## Verification

- Focused checker and subprocess regressions; full TypeScript and Rust suites; clippy all-targets with warnings denied; format, Praetor baseline/delta, release build/install and isolated terminal check; final frozen evidence

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T13:27:20+01:00 — Plan created before implementation.

## Invariants

1. I1: A checker reports passed only after successful execution; a nonzero syntax-check exit remains failed, while launch/permission/timeout failures are unavailable, never repair instructions. Pin with error classification and diagnostic rendering matrix.
2. I2: Successful checks remain silent; execution unavailability is reported to the operator without repeated model injections. Pin with extension event tests and existing token-budget tests. Existing explicit syntax failures still reach the model within budget.
3. I3: Force-kill escalation still uses actual exit state, never signal dispatch state. Tests bound startup/exit, retain real SIGKILL assertions, and clean up children. Denied capabilities must stay explicitly unavailable.
4. I4: With no new setting/input, Rust rendering, RPC payloads, session locking, Unicode input, compaction continuation and the new popup contracts remain unchanged. Full 250-test baseline plus relevant rendering/serialization tests pin parity.
5. I5: No new lint allowances, warning suppression, Praetor configuration changes, or test weakening. Warnings are removed by behavior-preserving cleanup or reported as unresolved with evidence. Kani/protocol declarations remain meaningful.

Baseline HEAD: 6a3203d. Prior full sweep: 250 Rust tests pass; 1220 extension tests pass, two subprocess tests fail under this execution environment. Exact original/final Clippy comparison at /tmp/officina-input-settings-evidence/clippy-baseline-delta.json shows 246 unchanged warnings. A valid Python child prints its result but Node execFileSync throws EPERM with status 0; this must not be misreported as a syntax failure. Preserve this counterexample. Stage order: checker/subprocess contracts, warning cleanup, then final frozen validation/install. The cleanup hypothesis is falsified by altered serialized commands, UI regression, lifecycle regression, added suppressions, or unavailable checks claimed clean.

### Checker stage — 2026-11-15 Europe/Amsterdam

Added explicit passed/failed/unavailable checker outcomes, preserving real nonzero syntax failures. Unavailability is recorded additively in per-tool facts, shown neutrally in the Rust TUI, notified once per checker outage and excluded from model repair injections. Pure classification, event/facts/notification and TUI regression tests cover these distinctions. Subagent signal verification now uses a bounded file readiness handshake rather than stdout delivery, preserves real SIGKILL assertions and cleans up with a child self-expiry fallback. Focused TS result: 34 passed, one real Python-through-Node check explicitly unavailable (EPERM); the real signal test passes. No capability check is treated as clean merely because execution was denied. Baseline source/Praetor and all evidence: /tmp/officina-checker-warning-evidence.
