# Gauge direction fix + row-clip fix + carved VITRIOL motto

**Date:** 2026-09-03 13:30 UTC
**Owner:** three items — inverted gauge fill, console box hiding bottom rows
  (diagnosed: paragraph re-wrap), carved motto decoration. Rule added:
  "if there's not enough room, tactically shorten the motto word for word."

---

## Stage 1 — Mercury gauge bottom-up (bug)

`layout.rs` gauge loop lights `row < full_rows` — row 0 is the TOP, so the
column grows downward. Inverted vs the approved design.

Fix: extract pure `gauge_row_bits(row, h, fill) -> u32`:
- bottom `floor(fill)` rows full (0x47)
- partial edge cell just ABOVE the filled block, dots bottom-up
  ([0x00, 0x40, 0x44, 0x46, 0x47] — dot7 first)
- fill clamped to [0, h]; fill = h → all full, no edge cell

Tests: fill 0 → all dark; h=10 fill=3 → rows 7-9 full only; fill 3.5 → row
6 partial 0x44; fill 0.25 → row 9 partial 0x40; fill 10 → all full.

## Stage 2 — Console row-clip fix

Mechanism (diagnosed): chat Paragraph wraps with `.wrap(Wrap{trim:false})`;
lines that bypass chat_lines' wrappers (raw code lines in `code_event`,
unbounded `flush_table_row` joins, blockquote lines that get their `│ `
prefix added AFTER wrapping) exceed the column → paragraph re-wraps →
rows push down → bottom rows clipped (ratatui Paragraph never scrolls).

Fix, two layers:
1. Remove `.wrap` from the chat Paragraph — pre-wrapped lines; future
   overlong lines clip ONE CHAR horizontally, never hide rows.
2. Width budgets at the sources:
   - `code_event`: truncate each code line to `self.width` (… marker)
   - `flush_table_row`: fit the joined row to `self.width` (… marker)
   - `flush()`: when `quote > 0`, wrap to `width − 2` BEFORE the prefix

Tests: 300-char code line at width 40 → every line ≤ 40; long blockquote
line → every line ≤ width after prefix.

## Stage 3 — Carved motto (owner: the FULL string)

`VISITA INTERIOREM TERRAE RECTIFICANDO INVENIES OCCULTUM LAPIDEM` (64
chars, the complete acrostic) in the carved ink (WATERMARK + DIM).

Tactical shortening (owner rule): whole words drop from the TAIL as width
shrinks — pure `motto_for(width) -> Option<String>`, greedy prefix fit:
≥64 full … ≥38 drops LAPIDEM+OCCULTUM+INVENIES … ≥17 `VISITA INTERIOREM`
… ≥6 `VISITA` … below, None.

Placements:
- Gap row (between transcript and editor, left edge): motto_for(main
  column width); skipped on a fresh screen (entries empty — the stone's
  own motto owns that moment); rendered after the fire so text keeps
  priority if flames rise through the gap.
- Editor placeholder: when `input` is empty, the prompt line shows the
  motto in carved ink (cursor still blinks at position 0); width budget =
  box inner − prompt prefix.

Tests: motto_for ladder (full / drops word-for-word / None).

## Commits

| # | Commit | Notes |
|---|---|---|
| 1 | `7497fd0` gauge fills bottom-up + transcript rows can't hide | gauge_row_bits + paragraph unwrap + 3 source budgets |
| 2 | `10c93ea` carved VISITA INTERIOREM… motto — gap row + prompt placeholder | motto_for ladder |

60 cargo tests green. Bin installed both stages.

## Outcome notes

- Gauge row arithmetic: edge cell detection is `h − full − row == 1`
  (row just above the block); `full == h` lights everything with no
  edge artifact — boundary sweep tested.
- Sidebar paragraph KEEPS its wrap (sidebar lines are truncate()d to
  CONTENT_W already; the clip bug was chat-only).
- Motto ladder test sweeps widths 0..=70 asserting every returned form
  fits its budget — the shortening rule is enforced, not assumed.
