# CTX bar (data fix) + top-bar divider — 2026-11-11

## Context

Operator request (2026-11-11): the context-fill status row ("current ctx vs
total ctx") added by commit `189149c` was **not visible** in the Rust TUI, and
the top bar was **missing a divider** (the sidebar and the ctx row both have a
`BORDER_DIM`-on-`PANEL` rule; the top bar does not).

## Root causes (verified against the code, not assumed)

1. **Stale binary.** `~/.local/bin/officina` is built **Nov 10 19:57**; the
   ctx-bar commit landed **Nov 11 09:40**. The running binary predates the
   feature.
2. **Wiring bug (would survive a rebuild).** `handle_event` (tui/mod.rs)
   reads `contextUsage` off the `agent_end` event. But pi's wire `agent_end`
   is only `{ type, messages, willRetry }` (`core/agent-session.js:738,766`) —
   it never carries `contextUsage`. That field lives only in
   `getSessionStats()` (`agent-session.js:3515`), which the interactive-mode
   footer consumes. So `ctx_usage` never populates and the row hides itself.
   The `ContextUsage` struct in `protocol.rs` is dead code on this path.

## Plan

### A — ctx bar data source (protocol.rs + mod.rs)
The RPC surface **does** expose `get_session_stats` (`modes/rpc/rpc-mode.js:466`)
which returns `session.getSessionStats()` →
`{ tokens: { input, output, cacheRead, cacheWrite, total }, cost, contextUsage: { tokens, contextWindow, percent } }`.

- Add `RpcCommand::GetSessionStats` + a `SessionStats` response struct in
  `protocol.rs`.
- In `tui/mod.rs`, on `agent_end` (and once at session start), fire
  `get_session_stats` and parse `contextUsage.{tokens,contextWindow}` into
  `state.ctx_usage`. Remove the dead `agent_end.contextUsage` read.
- `state.model.context_window` remains the fallback denominator if
  `contextWindow` is absent.

### B — top-bar divider (layout.rs)
`render()` is `[header 1px][blank 1px][body][footer 1px]`. The blank row is a
deliberate "breathing" seam (owner request 2026-09-03). Replace it with a 1px
`BORDER_DIM` `─` rule on `PANEL` (same convention as sidebar/ctx-bar). Height
stays 1px so the transcript keeps its space.

### C — rebuild + reinstall `officina`
So A+B are actually visible. This is what was blocking "I don't see this".

## Invariants
- I1: with no session stats reported, the ctx row stays hidden (no behaviour
  change for the no-data case).
- I2: the divider adds 0 rows — the breathing row stays 1px, so the transcript
  and composer keep their space.
- I3: a bare `cargo build`/`cargo test` is unaffected by the Kani-gated proofs
  (already true; no new Kani surface here).

## Gates
- `cargo test` (Rust suite green)
- `cargo fmt --check` clean
- `cargo clippy` no new warnings
- Rebuild + `cp` to `~/.local/bin/officina`; confirm the row appears after a
  turn completes and the top rule renders.
