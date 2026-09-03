# Carry preservation extensions abroad + seed the eviction contract

**Date:** 2026-09-03 18:00 UTC
**Owner report:** "the agent isn't actually pruning or updating the todo or
  the scratchpad."

## Root cause (confirmed)

The owner works in `~/Projects/ontic` — no `.pi/extensions`, no
`.officina/extensions`. The Ratatui bridge's carriage list
(`officina_extensions_to_carry`, bridge.rs:43) carries only
`llama-cpp-provider`, `agent-mode`, `vitriol-decode`, `session-panel`.
**task-state and scratchpad are not carried** → in ontic sessions the
agent has no `update_tasks` / `scratchpad_write` tools, never sees the
tails, and the Stage-4 eviction contract never reaches it.

Secondary (seed problem): `renderTaskBlock` returns "" when no tasks
exist on disk → the tail (and contract) is skipped until the agent writes
its first task. Nothing motivates the first write.

Verified safe to carry: `-e` registers the ORIGINAL paths (no copying —
task-state's `../_shared/events.ts` resolves); scratchpad is
self-contained; disk state lands in `<cwd>/.officina/` (per-project,
branding-shim default); the tail injection is pi's `context` hook
(frontend-agnostic — works under the TUI's RPC mode).

## Stages

A. `bridge.rs`: carriage list += "task-state", "scratchpad". Rebuild +
   install the TUI binary.
B. `task-state/state.ts` renderTaskBlock: empty list → minimal two-line
   contract block (no tasks yet + the eviction line) so the contract
   reaches the model from turn 1. Update pure-fn tests.

## Verify

cargo test (66) + rebuild + install; vitest (task-state pure-fn tests
updated) + tsc; owner restarts Officina in ontic — sidebar gains
note/tasks, contract rides every context build.

## Out of scope (flagged)

Carrying knowledge/small-lane/background-lane abroad — separate decision.
