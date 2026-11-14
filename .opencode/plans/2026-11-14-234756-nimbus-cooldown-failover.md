---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:47:56+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "a31249722af46d3b66a4a4b5f48e51b9afef5317"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-233002-caduceus-quiet-transient-recovery.md"]
---

# nimbus-cooldown-failover

## Purpose

Make the observed bare all-credentials-cooling error trigger bounded cloud failover instead of stopping or reselecting the failed lane

## Approach and alternatives

Preserve an explicit 429 cooldown signal through pinned SDK error normalization; mark failed cloud lanes before selection, use the refreshed candidate, and remove Nimbus health-bypassing fallback

- Retrying the same cooling lane wastes requests; swallowing the error without another admitted target strands the session. Bypassing context/privacy/budget gates is not acceptable failover.

## Scope and documentation

Caduceus cooldown response normalization and Lapis/Nimbus retry selection; regression tests and guide

## Verification

- Exact user-error SDK retry classification; Nimbus retry without response callback; all-cooling refusal; Lapis fresh replacement; typecheck, focused tests, Praetor, final repository gates

## Invariants

1. Gateway model-cooldown HTTP 429 remains recognizable after the SDK reduces it to error.message (real SDK test with the exact production envelope and bare message).
2. Nimbus retries mark the failed lane and choose another admitted cloud lane without requiring an after_provider_response event; all-cooling and exhausted retries terminate honestly (routing regressions).
3. Lapis returns the newly selected candidate, not the pre-failure candidate; retry admission uses fresh health facts (regression with two lanes).
4. Continuations stay on the previously successful model; no healthy-path changes, context truncation, quota bypass or local fallback for Nimbus (existing suite and oversized-context negative).
5. Unknown/auth/quota errors and original payloads retain existing behavior. Normalization is restricted to the gateway's explicit local-model-cooldown header plus structured model_cooldown code (negative envelope tests).

## Production evidence and boundary

Session selection is officina/nimbus. Last saved errors for aion/aion-labs/aion-2.0 and aion/aion-labs/aion-3.0 contain only the bare all-credentials-cooling message; there is no 429/rate-limit marker for Pi's retry classifier. Installed gateway buildModelCooldownBody/modelCooldownResponse emits structured code=model_cooldown, reset_seconds, HTTP429 and X-OmniRoute-Local-Cooldown:model. Nimbus currently ignores failed requests and falls back to a fit-only candidate when all healthy candidates are excluded. Lapis separately calculates a next candidate after a failure but returns its earlier candidate variable. The pinned SDK only emits the normal onResponse callback after successful admission, so tests must not manually deliver that callback to simulate these HTTP errors.

This corrects the escape into outer retry/failover. It does not promise that every intermediate error is absent from the current TUI; errors are rendered at message_end before the backend makes its retry decision. UI presentation is a separate contract. Final evidence: /tmp/officina-nimbus-cooldown-evidence. Rust cache-lifetime work remains in its separate plan and resumes after this fix.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T23:47:56+01:00 — Plan created before implementation.

- 2026-11-14T23:51:15+01:00 — Implemented restricted gateway model-cooldown response normalization preserving retryable429 and reset hints through the pinned SDK. Fixed Nimbus retry failure recording, removed fit-only resurrection of cooling lanes, added retry budget and prompt-size estimate, and corrected Lapis stale-candidate return. Actual saved session was Nimbus and contained the bare production error. Focused provider/routing suite:77 passed; typecheck passed. Explicit Praetor comparison over hidden production/test source:4 existing baseline warnings and0 new warnings. Existing TUI still may render an intermediate error before auto_retry_start; backend now recognizes and handles it. No gateway/engine restart or live requests. Final combined evidence will be collected with the Rust lifetime stage; private evidence /tmp/officina-nimbus-cooldown-evidence.
