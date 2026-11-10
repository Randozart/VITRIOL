# Verification rollout — execution plan (2026-11-10)

Strategy doc: `formal-verification-2026-11-10.md`. This is the execution
order. One commit per tier, gate before each commit: `npm test` (officina),
`cargo test` (officina/tui), `selfcheck.sh`, `build-patch` canaries.

## Status (2026-11-10, live)

| tier | status | landed as |
|---|---|---|
| T0.1 | DONE | `feat(officina-tui): declared UI phase transition table (verification T0.1)` |
| T0.2 | DONE | `feat(officina-tui): pin the pi RPC contract with goldens + drift detection (T0.2)` |
| T0.3 | DONE (uncommitted at time of writing — see gate note) | `scripts/config-schema.mjs` + `config-drift.mjs` (+tests), selfcheck §4b, blessed baseline `778e3269a3745bd2` |
| T1.a/T1.b | pending | — |
| T3 | pending | — |

Gate state at T0.3: 120 Rust (was 105 pre-T0), 704+22 JS, selfcheck PASS incl. new §4b, canaries 9.

### Deviations from the original plan (recorded)
1. `advance()` was renamed `check()` and made check-only (callers keep their
data mutations; violations are surfaced as `state:` Diag entries in the F9
overlay, never a wedge). Reason: a table gap failing hard would be the very
wedge class this work exists to prevent.
2. `phase()` priority was derived from the UI's visible state: Dialog >
Compacting > Streaming > Modal > Idle. Compacting initially sat under
Streaming, which the wiring tests caught: every compaction event during a
live turn was misclassified.
3. The table gained idempotent rows where pi legitimately re-delivers
(`agent_end` + `agent_settled` on an already-idle UI) and self-loops
(picker open/close while streaming, where Streaming outranks Modal in
priority).
4. T0.2 went further than planned: instead of only pinning the commands we
model, the `pi_contract_is_fully_modelled` test parses the VENDORED
`rpc-types.d.ts` and fails when upstream declares a command we lack. That
test immediately caught the real drift: 13 unmodelled commands
(`get_tree`, `fork`, `abort_bash`, `clear_queue`, auto-compaction /
auto-retry toggles, steering/follow-up mode setters, …). All 13 are now
modelled (wire-complete; `/tree` + `/fork` UI work still pending — the
Phase 5 items from `officina-great-harness-2026-11-10.md`).
5. The `Steer`/`FollowUp` variants were missing pi's `images` field and
`Bash` was missing `excludeFromContext` — shape drift the goldens pin.

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
- DONE, with extensions — see the Deviations section above.

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
- `selfcheck.sh` new section 4b: config schema + drift check (fail on
  missing REQUIRED keys; report drift since last bless, exit 2).
- DONE, with one correction caught by the first live run: pi keys
  `compaction.modelOverrides` by the FLAT combined string
  `"provider/model"` (e.g. `"llamacpp/Lapis Occultus"`), not nested
  objects. The initial schema produced false warnings; fixed and re-verified.

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
1. T0.1 → T0.2 → T0.3 → gate → commit Tier 0 (committed per-tier; the
   original single Tier-0 commit was split because T0.2 landed 13 new
   wire commands — too much to mix with the state table)
2. T1.a → T1.b → gate → commit Tier 1
3. Tier 3 → commit
4. Update `formal-verification-2026-11-10.md` with final status + addendum.

## What each layer actually catches (post-T0 addendum)
- **Transition table** — illegal UI-state events (the wedge class): fires as
  a Diag line, so the failure is VISIBLE instead of a silent freeze.
- **RPC goldens + d.ts parser** — wire drift vs pi: field renames, dropped
  `skip_serializing_if`, new upstream commands. A pi bump now breaks the
  build at `cargo test` instead of grey-ing out features.
- **Config schema + drift** — the 2026-09-03 MTP failure mode on the harness
  side: a silently dropped behaviour-bearing key in
  `~/.vitriol/officina/env` or `~/.pi/agent/settings.json`. Gate:
  selfcheck §4b. Intentional changes: `node scripts/config-drift.mjs
  --bless` after review. Secrets never enter the fingerprint (schema is
  an allowlist, never a file scan).

## Invariants (this work must preserve)
- I1: no change to pi RPC wire format (goldens are the spec, not the code).
- I2: no change to `~/.vitriol/config` engine keys; drift detector is
  read-only on the live config (env/settings only).
- I3: OFFICINA_PAID_ALLOWED=0 invariant untouched — budget properties
  only assert existing behavior.
- I4: build-patch canaries stay at 9+; no new hand-edits into patched/*.
