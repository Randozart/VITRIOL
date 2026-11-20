# Lapis slot-contention fix — plan (2026-11-20)

Research: `.opencode/plans/lapis-slot-contention-research-2026-11-20.md`.

## Problem
Lapis routes a turn to the ONE local slot based on a ≤700 ms-stale `/slots`
snapshot and never verifies the slot is actually free; a second session
claiming it produces a rejected turn that the router does not record or
recover from.

## Stages

**S1 — fresh admission probe (A).**
`_shared/engine.ts`: export `probeAdmission(timeoutMs)` — one `GET /slots`,
`parseSlots`, returns `{ok, anyBusy, slotCount}`; never throws (fail-open).
`resilient-default/index.ts` at the `if (local)` dispatch (:905): call it
before returning the local model; if `anyBusy` and a cloud candidate + budget
exist → demote this turn to cloud (reason `local contended — fresh probe`);
else proceed local. Probe timeout ~250 ms, result NOT cached.

**S2 — record local contention (B).**
`_shared/engine.ts` or a small shared module: `markLocalContended(nowMs,
ttlMs=3000)` / `localContended(nowMs)` in a globalThis singleton (jiti rule).
`resilient-default`: `after_provider_response` handles the LOCAL tier too —
when the response was local and status is 400/503-shaped, mark contention and
push a band notice. `decideTier` input gains optional `localContended?: boolean`
that makes `localBusy` true for this turn (parity: absent = unchanged).

**S3 — ledger + notice honesty.**
`localState` gains "contended" when the probe or mark says so; the route
notice names contention (no silent error).

**S4 — tests, gates, commit.**

## Invariants

- **I1 — parity**: with no probe result and no contention mark, decisions are
  byte-identical to today. Pinned by an overflow.test.ts parity matrix over the
  existing input space (absent optional inputs).
- **I2 — fail-open probe**: a probe timeout/error never blocks or delays a
  local request beyond its own budget; decision falls back to snapshot logic.
  Pinned by `probeAdmission` unit tests (timeout, bad JSON, 500).
- **I3 — contention demotion is arbitration**: it does NOT increment
  `state.switches` (same rule as peer demotion). Pinned by a decision test
  asserting `switches` unchanged on a contention demotion.
- **I4 — no cloud, no spill**: contention with no candidate/budget still runs
  local. Pinned by a decision test.
- **I5 — no fabricated state**: `localState` becomes "contended" only when the
  probe/mark observed it. Pinned by a ledger-shape test.
- **I6 — single-session unchanged end-to-end**: an idle engine with one session
  behaves exactly as before (probe returns free; no extra notices).

## Named tests
- `overflow.test.ts`: `contention_parity_matrix` (I1), `contention_demotes_without_switch_cap` (I3), `contention_without_cloud_stays_local` (I4).
- `engine-probe.test.ts` (new): `probe_admission_reports_busy`, `probe_admission_fails_open` (I2, timeout + bad JSON + 500).
- `resilient-default/slot-contention.test.ts` (new): probe-busy demotes to cloud; ledger `localState === "contended"` (I5); no-cloud fallback runs local.
