# Console paints BG universally — no terminal bleed-through

**Date:** 2026-09-03 14:30 UTC
**Owner:** "PANEL should stay PANEL. I just want BG to be used below panel
  universally without the terminal background peeking through."

## Root cause

The chat `Paragraph` has no style — ratatui only patches cells actual text
spans touch. Blank rows, line tails, trailing cells, the gap row: all
`bg = None` → the TERMINAL's own theme background shows through. Console
reads as two different darks (TUI `BG` #0d1117 vs terminal theme bg).

## Fix (layout.rs only)

1. Chat paragraph gains `.style(bg(theme::BG))` — Paragraph::render calls
   `buf.set_style(area, style)` first: the entire console rect becomes
   explicit BG. Unconditional — fresh screen too, which puts the stone on
   the explicit BG stage (set_style patches bg only; glyphs + DIM kept).
2. Gap row: always painted BG (motto optional as before — fresh screens
   skip the motto, not the paint). Fire dots rising through the gap keep
   their flame fg (set_style patches bg only; the motto line's cells get
   motto symbols for the motto's width).

## Unchanged

PANEL exactly where it is (sidebar, prompt bar, code spans); selection
COLD_BLUE; fire tint pass (fg-only); gauge; header/footer (already BG).

## Verify

62 cargo tests green, build, install, commit, push.
