# Sidebar declutter + header @path + /help modal

**Date:** 2026-09-03 11:00 UTC
**Context:** Owner redundancy audit (approved): four sidebar rows duplicated
  the Rust header or each other. Header absorbs session identity with the
  working folder. /help modal documents every remaining row. Two commits.

**Drift guard (standing rule):** the /help glossary is static Rust text —
ANY future sidebar content change must touch up SIDEBAR_GLOSSARY in
`officina/tui/src/tui/state.rs`.

---

## Stage 1 — Sidebar declutter (`officina/.pi/extensions/session-panel/index.ts`)

| section | action |
|---|---|
| model (P11) | DELETE — header center shows model id |
| title (P12) | DELETE — header right shows session name |
| ingest (P22) | DELETE — rate lives on eng, fill number on ctx |
| engine (P25) | simplify → `eng <gauge> <rate|idle>` (drop slots + boot-cumulative) |
| session (P45) | restructure → `↑X ↓Y · N turns` (cwd + id → header) |
| skills (P50) | relabel `skills ` → `tools ` (renders getRecentTools) |

## Stage 2 — Header session label with path (`officina/tui/src/tui/layout.rs`)

Pure helpers + unit tests:
- `shorten_home(path) -> String` — `/home/<user>/…` → `~/…`
- `session_label(state) -> Option<String>`:
  - named → `"name" @ ~/Projects/VITRIOL`
  - unnamed w/ id → `SESSION ID: #01a06380 @ ~/Projects/VITRIOL`
  - neither → None (current behavior)
- Left-truncate the PATH portion with leading `…` when the label would
  squeeze the brand column below 24 cols.

## Stage 3 — /help modal (`state.rs`, `mod.rs`, `layout.rs`)

- `AppState`: `help_open: bool`, `help_sel: usize`
- `SIDEBAR_GLOSSARY: &[(&str, &str)]` — every sidebar item + meaning,
  including gauge color ramps (capacity teal / mercury idle / activity
  green-cyan decode) and checklist marks
- Keys section (static), Commands section rendered FROM `LOCAL_COMMANDS`
  (single source of truth)
- `/help` command (LOCAL_COMMANDS entry) + F1 keybind; esc closes;
  ↑↓ scrolls; resume-style modal rendering (`render_help_modal`)
- session-panel hints row gains `/help`

## Verification

cargo test (48 + new), vitest 519 + typecheck, release build + install,
2 commits, outcome notes with hashes.

## Commits

| Commit | Notes |
|---|---|
| `465d288` sidebar declutter — header carries session id @ path | 3 layout helper tests |
| `4249006` /help modal — sidebar glossary, keys, commands | hints row leads with /help |

51 cargo tests, 519 vitest, tsc clean. Bin installed both stages.

## Outcome notes

- Rust: enum/struct items are NOT allowed inside `impl` blocks —
  HelpRow/HelpRowKind/help_rows live at module level in state.rs and
  reference `AppState::LOCAL_COMMANDS`.
- Elide coordinate bug caught by test: `rposition` returns window-
  relative indices; snapping `cut = pos + 1` (window coords) into a
  full-path slice produced `…OL`. Correct shrink: `cut -= pos + 1`.
- Default `AppState.cwd` renders as `.` through `to_string_lossy` —
  tests set cwd explicitly before asserting label contents.
- /help closes on esc, enter, or q; scrolls with ↑↓/jk.
