# Officina TUI: /settings, markdown rhythm, quicksilver gauge

**Date:** 2026-09-03 00:30 UTC
**Owner:** VITRIOL owner (Ratatui owner; dispatching via opencode)
**Context:** Continuing from the TUI overhaul commit (51ca26b, 26 files). Three independent features, each its own commit per AGENTS.md workflow directive.

---

## Stage 1 — `/settings` command

**Files:** `tui/state.rs` (LOCAL_COMMANDS + dispatch), `tui/mod.rs` (handle_key)

### LOCAL_COMMANDS additions (missing commands)

Add to the visible autocomplete list in `state.rs`:
- `quit` — "quit the TUI (alias /q)"
- `diag` — "toggle diagnostic overlay"
- `clear` — "clear the transcript"
- `config` — hidden alias (no duplicate autocomplete entry; dispatch accepts both)

These are currently handled in mod.rs handle_key but invisible to the user (no `/` autocomplete entry). Adding them means `/` popup shows every local command.

### `/settings` dispatch

Bare `/settings` → push multi-line diag block (4-5 short lines prefixed `⚙`):
```
⚙ glimmer: shimmer
⚙ fire: on prismatic
⚙ available: /settings {glimmer|fire} <args>
```

`/settings glimmer <args>` → delegate to `set_glimmer` + `persist_glimmer` + notice.
`/settings fire <args>` → delegate to `set_fire` + `persist_fire` + notice.
`/settings config` → alias for bare `/settings` (show listing).

`/config` → same dispatch as `/settings`.

Unknown key → usage line as diag.

### Persistence

No changes — glimmer and fire each keep their own files. `/settings` is a thin dispatch layer.

### Tests

Verify: existing 25 tests + new test: `LOCAL_COMMANDS` contains all visible local commands (quit, diag, clear, settings, glimmer, fire, plus RPC-backed ones).

---

## Stage 2 — Markdown: hanging indents + paragraph gaps

**File:** `tui/markdown.rs`

### Hanging indent for list items

`Renderer` gains:
- `indent: usize` — accumulated prefix width of all ancestor lists + items
- `hang: usize` — content column for the current item (indent + own prefix width)
- `prefix_widths: Vec<usize>` — stack of own prefix widths (parallel to `lists`), for safe pop on End(Item)

`Start(Item)`:
1. flush (previous content)
2. compute `(prefix, pw) = item_prefix()`
3. push prefix span + padding into `cur` (prefix + pw columns)
4. `self.indent += pw` → nested content indents correctly
5. `self.hang = self.indent` → continuation wraps here
6. push pw to `prefix_widths`

`End(Item)`:
1. flush (last chunk of this item)
2. `self.indent -= pw` (pop from `prefix_widths`)
3. `self.hang = 0`

`flush()` → call `wrap_line(line, self.width, self.hang)`:
- First line wrapped to `self.width`
- Continuation lines wrap to `width - hang`, prepended with `hang` blank spaces
- Visual: item text flows under the item's text start, not under the bullet

Nested lists: `Start(List)` doesn't change `hang`; only `Start(Item)` does. So a nested item starts at the correct depth.

### Paragraph gaps

Add `Renderer::gap()` method:
- If `out` is non-empty AND last line is NOT already empty AND `cur` is empty → push `Line::from("")` to `out`

Call `gap()` on:
- `End(Paragraph)`
- `End(Heading)`
- `End(CodeBlock)`
- `End(List)` (only when at nesting depth 0 — outermost list)
- `Rule` (before and after the rule line)
- `End(Table)` (after the last table row flush)
- `Start(List)` when depth 0 (gap before list starts)
- `Start(BlockQuote)` when nesting from 0→1

No gap between list items (items should be tightly spaced). No gap at start of document (first line already skipped by guard in `gap()`).

### Wrap update

`wrap_line` signature: `fn wrap_line(line: &Line, width: usize, hang: usize) -> Vec<Line<'static>>`.

When `hang > 0`:
- First line: wrap to `width` (same as now)
- Subsequent lines: each wrapped to `width - hang`, prefixed with `Span::styled(" ".repeat(hang), Style::default())` (invisible padding) — appended before the text spans

Implementation: after `push_chars` splits into lines, insert hang-prefix spans on lines index > 0. The first line already has the prefix from cur.

### Tests

1. Long bullet item text wraps aligned under item text (not under bullet)
2. Nested list indent increases by 2 columns per depth
3. Paragraph gap: `"a\n\nb"` renders 3 lines (content, blank, content)
4. No gap at document start/end (no blank line before first heading, no trailing blank after last paragraph)
5. List items stay tight (no blank line between items)
6. Existing tests pass unchanged (width param change is transparent; hang=0 = old behavior)

---

## Stage 3 — Quicksilver gauge

**File:** `tui/layout.rs` (new fn, called from `render_chat_and_editor`)

### Position

Rightmost column of `chat_area`: `gauge_x = chat_area.x + chat_area.width - 1`. Already reserved — `chat_width = width - 1` means text never touches it. Zero overlap, no text-priority conflict.

### Geometry

`total` and `visible` already computed in `render_chat_and_editor`:
- Thumb height = `(visible as f64 / total as f64 * height as f64).round()` — clamped ≥1
- Thumb top = `scroll as f64 / (total - visible) as f64 * (height - thumb_h) as f64` (maps scroll position to pixel position)
- Full cells: inner integer portion
- Top edge cell: fractional dots 1..k (k = round(frac_top * 4)), left-column braille
- Bottom edge cell: fractional dots 1..k for remaining space

When `total <= visible` (nothing to scroll) OR watermark is up (entries empty) → skip render entirely.

### Braille glyphs

Left-column dots only (column 0 of the braille cell):
- Dot 1 (top): `0x01` → `⠁`
- Dot 2: `0x02` → `⠂`
- Dot 3: `0x04` → `⠄`
- Dot 7 (bottom): `0x40` → `⡀`
- Full left column: `⠁|⠂|⠄|⡀ = 0x47` → `⡇`
- Partial: any subset via bitwise OR, offset from `0x2800`

Top/bottom edge: fractional fill picks `k` dots from top → bitmask, render char.

### Color

`theme::SILVER` (#C0C7CF) — the quicksilver thread. Drawn after chat text + fire tint pass (a thin silver thread through the fire strip reads as a gauge riding the flames). No animation on the gauge itself (keeps it calm; owner's fire/shimmer handles all the sparkle).

### Render order

1. Fire render (backdrop)
2. Chat text + fire tint pass (text priority)
3. **Gauge** (thin chrome column, no overlap with text)
4. Scroll badge (top-left of chat, already drawn)
5. Popup (autocomplete)
6. Editor

### Implementation sketch

```rust
fn render_gauge(frame: &mut Frame, state: &AppState, chat_area: Rect) {
    let total = (state.scroll_max as usize) + chat_area.height as usize;
    let visible = chat_area.height as usize;
    if total <= visible || state.entries.is_empty() {
        return;
    }
    let h = chat_area.height as usize;
    let thumb_h = (visible as f64 / total as f64 * h as f64).round().max(1.0) as usize;
    let scroll_pct = if total == visible { 0.0 }
        else { state.scroll as f64 / (total - visible) as f64 };
    let thumb_top_f = scroll_pct * (h - thumb_h) as f64;
    let thumb_bot_f = thumb_top_f + thumb_h as f64;
    let gx = chat_area.x + chat_area.width - 1;
    let buffer = frame.buffer_mut();

    for row in 0..h {
        let fy = row as f64;
        let dot_bits = braille_fill(fy, thumb_top_f, thumb_bot_f);
        if dot_bits == 0 {
            continue;
        }
        let ch = char::from_u32(0x2800 + dot_bits).unwrap_or('⡇');
        if let Some(cell) = buffer.cell_mut(Position { x: gx, y: chat_area.y + row as u16 }) {
            cell.set_symbol(ch.to_string());
            cell.set_fg(theme::SILVER);
            cell.set_bg(theme::BG);
        }
    }
}
```

`braille_fill(fy, top, bot) -> u32`: returns the left-column braille bitmask for this row. Row `fy`:
- `fy < top - 1` or `fy > bot + 1` → 0 (empty)
- `fy >= ceil(top)` and `fy <= floor(bot)` → full left column (0x47)
- Top edge: partial fill based on `ceil(top) - fy` fraction → dots 1..k
- Bottom edge: partial fill based on `fy - floor(bot)` fraction → dots from bottom up

### Tests

- Gauge renders when scroll_max > 0, skipped when entries empty
- Thumb height ≥1, ≤ total height
- braille_fill returns full 0x47 for interior rows, 0 for exterior rows
- Gauge column (last column of chat_area) is not touched by text rendering (verify `chat_width = width - 1`)

---

## Commit plan

| # | Commit message | Key files |
|---|---|---|
| 1 | `officina: /settings command + complete LOCAL_COMMANDS list` | `tui/mod.rs`, `tui/state.rs` |
| 2 | `officina: markdown hanging indents + paragraph gaps` | `tui/markdown.rs` |
| 3 | `officina: quicksilver braille gauge scrollbar` | `tui/layout.rs` |

Each: `cargo test` → `cargo build --release` → `cp target/release/officina ~/.local/bin/` → append plan addendum → commit.

**Plan doc:** `.opencode/plans/officina-tui-settings-md-gauge-2026-09-03.md` (this file).
