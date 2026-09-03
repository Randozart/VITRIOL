# Keymap safety + drag-select text copy

**Date:** 2026-09-03 12:30 UTC
**Owner driver:** "just so you don't accidentally quit" — five quit paths
existed, two were traps (idle-Esc quit the app even with text in the input;
idle-^c likewise).

## Keymap

| binding | before | after |
|---|---|---|
| ^q | unconditional quit ("GUARANTEED") | removed |
| ^d | unconditional quit | removed |
| Esc (idle) | QUIT | no-op (modals still close; streaming still aborts) |
| ^c (idle) | QUIT | copy |
| ^c (streaming) | abort | copy |
| ^esc | — | quit (the only chord; /quit also works) |
| drag (left) | ignored | text selection |

## Text selector

- Left-drag across the transcript: `sel_anchor`/`sel_head` screen coords,
  COLD_BLUE full-row highlight (gauge column excluded), drawn after the
  fire-tint pass. Plain click clears; wheel scroll clears; selection
  survives mouse-up (so ^c works after the drag).
- ^c copies selection first, else `last_assistant()` (latest non-empty).
- Clipboard: wl-copy → xclip → xsel (real clipboards), then OSC52
  (`ESC]52;c;<b64>BEL`) for honoring terminals. Hand-rolled base64 — no
  new crates. OSC52 capped at 100 KB.
- Whole-LINE granularity: wrapped continuations copy intact rather than
  mangled mid-glyph — deliberate tradeoff, noted for the owner.
- `last_chat_area` stashed per frame so handle_key can map selection rows
  → transcript lines without a Frame (offset-from-bottom math reused).

## Commits

| Commit | Notes |
|---|---|
| `47378f0` keymap safety + drag-select text copy | 4 new tests, 55 total |

55 cargo tests green; bin installed.

## Outcome notes

- Match-arm order bug caught before compile: the ^esc guard arm must
  precede the plain Esc arm, or the unguarded arm swallows the chord.
- First extract_selection test used a 3-row viewport for 6 lines —
  offset-from-bottom anchoring showed the tail (blank rows) and the
  single-row selection correctly yielded None. Viewport widened to 6.
