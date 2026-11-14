---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T21:11:22+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "2c2fe002f4790452ffc1a6ee0956e77228a4884b"
supersedes: []
superseded_by: []
related: [".opencode/plans/officina-multisession-hardening-2026-11-11.md", ".opencode/plans/slot-scaling-2026-09-28.md"]
---

# lapis-peer-presence

## Purpose

Correct Lapis single-slot arbitration and retain active peer visibility through long prefill; evaluate second-slot feasibility without changing live configuration.

## Approach and alternatives

Separate immutable session start from freshness timestamps, refresh active registry entries through existing engine polling, retain sticky continuations and privacy/cloud-admission gates, and stamp route ledger records with session and process identities.

- Increasing slots can mask arbitration defects and incurs sequence/MTP memory overhead. Expanding TTL alone leaves dead peers stale and does not fix the all-sessions-senior ranking bug. Avoid new model calls or independent pollers.

## Scope and documentation

Registry identity and heartbeat, both router integrations, test-store isolation, route identity metadata, regression tests and operator guide. Preserve concurrent Rust TUI/core work.

## Verification

- Two publishers with refreshed timestamps elect one senior; long active run remains live past TTL while settled/shutdown does not; legacy entries round-trip; actual router heartbeat integration; isolated test registries; full extension tests/typecheck/Praetor and source-bound review.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T21:11:22+01:00 — Plan created before implementation.

## Invariants

1. I1: With no peers, policy decisions retain existing behavior; the existing golden overflow and router suites pin parity.
2. I2: Two active local publishers with distinct immutable start times elect the same single senior regardless of heartbeat order; newer session yields under contention when a permitted cloud lane fits. Pinned by asymmetric heartbeat and swapped-view tests.
3. I3: Active runs remain visible beyond the 60-second TTL via the existing telemetry heartbeat; settled sessions expire normally, and shutdown removes the shard. Pinned by lifecycle tests with an isolated real registry.
4. I4: Legacy registry entries without sessionStartedAt remain readable and use their existing ts ranking fallback; new optional metadata round-trips without prompts/credentials. Pinned by serialization tests.
5. I5: No inference requests, extra polling loops, privacy relaxation, forced continuation migration or live engine configuration changes are introduced. Pinned by router tests and lifecycle review.
6. I6: Router tests never write the operator registry. Per-test temporary directories and production-store hash checks pin isolation.

## Observed baseline / limits

At 21:07 local the last real local registry entry was already 122 seconds old (TTL 60s) while the engine processed task 110683 through a ~130k prompt. Two synthetic cloud registry entries originated from router tests (including second-model). Self ranking uses process start, but publishers refresh peer ts on each route, so both sessions can independently rank themselves senior. Continuation routes intentionally bypass tier changes; the fix preserves that boundary. Latest inspected session metadata: Projects uses Caduceus dahl/MiniMax-M2.7; briev-lang last local response is an error. Active engine task ownership cannot be proved because its logs contain no session ID and direct localhost/GPU queries are denied/unavailable in this sandbox.

## Second-slot feasibility

Live config: context 262144, parallel 1, unified KV, tensor split 27,21, MTP draft_n_max 2, ubatch 128. Source confirms unified KV does not divide the per-sequence cap by slot count; both slots would draw from the shared pool. Fixed ~131k caps require an explicit per-slot setting. Additional sequence/recurrent state and speculative contexts still require memory; exact current headroom is unavailable. Historical 2026-09-28 measurements show two-slot aggregate ~1.0–1.17x and improved decode responsiveness beside prefill, with older configuration. No two-slot load certification or live config write is authorized by a feasibility question. The hypothesis is falsified by memory exhaustion at load/depth or worse task latency under an equivalent concurrent workload.

## Implementation notes — 2026-11-14T21:22:08+01:00 (Europe/Amsterdam)

Added optional sessionStartedAt to registry entries and use it for peer ordering/ranking, keeping ts only for TTL freshness. Both virtual Lapis and direct adaptive routers publish the immutable identity. Active lifecycle state drives a 10-second refresh through existing engine callbacks; settled runs stop renewing, shutdown removes the shard and detaches the listener, and a new session reattaches without inheriting the old decision. No independent timer, inference or service/config write is added. Legacy entries remain readable.

New route records carry session and process IDs. /route-why uses this session's in-flight route or its own completed ledger rows; other sessions' latest completions cannot override it. Sticky cloud continuations refresh their physical-backend badge. Continuations stay on their existing physical model; this fix does not interrupt or migrate requests already running/queued. Both active TUIs must reload extensions to adopt the changed publisher/reader schema.

Router fixtures now isolate all registry shards and route ledgers under per-test temporary directories; synthetic test-model/second-model records no longer enter the production registry. No runtime registry cleanup was performed beyond normal reader TTL GC.

Development checks: 137 related tests passed before the final listener-cleanup amendment. Typecheck passed. Full suite reproduced two untouched subprocess failures; source-scoped Praetor baseline has nine existing findings and the candidate added none. Final checks rerun after the listener-cleanup change. Evidence/logs: /tmp/officina-peer-evidence/ and /tmp/officina-peer-*. Live GPU headroom and /slots queries remain unavailable in the sandbox; two-slot feasibility is an inference from unified-KV code and historical experiments, not a fresh load certification.

Final gates: focused routing/registry/ledger/loader tests, full npm test, npm run typecheck, git diff --check, source-scoped Praetor baseline comparison, existing commit hooks, verified-development run/status. Local commit is authorized by the standing repository workflow and the user's instruction to commit this work; unrelated edits must remain excluded. Final validation will bind to the resulting commit using its parent as the evidence base.
