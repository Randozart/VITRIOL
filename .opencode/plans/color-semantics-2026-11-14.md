# Colour semantics + clickable tool verbosity (2026-11-14 11:31)

Operator report: "tool calls are currently red while still running, implying
failure. Same with watchdog boxes. All red, which implies failure where there
is none."

## Diagnosis

| surface | current | problem |
|---|---|---|
| tool (line mode, default) | `theme::warn()` = ORANGE `#FF5F1F` while running | reads as failure; indistinguishable from `RED` errors |
| tool (block/full) | running status + elapsed ORANGE | same |
| watchdog card | edge + label ORANGE for **every** verdict | an `ALIVE` card is an orange/red box (green chip ignored) |
| `ChatEntry::Diag` | `theme::crit()` = RED | ~95 informational pushes render as errors (janitor, thinking-view, compaction summary, settings, scans) |

## Decisions (operator)

1. **Full severity split**: `Diag` becomes neutral; add `Warn` (amber) and
   `Error` (red) variants; convert true failures.
2. **ORANGE → amber `#FFA000`** so warnings can't be confused with failures.
3. **Watchdog cards update in place** for the same command+verdict (the 4×
   ALIVE stack comes from per-judgment notifications).
4. **Tool clicks cycle three states**: closed (line) → formatted (block) →
   full (args + output) → closed.

## Implementation

### Palette (`officina/tui/src/theme.rs`)
- `ORANGE` `#FF5F1F` → `#FFA000`. `RED #F85149` stays the only failure hue.

### Severity model (`state.rs`, `mod.rs`)
- `ChatEntry::Diag(String)` — neutral silver (info voice).
- `ChatEntry::Warn(String)` — amber, `⚠` prefix.
- `ChatEntry::Error(String)` — red, `✗` prefix.
- `apply_notify`: `error` → `Error`, `warning` → `Warn`; hang-watchdog path
  unchanged (own card).
- Conversions: genuine failures → `Error` (session scan ×2, import/import
  task, compaction failed, `rpc … failed`, no models available). Watchdog
  aborts ("never started", "idle … no result", "abort did not settle") →
  `Warn`. Everything informational stays neutral `Diag`.

### Tools
- Running text/glyph → `theme::live()` (cyan); elapsed muted; completed green;
  failed red.
- `ChatEntry::Tool.expanded: Option<bool>` → `click_mode: Option<ToolVerbosity>`.
  Click cycles the *effective* mode: Line → Block → Full → Line; `None`
  follows the `/tools` config until the first click. Render cache cleared on
  each toggle.

### Watchdog card
- `watchdog_edge(verdict)`: `ALIVE` green, `JUDGING` gold, `HUNG` amber,
  unknown/raw muted; applied to edge, label, field keys.
- `apply_notify` replaces the last card when the new facts doc has the same
  `command|verdict` key (no duplicate stacks); appends otherwise.

### TUI compaction banner and held row keep their current colours
(amber now = amber; held row gold; running-working-line cyan).

## Invariants

- **I1 parity**: a tool that is running is never rendered in `RED`; a failed
  tool always is. Pinned by render tests.
- **I2 severity**: `Diag` never uses RED; `Error` always does. Pinned by
  variant render tests + `apply_notify` mapping tests.
- **I3 watchdog truth**: an `ALIVE` verdict card is never amber/red-bordered.
  Pinned by `watchdog_edge` unit test.
- **I4 dedupe**: repeated identical watchdog facts replace the previous card
  (one card per command+verdict). Pinned by a facts-file test.
- **I5 tool cycle**: clicking a tool always advances Line→Block→Full→Line and
  never mutates `/tools` config. Pinned by a state test.

## Tests / gates
`cargo test` + `cargo fmt --check`; rebuild + install to `~/.local/bin/officina`.
New tests: running tool fg == CYAN; Diag neutral; Warn amber; Error red;
`apply_notify` mapping; `watchdog_edge`; card dedupe; tool cycle 3-state.
