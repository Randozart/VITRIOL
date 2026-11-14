# GRIND mode — persistent build with an armed continuation loop (2026-11-14 11:31)

Operator request: a mode "like Build, but explicitly required to keep going
until there is truly nothing left to do". Name decided: **GRIND**.

Missing piece: a directive alone cannot continue work — pi ends a turn when
the model stops calling tools. The mode therefore ships with a loop extension
(same mechanic as `fix-loop`: `agent_end` → `ctx.sendUserMessage(prompt)`).

## Mode definition (`_shared/agent-mode.ts`)

```
name: grind, label: GRIND, glyph: ∞, color: violet
hint: "persistent build · runs until GRIND: DONE"
```

- Writes unlocked (no `blockNonMdWrites`).
- Directive (every turn): drive the task list to zero; expand remaining work
  into the task list first; never stop at a checkpoints and never ask
  permission for in-scope work; verify changes; terminate only with a final
  `GRIND: DONE` (task list empty + changes verified) or `GRIND: BLOCKED: <reason>`;
  stand down on user intervention.
- `enterDirective`: announces the armed loop once after switching in.

## Cross-extension mode signal

`_shared/agent-mode.ts` keeps `current` as plain module state and pi loads
every `-e` extension through a separate jiti instance (moduleCache: false) —
an importing extension would always read `"build"`. Contract-compliant fix:
agent-mode writes a facts file on switch and `session_start`
(`~/.vitriol/officina/agent-mode.json`: `{version:1, ts, mode, label}`,
atomic tmp+rename). The grind extension reads it (mtime-cached).

## Loop (`officina/.pi/extensions/grind/`)

- `rules.ts` (pure): marker parsing (`GRIND: DONE` / `GRIND: BLOCKED: …`),
  `shouldContinue(state)` with explicit reasons, `continuationPrompt(...)`,
  caps.
- `index.ts` wiring (trigger = `agent_settled`, pi's documented "no
    automatic retry, compaction, or queued continuation will run" point;
    the assistant text for markers arrives via `message_end` so abort and
    settle ordering cannot race a continuation):
  - stand down if: not grind, aborted (settled flag or `stopReason:
    "aborted"`), user input while the loop ran, steering/follow-up input
    queued, caps hit, DONE/BLOCKED marker.
  - DONE is accepted only when task-state has no pending/in_progress items and
    (when the repo has gates) the last verify was green; a DONE claim that
    fails the check gets one reminder turn, then is accepted (no fight).
  - Verify cadence (operator choice): on a DONE claim and every 3rd
    continuation, only when `detectGates` finds gates; failures are injected
    into the next continuation prompt.
  - Disarm: any `input` with `source !== "extension"` (interactive/rpc),
    `agent_settled{aborted}`, switching away from grind.
  - Caps: `OFFICINA_GRIND_MAX_TURNS` (default 24, hard 100);
    `OFFICINA_GRIND_MAX_MINUTES` (default 120). Kill switch
    `OFFICINA_NO_GRIND=1`.
  - Widget line `↻ grind n/24` while armed.
- Reuses `task-state` `getTaskSummary()` (globalThis singleton contract) and
  `verify` `detectGates`/`runVerify` (as `fix-loop` does).

## TUI handoff (`tui/src/tui/`)

- `mode_chip`: `GRIND → ("∞", VIOLET)`.
- The threshold-compaction auto-continue (`CONTINUE_PROMPT`) is skipped while
  the mode chip reads GRIND — grind owns continuation (no double turns).

## Invariants

- **I1 parity**: with mode ≠ grind the loop performs no sends and no extra
  side effects. Pinned by rules + wiring tests.
- **I2 user wins**: any interactive/rpc input disarms the loop before the
  next continuation. Pinned by an `input` test.
- **I3 abort**: `agent_settled{aborted:true}` (Esc) never re-prompts. Pinned.
- **I4 bounded**: continuation stops at turn/minute caps and announces it —
  never an unbounded loop. Pinned by cap tests.
- **I5 single continuation**: at most one auto-prompt per settled turn; the
  TUI compaction auto-continue is suppressed under GRIND. Pinned by
  repeated-`agent_end` idempotence test + a TUI gate test.
- **I6 cache-safe**: the continuation is a tail user message
  (`sendUserMessage`), never a system-prompt rewrite (Rule 7).
- **I7 restart**: loop state is in-memory; a TUI restart ends the loop (the
  mode may still be selected — re-arm is explicit next turn).

## Tests / gates
`rules.test.ts` (markers incl. DONE-with-pending, caps, disarm matrix, prompt
shape), `index.test.ts` (continue once per settle, idempotent, abort/input/
queue disarm, mode-file gate), TUI chip + compaction-gate tests.
`npm test` + `npm run typecheck`, `cargo test` + `cargo fmt --check`,
rebuild + install.
