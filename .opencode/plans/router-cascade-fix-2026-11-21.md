# Router cascade fixes — plan (2026-11-21)

Fixes the recurring "all 2 candidate(s) unavailable → retry storm → Agent is
already processing" cascade. Research:
`router-cascade-research-2026-11-21.md`. Confirmed by owner (PIPELINE
confirm gate, "Proceed").

## Changes

- **R1** router: no-model failures never enter the park table (both park
  sites in `caduceus-router.mjs`). A `?` key still may exist from OLD state
  files — `pruneParked` also drops it; `giveUpMessage` counts only real
  lanes.
- **R2** router: hint parks capped at `min(hint, CADUCEUS_ROUTER_MAX_PARK_S,
  default 900)`; `DELETE /park?model=X` admin route; park table surfaced in
  `vitriol caduceus doctor` (extensions-side `doctor` reads `/health`).
- **R3** TUI heal: `classify_error` recognises `router_unavailable` /
  `router_exhausted` as a new `Failure::RouterPolicy` BEFORE the 503/502
  match; recovery for RouterPolicy = notice-only (never engine actions).
- **R4** TUI: compaction_end auto-continue guarded by `!state.is_streaming`
  (same guard as queued delivery at mod.rs:775).
- **R5** ops: enable `vitriol-caduceus-router.service` (Restart=always).
  Sudo-gated — if not available, record BLOCKED-optional and leave the bare
  process running (document in the plan close-out).

## Invariants

- **I1 — walk parity**: a model-tagged request walks candidates identically
  to today when no `?` entry exists. Pinned by: `caduceus-router-lib`
  unit test `candidateOrder` golden matrix (unchanged) + existing e2e.
- **I2 — park cap**: no park entry exceeds max(hint-cap, default 900s).
  Pinned by: unit test `hint parks are capped` (lib).
- **I3 — phantom lane**: a no-model request failure leaves real lanes
  servable and absent from the park table. Pinned by: e2e
  `no-model 401 storm leaves lanes servable`; unit `no-model failures do
  not park`.
- **I4 — heal attribution**: `router_unavailable` never classifies as
  EngineDown. Pinned by: `heal.rs` unit test `router_policy_is_not_engine_down`.
- **I5 — auto-continue safety**: the compaction continue prompt is never
  sent while `state.is_streaming`. Pinned by: TUI test asserting the guard
  on the compaction_end handler path.
- **I6 — unpark is explicit and safe**: DELETE /park removes exactly the
  named entry; unknown model → 404. Pinned by: e2e `unpark endpoint`.

## Named tests

1. `caduceus-router-lib.mjs` unit: `hint parks are capped`,
   `no-model failures do not park`, `giveUp counts real lanes only`.
2. `caduceus-router.e2e.test.mjs`: `no-model 401 storm leaves lanes
   servable`, `unpark endpoint`.
3. `heal.rs` `#[test] router_policy_is_not_engine_down` (+#74 parity).
4. TUI: compaction auto-continue guard test (mod.rs test module).

## Stages

S1 router lib + unit tests (R1/R2 core) → S2 router endpoints + e2e (R2)
→ S3 heal classification (R3) → S4 TUI guard (R4) + rebuild/install →
S5 doctor surfacing + systemd attempt (R5, sudo-gated) → gates, commits
per stage, push, CI.
