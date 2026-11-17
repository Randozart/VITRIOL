# QOL: user turns get a background band (2026-11-16, PLAN)

Proposal: give every `ChatEntry::User` block a background tint, so the operator
can see at a glance where their own turn starts and ends in a long transcript.

Status: **planned, not built.** This document is the research + the concrete
change set.

## What the transcript does today

`chat_lines()` (`officina/tui/src/tui/state.rs:3985-4002`):

```rust
ChatEntry::User(text) => {
    for (i, seg) in wrap_text(text, width.saturating_sub(2)).into_iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(format!("{} ", theme::GLYPH_USER), Style::default().fg(theme::GREEN)),
                Span::raw(seg),
            ]));
        } else {
            lines.push(Line::from(vec![Span::raw("  "), Span::raw(seg)]));
        }
    }
}
```

- No background is set anywhere; the whole chat rect is painted with `theme::BG`
  by the paragraph (`layout.rs:786`).
- Lines are **pre-wrapped** to `chat_width` (`width - 1`, the last column is the
  scroll gauge), so a full-width band is one character short of the area — the
  same convention the selection highlight uses.

## The mechanism already exists — twice

1. **Selection highlight** (`layout.rs:833-852`) walks the frame buffer after the
   chat paragraph and sets `cell.set_bg(theme::COLD_BLUE)` across full-width rows,
   gauge column excluded.
2. **Fire tint** (`layout.rs:778-802`) does the same walk for `fg`, and skips
   cells whose bg is `theme::PANEL` (opaque tool plates).

So a per-entry background is a *third* pass of the same shape — and, critically,
`state.line_entries` already maps every rendered row back to its entry index
(built in `chat_lines`, consumed by the thinking/tool hitboxes at
`layout.rs:610-645`). No new plumbing is needed: iterate the visible window,
and for rows whose entry is `User`, paint the row.

## Proposed design

**Band, not block tint.** Full-width row (minus the gauge column) filled with a
new `theme::USER_BG`, plus the existing green `☿` glyph left as the accent. That
reads as "this is my turn" without touching glyph colours, so markdown, code and
tool output stay exactly as they are.

Palette: one new token, deliberately close to `BG` (`0x0D1117`) so it is a
*turn marker*, not a highlighter — e.g. `0x13,0x1A,0x22` (a half-step toward
`PANEL`, `0x161B22`). Text contrast is unaffected because only the background
changes.

**Where the pass goes:** immediately after the chat paragraph render and
**before** the fire tint and selection passes (`layout.rs:786` → then bg band →
tint → selection). Ordering gives the right semantics for free:

| pass | wins over |
|---|---|
| chat paragraph (BG) | nothing |
| **user band (USER_BG)** | BG |
| fire tint (fg only) | nothing — bg preserved |
| selection (COLD_BLUE bg) | the band, on selected rows (correct: selection is the operator's intent) |

**Blank separators:** `chat_lines` pushes the inter-entry blank row with the
*current* entry's index, so a naive pass would tint the gap under a user turn.
Rule: paint only rows whose mapped entry is `User` **and** whose line is not
blank (check the buffer row's symbols, as the tint pass does).

**Scrolling:** the pass uses the same visible-window mapping as the hitboxes
(`start` + `chat_area.height`), so a scrolled-back view colours the rows it
actually shows.

## Toggle + persistence

New `/chat-bg on|off|` (bare = toggle), registered in
`settings.rs::SETTINGS` so `/settings` lists it, mirroring `/fire`:

- `AppState.user_turn_bg: bool` (default **on** — the owner asked for it);
- `state.set_user_turn_bg(args) -> String` (same shape as `set_fire`);
- `mod.rs::persist_chat_bg(state)` writing `~/.vitriol/officina/chat-bg` (the
  `persist_fire` pattern), read at startup.

Default-on is the right call for a QOL feature; `/chat-bg off` restores today's
look byte-for-byte.

## Tests

1. **Parity (the load-bearing one):** with `/chat-bg off`, a two-frame render is
   byte-identical to the current build (golden screen string).
2. **Band exists:** with it on, every row of a multi-line user turn carries
   `USER_BG`; the row of the following assistant entry does not.
3. **Gap not painted:** the blank separator under a user turn keeps `BG`.
4. **Multi-line turn:** a wrapped user message (3+ rows) is banded on all rows.
5. **Gauge column untouched:** the last column of the chat area keeps `BG`.
6. **Selection wins:** a selection over a user row shows `COLD_BLUE`.

## Risks / trade-offs

- **Visual noise.** Every user turn becomes a band; in a rapid back-and-forth
  transcript that can read as stripes. Mitigation: the tint is a half-step, and
  the toggle exists. If it still reads heavy, the fallback is a 1-column left
  accent bar instead of a full band (same pass, one column wide).
- **Themes.** If the TUI later gains per-mode palettes, `USER_BG` must be a
  function of the palette rather than a constant; keeping it in `theme.rs` beside
  `BG`/`PANEL` makes that a one-line follow-up.
- **Fire tint interplay.** The tint pass writes `fg` only, so the band survives;
  the one exception is `theme::PANEL` cells (tool plates) which the band will
  never cover anyway (different entry type).
- **Performance.** One buffer walk over ~40 rows × ~120 columns per frame, the
  same cost as the selection pass. Negligible.

## Files that would change

- `officina/tui/src/theme.rs` — `USER_BG`.
- `officina/tui/src/tui/state.rs` — `user_turn_bg` field, `set_user_turn_bg`,
  and a helper exposing "which visible rows belong to User entries".
- `officina/tui/src/tui/layout.rs` — the band pass (after the chat paragraph,
  before tint/selection).
- `officina/tui/src/tui/settings.rs` — the `/chat-bg` entry + `SETTINGS` list.
- `officina/tui/src/tui/mod.rs` — `persist_chat_bg`, startup read.
- tests in `layout.rs`/`state.rs` per the list above.

Effort: ~120 lines with tests. Display-only, no RPC, no engine, no persistence
format change beyond one new small preferences file.
