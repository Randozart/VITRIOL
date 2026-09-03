# Officina Tool Verbosity System + Green Shimmer Streak

**Date:** 2026-09-03 03:00 UTC
**Context:** Three tool display modes, per-tool config with override strictness,
  ctrl+v hotkey, /tools command + resume-style modal picker, green streak in
  watermark shimmer. Extends the TUI overhaul (51ca26b → d83bf90).

---

## Data model (`state.rs`)

### ToolVerbosity enum

```
Line  — single line (status quo)
Block — pi-like bordered block: header + args summary + output preview (~30 lines)
Full  — full pretty-printed args + live streaming output + full output (cap 2000)
```

Order: Line < Block < Full (used in min/max resolution).

### ChatEntry::Tool extended

```rust
Tool {
    tool_call_id: Option<String>,
    name: String,
    summary: String,              // collapsed line text (always present)
    args: Option<serde_json::Value>,
    output: Vec<String>,          // raw output lines (capped at Full cap on ingest)
    output_truncated: bool,
    running: bool,
    error: bool,
    rendered: Option<(usize, Vec<Line<'static>>)>, // cache: (width, gen)
}
```

### Per-tool config

`ToolOverride` struct per tool:
```rust
struct ToolOverride {
    mode: ToolVerbosity,
    strictness: Strictness, // Pinned | AtLeast | AtMost
}
```

Global default: `tool_default: ToolVerbosity` (default: Line).

Config file `~/.vitriol/officina/tui-tools`:
```
default line
write block+
bash line-
read block!
```

### Effective mode resolution

```rust
fn effective_mode(tool_name: &str, global: ToolVerbosity, overrides: &HashMap<String, ToolOverride>) -> ToolVerbosity {
    match overrides.get(tool_name) {
        None => global,
        Some(ToolOverride { mode, strictness: Pinned }) => mode,
        Some(ToolOverride { mode, strictness: AtLeast }) => global.max(mode),
        Some(ToolOverride { mode, strictness: AtMost }) => global.min(mode),
    }
}
```

### Output capping

Full mode ingest cap: 2000 lines. Block/Line: same 2000 cap (sliced at render).
When truncating: keep first 1500 + last 500, insert `… {n} lines omitted` marker.
Live updates (tool_update): append to output vec, bump entry's rendered cache.

### Tool call ID matching

`tool_end(tool_call_id, name, result_text, error)`:
1. Scan entries reverse for matching `tool_call_id` if Some
2. Fallback: scan reverse for running entry with matching `name`
3. No match → ignore (defensive)

---

## Hotkey (`mod.rs`)

**ctrl+v** cycles global default: Line → Block → Full → Line.
Persists via `persist_tools_config(state)`. Shows notice.

## Commands

`/tools` bare → print config table as Diag lines (203 prefix, gold):
```
⚙ global: line
⚙ write: block+
⚙ bash: line-
⚙ clear /tools <name> to remove override
```

`/tools <name> <mode>[!|+|-]` → set override for tool name.
`/tools <name> clear` → remove override.
`/tools default <mode>` → set global default.

Also accept `/settings tools ...` delegating to same.

## Modal picker (resume-style)

`/tools` with **ctrl+g** (or ctrl+t?) hotkey opens modal picker:
- Full list of known pi tools (bash, read, write, edit, find, grep, ls)
  + any overrides already configured
- Select tool → mode cycle prompt (Line/Block/Full)
- Strength sub-prompt (pinned/at-least/at-most) or immediate apply
- Uses same modal pattern as resume (centered block, ↑↓ select, enter confirm, esc cancel)

State fields: `tools_modal_open`, `tools_modal_sel`, `tools_modal_entries`.

Modal layout: centered block (w=56), rows:
```
╭ tool verbosity config ─────────────────────╮
│  ▸  bash        line−                      │
│     read        block!                     │
│     write       block+                     │
│     edit        (global: line)             │
│     find        (global: line)             │
│     grep        (global: line)             │
│     ls          (global: line)             │
│  ─── global: line ───                      │
│  ↑↓ select · enter change mode · esc close │
╰────────────────────────────────────────────╯
```

## Render (`layout.rs`)

### Line mode
Current single line: `✓ name summary` / `🝥 name` (unchanged)

### Block mode
Bordered block (rounded, BORDER_DIM border):
```
╭ ⌬ write ▸ src/main.rs ──────────────────
│ ▌args   { "path": "src/main.rs", ... }
│ ▌output
│   use tokio::sync::mpsc;
│   // bridge...
│   … 3 more lines
╰ ✓ ────────────────────────────────────────
```

- Header: `⌬ name tool_summary_short` (one-line args summary, same as collapsed)
- Args section: pretty-printed JSON (cyan keys, TEXT values, wrapped to width)
- Output section: first ~30 lines, then `… {remaining} more lines`
- Running tools: `🝥` + live tail (last ~8 output lines)

### Full mode
Same block structure as Block but:
- Args: full pretty-printed JSON (no one-line summary)
- Output: live streaming (all lines, up to 2000 cap)
- Running: full live tail (last ~50 lines)
- Completed: all output lines shown

### Cache
Per-entry render cache keyed `(width, tools_gen)` where `tools_gen` is a u64
incremented on any config/toggle change. Invalidates all entries.

---

## Green shimmer streak (`watermark.rs`)

Add a trailing green band to Shimmer mode:
- Band width: `STREAK_BAND = 3.0` (half of SHIMMER_BAND = 6)
- Offset: `STREAK_OFFSET = 10.0` cells behind the silver sweep position
- Green peak: `SHIMMER_GREEN_PEAK = (0x3A, 0x6E, 0x4A)` — dim emerald,
  same subtle luminance as silver peak, mysterious not Christmas
- Computation: `d_streak = d - (pos - STREAK_OFFSET)`, same intensity formula
- Blend: where green intensity > 0.05 AND >= silver intensity → green color;
  otherwise silver. No additive mixing (would desaturate both).

---

## Persistence

`~/.vitriol/officina/tui-tools` — human-readable, one directive per line:
- `default <mode>`
- `<tool_name> <mode>[!|+|-]`

Loaded on startup. Saved on any change (ctrl+v cycle, /tools set, modal change).

---

## Commits

| # | Message | Key files |
|---|---|---|
| 1 | `officina: tool verbosity plumbing — ids, live updates, mode resolution` | `tui/state.rs`, `tui/mod.rs` |
| 2 | `officina: tool renderers + ctrl+v + /tools config + modal picker` | `tui/state.rs`, `tui/layout.rs`, `tui/mod.rs` |
| 3 | `officina: green streak in watermark shimmer` | `tui/watermark.rs` |

Each: cargo test → release build → install → plan addendum → commit.
