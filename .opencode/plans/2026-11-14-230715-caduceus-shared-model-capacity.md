---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:07:15+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "b5bf29b1433cd0de7604369fc2fec77d82a008f2"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-224020-local-prefill-progress.md"]
---

# caduceus-shared-model-capacity

## Purpose

Keep sessions on the same cloud model through temporary gateway queue saturation

## Approach and alternatives

Reuse the gateway FIFO queue; retry only explicit semaphore-capacity HTTP 429 responses before they reach SDK quota handling, with a bounded count and abortable delay

- A separate cross-session scheduler duplicates the gateway queue and adds fairness and crash-recovery state. Treating all 429 as capacity would violate genuine quotas.

## Scope and documentation

Caduceus provider transport, regression tests and operator guide

## Verification

- Focused Caduceus and routing tests, typecheck, source-scoped Praetor, repository evidence gates including baseline environment failures

## Invariants

1. With no explicit gateway capacity code, the original response and all request options are unchanged (status/body matrix and SDK callback tests).
2. Only HTTP 429 with a structured `SEMAPHORE_TIMEOUT` or `SEMAPHORE_QUEUE_FULL` error can be retried here; genuine quota/auth/server errors preserve ordinary handling (negative matrix).
3. Retries preserve the exact request body/model, have a two-retry and 120-second admission budget, respect Retry-After, and stop on cancellation (request identity, budget and abort tests).
4. The gateway owns concurrency and FIFO ordering. Officina never interrupts an accepted stream or creates cross-session locks (two-session gateway simulation and streaming callback tests).
5. Capacity responses consumed internally never reach the quota-health response callback; final errors do (real SDK test).

## Baseline and falsification

Baseline b5bf29b: cloud lane spreading already reuses a model when all admitted lanes are occupied. Gateway source in installed OmniRoute provides account/model gates and releases streaming slots on drain/cancel. `streamingResponse.ts` emits structured semaphore capacity HTTP 429; both routing extensions currently apply generic 429 cooldowns before seeing the body. This falsifies a need for another scheduler, but demonstrates a capacity/quota classification gap. No live-wire confirmation is available in this sandbox. If SDK callbacks observe intermediate retries, if genuine quota errors retry, or if two sessions cannot progress through the simulated one-slot gateway, this implementation is rejected. No vendor/config/engine changes or live requests.

Final evidence will be stored outside the repository at `/tmp/officina-cloud-capacity-evidence`; broad-suite baseline includes the two previously identified sandbox subprocess failures.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T23:07:15+01:00 — Plan created before implementation.

- 2026-11-14T23:14:05+01:00 — Implemented provider-scoped capacity retries through the builtin SDK with original payload/cancellation/hooks preserved. Focused Caduceus/routing/provider-health suite: 93 passed before logical wait-phase extraction; final capacity suite: 23 passed. Typecheck passed; explicit source-scoped Praetor including tests: zero failures, 29 diagnostics (advisory). Existing model reuse and gateway FIFO remain responsible for scheduling. Clarification: installed SDK calls onResponse after successful stream admission, so ordinary HTTP errors may bypass that callback; the virtual-router error path still imposes a generic model cooldown. A 30-minute penalty is conditional on a response event reaching the generic 429 handler, not established as the live failure. This patch prevents recognized admission refusals from reaching either outer failure path until its budget is spent. Final combined evidence with the pending single-slot background fix will use /tmp/officina-cloud-capacity-evidence. No deployment, inference, vendor edits or config changes.
