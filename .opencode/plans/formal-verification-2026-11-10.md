# Catching Officina bugs earlier — verification strategy (2026-11-10)

## Findings: what the bug classes actually are

From the post-mortems, AGENTS.md contracts, and the 2026-11-10 wave:

| # | Bug class | Examples | What actually catches it |
|---|-----------|----------|--------------------------|
| 1 | **Cross-process/state contracts** (module state, globalThis singletons, RPC ordering) | jiti-isolation triple-bug (stale sidebar, vanished task rows, triple engine pollers); RPC toolCall without toolResult wedging (session-stall 2026-09-07) | Contract tests that recreate the hostile environment (jiti-isolation.test.ts already does this well) + protocol round-trip tests |
| 2 | **Silent config/flag drift** | `[spec]` dropped from `~/.vitriol/config` → −40% decode for weeks (mtp-flag-drift postmortem); 8-31 config retarget | Provenance: fingerprint diffing (engine-side, exists) + a **config schema with required-keys + drift detector** on the harness side |
| 3 | **Vendor patch loss** | P4/P8/P9 patches hand-edited into generated file, wiped by regeneration (2026-09-01) | Already solved: build-patch canaries (9). Extend the same canary pattern to anything else regenerated |
| 4 | **State-machine / UI state bugs** (TUI: 3,113-line state.rs with 119 fns; layout.rs 1,955) | markdown overflow, session guard, cache scoping — the recurring "TUI wedge / wrong pane" class | Property tests on state transitions + a declared transition table (see below) |
| 5 | **Concurrency/liveness** (pollers, watchdog, event-loop wedges) | session-stall self-heal mystery; triple pollers | Mostly NOT statically catchable; needs liveness tests + timeouts-as-contracts |
| 6 | **Protocol drift vs pi-coding-agent** (pinned vendor, v0.83.0 rpc-types) | `officina/lapis not found` in sub-coders today is an example of runtime contract drift | Wire-format golden tests against the vendored `rpc-types.d.ts` |

**Key observation**: the bugs that hurt most (1, 2, 3) are *contract* bugs, not logic bugs.
Full formal verification (Kani/Verus) catches a small slice of class 4 at high effort.
The highest-leverage move is **making contracts executable and diff-checked**, not a theorem prover.

## Plan (ordered by ROI)

### Tier 0 — free, do first (no new dependencies)
1. **Transition table for the TUI state machine.** In `officina/tui/src/tui/state.rs`
   declare a `const TRANSITIONS: &[(State, Event, State)]` table next to `AppState`;
   route all mutations through one `fn advance(&mut self, ev: Event) -> State`
   that walks the table (illegal transitions `debug_assert!`). Add a
   **property test**: every legal transition from the table round-trips; every
   (state, event) not in the table is rejected. This is the cheapest "formal"
   win in the repo — it converts the 119-fn mutability soup into a checked graph.
2. **RPC protocol golden tests.** `protocol.rs` already claims "matches
   rpc-types.d.ts exactly". Make it provable: golden JSON fixtures for every
   RpcCommand/RpcEvent variant (serialize → assert exact bytes), plus a
   deserialize round-trip property test. Pin to pi version; bump = diff review.
3. **Config schema + drift detector (harness side).** `~/.vitriol/officina/env`
   + settings JSON: a small `officina/scripts/config-schema.mjs` with the
   speed/correctness-bearing keys marked REQUIRED (mirrors the engine's
   fingerprint doctrine). selfcheck.sh gains a section: "config schema +
   drift vs last-good" (store a `last-known-good.json` hash, like the engine
   does with fingerprints). Catches class 2 on the harness side.

### Tier 1 — cheap properties, high catch-rate
4. **fast-check (property-based tests) for the pure policy cores** — all of
   these are already pure functions, so it's near-free:
   - `adaptive-router/overflow.ts` (idle/busy spill, switch cap, dwell,
     return budget — 13 unit tests exist; properties: "never routes
     confidential to cloud", "switch count never exceeds cap", "cooldown
     monotonic")
   - `route-ledger.ts` (30-day retention, JSONL append-only shape, mode 600)
   - `provider-health.ts` (cooldown rules table: 404/401/429/5xx → duration;
     properties: "success clears cooldown", "expired entries prune")
   - `budget.ts` (2M/day, 40M/month; "paid never auto-routed when
     OFFICINA_PAID_ALLOWED=0")
   - `caduceus-lib.mjs` catalog filter (free-only, tool-capable, effort-dedup)
5. **Kani (Rust) on the TUI pure cores only** — `state.rs` transition table,
   `markdown.rs` measurement/truncation, `envfile.rs` parsing. Kani needs no
   annotations for the basics (overflow, index, deadlock checks) and handles
   `#[kani::mod]`. NOT for: ratatui rendering, crossterm I/O, anything that
   touches the filesystem. Cost: one `cargo kani` in CI (a few min), moderate
   learning. Verdict: worth it for state.rs after Tier 0.1 lands (the table
   gives Kani a small, checkable surface).

### Tier 2 — situational, only if a class recurs
6. **Miri** for the engine-side Rust (engine/) — UB/pointer bugs; not for
   Officina's JS-centric surface.
7. **Typestate in TS** (state machine types) — only if a specific extension
   keeps having illegal-state bugs; the `globalThis` singleton pattern already
   covers the cross-extension class.
8. **Full formal (Verus/Verus-style specs)** — verdict: NOT worth it for this
   repo. UI/event-loop code is I/O-bound; the contracts that matter are
   behavioral, and vitest contract tests + properties express them in a
   maintainable form. Keep as a paper option for a future engine-side kernel.

### Tier 3 — process
9. **Add a `## Invariants` block to every new plan doc** (the convenience
   doc already has one — make it mandatory, it's the cheapest regression
   spec that exists) and have the plan's final checklist turn each invariant
   into a named test.
10. **Selfcheck additions**: wire Tier 0.3 (config schema) and a
    "transition table nonempty + advance() used" grep into
    `officina/scripts/selfcheck.sh` so the gate enforces the pattern, not just
    the tests.

## Explicit non-goals
- No theorem prover on the JS surface.
- No formalization of the pi RPC *transport* (it's stdin/stdout JSONL; the
  golden tests + liveness timeouts are the right layer).
- No re-architecture of `state.rs` — keep the `AppState` struct, add the
  table and route mutations through `advance()`.

## Effort estimate
- Tier 0: 1–2 focused sessions (state table + goldens + config schema)
- Tier 1: 1 session for fast-check suite + Kani setup
- Tier 2: on demand
