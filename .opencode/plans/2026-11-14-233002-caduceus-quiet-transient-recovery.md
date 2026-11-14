---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:30:02+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "a63b6e5a870cf6c9a2f7437e68606451b0b01916"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-230715-caduceus-shared-model-capacity.md"]
---

# caduceus-quiet-transient-recovery

## Purpose

Recover temporary gateway HTTP failures inside Caduceus before they become session errors

## Approach and alternatives

Extend the existing bounded abortable admission recovery to transient HTTP status responses; honor explicit no-retry and Retry-After without retrying unknown quota, authentication or malformed requests

- Swallowing every failure can silently lose answers or repeat partially delivered tool calls. A second model-routing implementation inside the provider duplicates existing admission/privacy logic.

## Scope and documentation

Caduceus transport response classification, SDK regression tests and guide

## Verification

- Real SDK transient-to-success and exhausted recovery tests; quota/auth/no-retry/cancel negative cases; typecheck, source-scoped Praetor and full repository evidence gates

## Invariants

1. Non-transient status/body pairs retain ordinary SDK handling, including generic/quota 429, authentication and invalid requests (negative matrix).
2. Explicit gateway capacity 429 and selected transient HTTP responses share the same two-extra-attempt/120-second admission budget; accepted streams are never replayed (budget and SDK stream tests).
3. Original model, body, payload hooks and transport options survive; Retry-After is respected and waiting is abortable (request identity, hook, cancellation and timing tests).
4. Recovered attempts emit no assistant error or intermediate response callback; exhausted recovery returns one final SDK error (real SDK events tests).
5. `x-should-retry: false` and the existing kill switch disable the corresponding recovery. With recovery disabled, delegation is unchanged (negative tests).

## Baseline, limits and falsification

Baseline a63b6e5 has explicit semaphore-only recovery. Pinned SDK's provider request helper defaults maxRetries to zero when provider-specific settings are absent; it does have its own retry support when configured. We reuse the existing provider fetch recovery budget rather than add another routing policy. Only pre-stream HTTP rejections are handled here: silent replays after partial text/tool delivery can duplicate visible output or tool calls and are excluded. Network failures without a response are ambiguous (upstream may still be generating), so retain current outer handling. No promise to hide permanent errors or wait forever. This approach is rejected if an intermediate error escapes after eventual success, if quota/auth errors get retried here, or if ordinary accepted streaming is buffered/replayed.

Affected contract: Caduceus provider transport recovery and operator guide; no gateway vendor/config/TUI changes. Final evidence: `/tmp/officina-quiet-recovery-evidence`. Live acceptance remains unavailable because the sandbox denies localhost sockets. Broad-suite baseline has two unrelated subprocess sandbox failures.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T23:30:02+01:00 — Plan created before implementation.

- 2026-11-14T23:34:25+01:00 — Implemented shared internal recovery for explicit capacity codes and HTTP 408/500/502/503/504/520/524/529, bounded by two extra attempts and the existing 120-second admission window. Added one/two-second backoff, explicit no-retry handling, and status/count/delay-only harness events. SDK tests prove eventual success emits no error, payload hook runs once, quota/auth fail without transport retries, exhausted recovery emits one final SDK error, kill switch works, and accepted partially delivered streams are not replayed. Focused Caduceus/routing/health suite: 112 passed before final log/privacy assertions; final recovery suite: 42 passed, followed by strengthened real partial-stream test passing separately. Typecheck passed. Source-scoped Praetor including hidden extension code and tests: zero failures. One fake-clock mixed-response test was corrected to use real bounded delays because native Response.clone stream scheduling does not follow fake timer boundaries. Final frozen repository gates and source-bound review will be saved at /tmp/officina-quiet-recovery-evidence. No gateway/vendor/config changes, deployment or live requests; reload running extensions to activate.
