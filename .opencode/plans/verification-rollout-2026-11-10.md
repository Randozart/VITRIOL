# Verification rollout — execution plan (2026-11-10)

Strategy doc: `formal-verification-2026-11-10.md`. This is the execution
order. One commit per tier, gate before each commit: `npm test` (officina),
`cargo test` (officina/tui), `selfcheck.sh`, `build-patch` canaries.

## Tier 0 (commit: "test(officina): executable contract layers — T0")

### T0.1 — TUI state transition table (`officina/tui/src/tui/state.rs`)
- Add `pub enum UiEvent` (the event vocabulary that mutates `AppState`:
  key actions, RPC events, timer expiries — derived from the actual
  mutation call sites in `mod.rs`/`layout.rs`).
- Add `const TRANSITIONS: &[(AppStatePhase, UiEvent, AppStatePhase)]` where
  `AppStatePhase` is a small derived enum (Idle | Prompting | Streaming |
  Dialog | Confirming | ...) extracted from the existing bools/flags on
  `AppState` — the struct keeps its fields, we just *name* the phases.
- Add `fn advance(&mut self, ev: UiEvent) -> Result<AppStatePhase, IllegalTransition>`
  that (a) computes the current phase, (b) walks the table, (c) `debug_assert!`
  + returns the typed error on illegal (phase, event).
- Refactor the top-level event loop in `mod.rs` to route UI-mutating events
  through `advance()` (keep direct field writes for non-phase data like
  scroll offsets — the table only governs phase-level state).
- Property test `state.rs`: every (phase, event) in the table reaches the
  declared target; every (phase, event) not in the table is rejected.

### T0.2 — RPC protocol golden fixtures (`officina/tui/src/rpc/protocol.rs`)
- Golden JSON fixture per `RpcCommand` variant: serialize → exact expected
  string (proves wire format matches `rpc-types.d.ts` v0.83.0).
- Round-trip test: deserialize(golden) == original for every variant.
- Pin comment: "pin: pi-coding-agent v0.83.0 — bump = regenerate goldens".

### T0.3 — Harness config schema + drift detector
- `officina/scripts/config-schema.mjs`: REQUIRED keys for
  `~/.vitriol/officina/env` (speed/correctness-bearing) and the settings
  JSON; exports `validateConfig()` + `fingerprintConfig()` (stable hash of
  sorted provenance-bearing keys).
- `officina/scripts/config-schema.test.mjs`: missing REQUIRED key fails;
  drift vs last-known-good detected; clean config passes.
- `officina/scripts/config-drift.mjs`: stores
  `~/.vitriol/officina/last-known-good.json` (hash + ts + source note);
  `--check` diffs live vs stored, `--bless` updates after an intentional
  change. Mirrors the engine fingerprint doctrine.
- `selfcheck.sh` new section 5: config schema + drift check (warn on drift,
  fail on missing REQUIRED keys).

## Tier 1 (commit: "test(officina): property tests + Kani — T1")

### T1.a — fast-check property tests
Add `fast-check` to officina dev-deps. New test files next to the libs:
- `adaptive-router/overflow.property.test.ts` — never routes
  confidential to cloud; switch count ≤ cap; cooldowns monotonic;
  failover bypasses switch cap iff engine DOWN.
- `_shared/route-ledger.property.test.ts` — append-only shape; 30-day
  retention prunes strictly; entries parseable.
- `_shared/provider-health.property.test.ts` — cooldown rules table holds;
  success clears; expired prune.
- `_shared/budget.property.test.ts` — daily/monthly caps never exceeded;
  `OFFICINA_PAID_ALLOWED=0` ⇒ no paid candidate ever auto-selected.
- `scripts/caduceus-lib.property.test.mjs` — catalog filter: free-only,
  tool-capable, effort-tier dedup.

### T1.b — Kani on TUI pure cores
- Add `kani` to toolchain; `cargo kani` on `officina/tui` with
  `#[kani::mod]` (or per-fn checks) on `state.rs::advance`, the transition
  table invariants, `markdown.rs` measurement/truncation fns, `envfile.rs`.
- CI/selfcheck: `cargo kani` runs in selfcheck `--live` (not the default
  gate, to keep CI fast) + documented in the plan doc.
- Non-goals: ratatui rendering, crossterm, FS I/O stay out of Kani scope.

## Tier 3 — process (commit: "chore(officina): invariants-in-plans convention")
- `docs/officina-guide.md` (or OFFICINA.md): new section "Plan invariants
  contract" — every plan doc in `.opencode/plans/` MUST carry an
  `## Invariants` block; each invariant gets a named test in the same
  commit as the feature (or an explicit deferral line).
- `selfcheck.sh`: new section greps `.opencode/plans/*.md` newer than a
  marker commit for missing `## Invariants` (warn-only at first).

## Order of execution
1. T0.1 → T0.2 → T0.3 → gate → commit Tier 0
2. T1.a → T1.b → gate → commit Tier 1
3. Tier 3 → commit
4. Update `formal-verification-2026-11-10.md` with final status + addendum.

## Invariants (this work must preserve)
- I1: no change to pi RPC wire format (goldens are the spec, not the code).
- I2: no change to `~/.vitriol/config` engine keys; drift detector is
  read-only on the live config (env/settings only).
- I3: OFFICINA_PAID_ALLOWED=0 invariant untouched — budget properties
  only assert existing behavior.
- I4: build-patch canaries stay at 9+; no new hand-edits into patched/*.
