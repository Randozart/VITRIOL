---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T22:40:20+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "95bc7b383143fd3bf0a48e8ec1a01b7985ee37f7"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-223206-local-prefill-retry-diagnosis.md"]
---

# local-prefill-progress

## Purpose

Reproduce healthy local prefill timing out as terminated and prevent unnecessary retry by streaming request-specific progress.

## Approach and alternatives

Exercise real Undici HTTP parsing against an in-memory duplex connector with shortened inactivity deadlines; compare silent SSE with request-specific progress. Delegate the local provider to the pinned built-in API with return_progress enabled before payload hooks, preserving hook replacement and abort semantics.

- Globally increasing timeouts would mask stalls and affect cloud requests; suppressing retry after a dead connection cannot recover it. Global engine activity cannot establish progress of this request.

## Scope and documentation

Local llama.cpp provider streaming adapter, registration paths, regression tests, dependency declaration if needed, operator guide. No engine restart, slot mutation, global dispatcher or cloud-timeout changes.

## Verification

- Transport negative control reproduces terminated/UND_ERR_BODY_TIMEOUT; progress survives the same deadline with one request; real SDK parses progress without assistant text pollution; abort and genuine failure remain failures; local-only registration survives re-probe; focused/full tests, typecheck, Praetor and normal commit hooks.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T22:40:20+01:00 — Plan created before implementation.

## Invariants

1. I1: Cloud and non-OpenAI local APIs retain their existing request options and behavior; registration/adapter tests pin scope.
2. I2: Local OpenAI-compatible inference requests enable native request-specific return_progress before payload hooks; hook replacements are honored, and final assistant content contains no progress text. Pinned by real built-in API/parser tests.
3. I3: User abort, real stream termination, HTTP errors and existing bounded timeout policies remain effective. No progress from another slot/provider can extend this request. Pinned by negative transport and cancellation tests.
4. I4: Initial registration and every local alias/context re-registration retain the adapter; provider identities, model catalogs, auth and model selection remain unchanged. Pinned by registration/re-probe tests.
5. I5: No engine restart, active-slot mutation, global dispatcher change, synthetic inference, or new poll loop is introduced. A kill switch OFFICINA_NO_PREFILL_PROGRESS=1 restores the prior local transport path.

## Reproduction before implementation

Prototype /tmp/officina-prefill-repro.mjs uses the pinned Undici client and HTTP parser over an in-memory Duplex connector (no sockets, server or network). With headers/body inactivity set to 100 ms and answer delayed 1600 ms, silent response failed at 1065 ms with message terminated and cause UND_ERR_BODY_TIMEOUT. Identical response with 30 ms progress SSE succeeded at 1605 ms. Timer granularity explains the scaled timeout delay. This proves the failure mechanism, not the original live exception cause, which was not retained.

Native engine source already emits per-request OpenAI-compatible progress chunks when return_progress is enabled: initial update at slot start and updates after prompt-processing batches. The built-in pi OpenAI parser ignores empty assistant deltas and surfaces raw events through its existing callback. Enabling that protocol is preferable to suppressing retries after a broken connection or using global engine activity as a proxy for this request.

Evidence and full logs: /tmp/officina-prefill-evidence/. Exact direct dependency on the already pinned pi-ai 1.5.0 may be declared for the adapter; no SDK version changes are intended. Full verification retains the two known unrelated subprocess-test failures if reproduced.

## Counterevidence / design amendment — 2026-11-14T22:59:43+01:00

Server source AND the configured implementation library already contain 30-second SSE comment keepalives. Narrow the initial hypothesis: failure to request native progress is not sufficient proof that the live stream was silent. Added a positive comment-keepalive control and a local, completion-path-only Undici diagnostics observer to retain raw codes such as UND_ERR_BODY_TIMEOUT, without logging headers, query parameters, URL paths or prompts. The observer is globalThis-backed and subscribes once across jiti instances/reloads; probes/cloud endpoints are excluded. Original live root cause remains unconfirmed until error cause and wire delivery can be observed on the active process.

## Implementation notes / freeze

The llama.cpp provider delegates to the pinned built-in API, adding return_progress before existing payload hooks and honoring their replacement. Other APIs/providers and all timeout/retry/cancel limits retain their old behavior. All four initial/alias/context registration sites carry the adapter. The kill switch disables automatic progress injection. Raw transport-code diagnostics remain available for troubleshooting. The direct pi-ai dependency pins the already installed 1.5.0; offline lock update changes only that root dependency and synchronizes stale lock root version metadata with the existing package version (no SDK upgrade).

Tests exercise real SDK/SSE parsing and real Undici HTTP/1 parsing/inactivity timers over an in-memory Duplex connector: quiet negative control; progress survival with one request and no generated progress text; working comment keepalive countercontrol; cancellation; HTTP error and broken stream; cloud/non-OpenAI parity; hook replacement; provider re-registration; isolated-jiti observer and sanitized metadata. The original diagnostic report has a dated correction preserving its earlier evidence.

Final checks: focused local-provider/events tests; full npm test; npm run typecheck; source-scoped Praetor on modified/new TS including transport fixture; git diff --check; normal commit hook; verified-development run/status bound to the commit. Existing two subprocess-test failures remain explicit. No live inference/slot mutation, engine restart, global dispatcher edit, increased timeout or live-network reproduction was performed. Evidence: /tmp/officina-prefill-evidence/.
