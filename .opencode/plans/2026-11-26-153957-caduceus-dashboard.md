---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T15:39:57+01:00"
timezone: "Europe/Amsterdam"
status: "active"
baseline_commit: "82c8c66cd36b66c21b5ac4207f5505c385be4d58"
supersedes: []
superseded_by: []
related: []
---

# caduceus-dashboard

## Purpose

Expose Caduceus route health and controls in the VITRIOL dashboard, with concise routing failures in Officina.

## Approach and alternatives

Reuse the loopback router health and targeted DELETE /park endpoints and the existing router event log. Add a dashboard row and detail pane rather than a permanent tab. Preserve the existing Officina model pickers and surface router policy failures inline.

- Extend the router health response with the last requested model and park reasons. The dashboard labels routes as eligible under router policy, since health is not an upstream availability probe. Older routers still show their available fallback and parked data.

## Scope and documentation

Stages: router telemetry, VITRIOL dashboard and scoped router logs, then Officina inline messages. Document the dashboard flow in the repository guide.

## Invariants

1. Health queries do not affect routing or send traffic to providers. Router fixtures verify request order, fallback and unpark behavior.
2. Only the explicitly selected live parked ID can be unparked. The action targets the loopback router and is asynchronous. Input and HTTP fixture tests pin the target.
3. Unreachable routers are shown separately from engine state. A ready route means eligible under the reported router policy, so no upstream capacity guarantee is inferred. Fixtures cover legacy and current responses.
4. Dashboard and Logs keep their existing tab order and engine log scopes. Caduceus has a dashboard pane and an additional log source. Render and input fixtures verify access.
5. Officina distinguishes all routes cooling down from exhausted attempts and performs no engine recovery for either. Classifier regression tests pin both forms.

## Verification

- Router fixture E2E, both Rust cargo test and fmt gates, release builds, TUI render/input regression tests and a terminal smoke against a loopback fixture. Store full logs and source-bound evidence under /tmp/vitriol-caduceus-dashboard/

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T15:39:57+01:00 — Plan created before implementation.

- 2026-11-26T15:44:43+01:00 — Router stage 6eb47c9 passed 44 fixture tests. VITRIOL dashboard stage passed 56 Rust tests, cargo fmt and its release build. A terminal smoke against a loopback stub verified Aether rendering, a targeted unpark with a reserved query character, unchanged secondary cooldown, and route-filtered logs.

- 2026-11-26T15:47:23+01:00 — Officina summarizes cooldown, exhausted lanes and exact-route policy failures inline without selecting a different model or invoking engine recovery. All 406 native tests passed after documenting the existing server.launcher config key and ratcheting the shorter dispatcher. Praetor baseline comparison prompted a final dashboard rendering review.
