---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T19:15:39+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "dab221463ba3806c97d9ec9467b532722d7ef79b"
supersedes: []
superseded_by: []
related: []
---

# lapis-local-first

## Purpose

Make Lapis try an available, fitting Kintsugi slot first on new turns and distinguish capacity, contention and outage in route reporting.

## Approach and alternatives

Use the existing fresh slot response, including its context size. Permit the local fast path before cloud discovery or warm-cloud affinity. Preserve bounded retries and ongoing physical tool-loop continuity. Record the policy reason in the route ledger and /route-why.

- Disabling cloud globally would remove useful failover and would still leave oversized sessions unable to fit Kintsugi. Forcing an oversized prompt to the engine would be a predictable failure.

## Scope and documentation

Officina's Pi extensions and guide. Keep the engine, slot window and caller's active session configuration unchanged.

## Invariants

1. A new fitting Lapis request with a freshly confirmed idle local slot uses local before cloud routing policy, even after a cloud turn or with complexity promotion configured. No cloud discovery is needed on that path.
2. If the fresh probe fails, retain snapshot-based contention and outage fallback. A busy slot or an already-running tool loop retains its established handling.
3. A request estimated to exceed the reported local slot window never blind-retries on that slot. Protected requests stay local-only and surface whether compaction is needed.
4. The ledger keeps the trigger reason and separately reports the selection reason. Nimbus and adaptive-router policy remain unchanged.

## Verification

- Exercise fresh idle/busy/down, stale telemetry, window mismatch, cloud-only Nimbus parity and /route-why fixtures. Smoke the release path using a short synthetic request sent only to local Kintsugi.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T19:15:39+01:00 — Plan created before implementation.

- 2026-11-26T19:20:57+01:00 — The incident ledger recorded 92233 cloud input tokens, localState=up, and no exact selection reason. The live idle slot advertised 65536 tokens and local complexity promotion was configured at 0.65. A synthetic short prompt through real Pi RPC used vitriol/Lapis Occultus and completed locally with the fresh-slot selection reason. 122 focused tests and typecheck passed, with zero new Praetor signatures. Oversized fixtures report the live slot window and reject protected overflow without cloud. The existing Officina Pi child must be reopened to load the new extension source. Evidence under /tmp/vitriol-lapis-first/.
