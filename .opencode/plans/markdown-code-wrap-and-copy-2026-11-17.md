# Markdown code: wrap or copy — findings and plan

**Date:** 2026-11-17 19:30
**Mode:** PLAN (research only)
**Owner:** "markdown formatting of ` code ` doesn't wrap around in regular text. It should, otherwise I
can't copy paste it. OR, I want a copy paste button for those blocks."

## 1. What the renderer actually does

| case | path | behaviour today |
|---|---|---|
| **inline code** in a paragraph (`` `x` ``) | `E::Code` → `push()` → `flush()` → `wrap_line()` (`markdown.rs:236, 713, 811`) | **wraps** — hard-broken at `width` chars, continuation lines get the `hang` indent |
| **fenced / indented code block** | `code_event()` (`markdown.rs:366-378`) | **silently truncated**: `line.chars().take(width-1)` per source line, no marker, no continuation line |
| tables | `render_table()` | cells wrap, columns shrink |
| transcript paint | `elide_line()` (`layout.rs`, added today) | a line that still exceeds the pane gets `…` |

So the "doesn't wrap" case is the **fenced code block**, not inline code — `code_event` throws away
everything past the width with no indication. That is the thing that makes it uncopyable: what is
not drawn cannot be selected, and `extract_selection()` (`state.rs:3887`) reconstructs text from
the rendered lines, then trims them.

Two second-order problems even when wrapping does happen:

1. **Wrapping corrupts the payload.** A wrapped code span carries the `hang` indent on every
   continuation line, so a paste of a selection contains inserted leading spaces (and newlines that
   the source did not have). Wrapping makes code *readable* but not *copyable*.
2. **There is no per-block copy.** `^c` copies the drag selection, else the last assistant reply.
   A code block has no affordance of its own.

## 2. Plan

### Option A — click-to-copy on a code block (recommended primary)
The raw text is still in `ChatEntry::Assistant(text)`; the renderer is the only thing that lost it.
So:

- Record a **hitbox per code block** (entry index + byte range), the same way `tool_hitboxes` /
   `thinking_hitboxes` already work from `line_entries` (`layout.rs:623-645`), so scrolling keeps the
  mapping exact (the existing code already applies `start`/`last_visible`).
- **Click** a block → copy the **exact source** (re-parse the entry's markdown with pulldown and take
  the Nth code block) via the existing `copy_to_clipboard()` (`mod.rs:3667`, wl-copy/xclip/xsel →
  OSC52 fallback). Notice: `copied 42 lines (1 204 chars) · block 2/5`.
- `^y` (or `^b`) → copy the **last** code block, falling back to the last assistant reply.

Why this first: it solves the stated goal (paste the code) without changing geometry, and it is
immune to wrapping and to the elision added today.

### Option B — wrap long code lines (recommended secondary, small)
In `code_event`, replace the truncation with wrapping:

- keep syntect highlighting per source line, then feed each rendered line through `wrap_line()` with
  a continuation prefix (`"  │ "` or `"↪ "`) so the eye knows it is one logical line;
- append a `…` marker only when a single *token* (no spaces) still exceeds the pane;
- never truncate silently again.

Cost: multi-line code blocks get taller. Benefit: nothing is ever invisible, even for readers who
never click.

### Option C — horizontal pan (not recommended now)
`⇥/⇤` or drag-to-pan per block. Most faithful to a terminal, but adds per-block scroll state that
must be kept aligned with the transcript's own scroll (`SCROLL-1`) — a new class of drift for a
problem the copy action already solves.

### Copy fidelity contract (the test that matters)
Whatever the display does, the clipboard payload must equal the **source block byte-for-byte**:

- test: render a block with a 300-char line at width 60, copy block 1, assert the payload contains
  the full 300 chars, no `…`, no `hang` indent, and the same leading whitespace as the source;
- test: hitboxes resolve to the right block after `scroll > 0` and after a width change;
- test: a code block inside a list/blockquote still copies without the quote prefixes.

## 3. Files

- `officina/tui/src/markdown.rs` — `code_event` wrapping (B).
- `officina/tui/src/tui/state.rs` — block extraction from `ChatEntry::Assistant`, hitboxes.
- `officina/tui/src/tui/layout.rs` — hitbox registration (existing pattern), `…` marker.
- `officina/tui/src/tui/mod.rs` — click handling + `^y`, notice text.
