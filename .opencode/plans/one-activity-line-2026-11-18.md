# One activity line: "compacting…" / "thinking…" / "working…" instead of a stack

**Date:** 2026-11-18 08:30
**Mode:** PLAN (research only — nothing changed)
**Owner:** "currently compacting and thinking and working all sit beside each other. Can we make it
change to 'thinking…' and 'compacting…' when it's performing those other actions, instead of having
two lines sit on top of each other."

## 1. Where the three lines come from

`officina/tui/src/tui/layout.rs` (tail of `chat_lines`):

| # | source | render | condition |
|---|---|---|---|
| 1 | `ChatEntry::Thinking { .. }` — a real transcript entry (expandable, clickable, hitbox) | its own row; while it is the LIVE TAIL of a streaming turn it carries a spinner + char count (`state.rs:4146-4165`) | the entry exists |
| 2 | `is_streaming` (`layout.rs:520-553`) | `🜎 ⠋ working… 12s` | a turn is streaming |
| 3 | `is_compacting` (`layout.rs:554-568`) | `  🝆 ⠋ compacting context… 4s` | a compaction is running |

2 and 3 are **independent `if` statements that each push a line**, and compaction runs *while* a
stream is open — hence two lines stacked. The thinking entry sits above them and adds a third
activity-looking row while reasoning streams.

Elapsed time is also split: 2 uses `state.started` (turn clock) and 3 uses
`compaction_started_at` (compaction clock). Both are honest, so the merged line must keep the
per-activity clock.

## 2. Design: one line, verb by precedence

Introduce a single derived state instead of three independent flags:

```rust
// state.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnActivity { Compacting, Thinking, Working }

impl AppState {
    /// The live thinking block is the tail of the transcript while streaming.
    pub fn thinking_is_live(&self) -> bool;      // mirrors state.rs:4149
    /// What the turn is doing right now, if anything.
    pub fn turn_activity(&self) -> Option<TurnActivity>;
    /// Seconds to show for the current activity (compaction has its own clock).
    pub fn activity_secs(&self) -> u64;
}
```

Precedence (later states do not stack, they replace):

```
compacting  →  "🝆 ⠋ compacting… 4s"
thinking    →  "🜎 ⠋ thinking… 8s"
working     →  "🜎 ⠋ working… 12s"
idle        →  no row at all
```

`layout.rs` then renders **one** line from `turn_activity()`. The existing stalled variant
(`working… · stalled 12s`, muted) is preserved for the `Working` case only — a stalled compaction or
stalled thinking block should keep their own wording, so the stalled suffix is chosen by activity,
not by a boolean.

Trade-offs, stated:

- **The thinking ENTRY stays.** It is content (expandable, clickable), not a status; hiding it would
  lose the operator's ability to read what the model thought. Only the *trailing status line*
  changes. If the owner wants the collapsed thinking row to disappear while the merged line says
  "thinking…", that is a second, separable switch (`hide_thinking` already exists).
- **Compaction keeps its own glyph** (`GLYPH_ALEMBIC`) so the two are still visually distinct even
  though only one is shown at a time.
- **No new state**: `turn_activity()` is derived from `is_compacting`, `is_streaming`, and
  `thinking_is_live()`, so it cannot drift out of sync with them.

## 3. Tests (differential, one per transition)

1. `is_compacting && is_streaming` → exactly ONE activity line, and it says `compacting`.
   Negative control: the old two-`if` shape fails this by containing both phrases.
2. `is_streaming && thinking_is_live()` → one line, `thinking`, no `working`.
3. `is_streaming` alone → one line, `working`.
4. nothing active → no activity line at all (and the transcript does not gain an empty row — check
   `line_entries`/hitbox alignment, which `layout.rs:569` resizes after these lines).
5. Ghost/`stalled` suffix: a stalled `working` reads muted `· stalled Ns`; a stalled compaction keeps
   `compacting… Ns` (no false "stalled" claim about a local summarisation that is legitimately slow).
6. A compaction that ENDS while the turn keeps streaming returns to `working` in one frame (the
   settle logic for the band must not delay this: this line is not part of the settled band).

## 4. Files

- `officina/tui/src/tui/state.rs` — `TurnActivity`, `thinking_is_live()`, `turn_activity()`,
  `activity_secs()` (+ unit tests for the precedence).
- `officina/tui/src/tui/layout.rs` — replace the two `if` blocks with one render; extend the
  existing `compaction_tests` / status-line tests.

## 5. Owner note

Also from this turn: `gh auth status` still reports **"The token in default is invalid"** after
`gh auth login`, so either the login landed on a different account keyring entry or it needs a
`gh auth switch`. Pushes continue to work over SSH; nothing here depends on it.
