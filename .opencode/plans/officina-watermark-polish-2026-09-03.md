# Watermark polish: streak travel fix + braille margin strip

**Date:** 2026-09-03 06:30 UTC
**Context:** Two owner-reported findings after the green streak landed
(b583e27). Both fixes in `officina/tui/src/watermark.rs`.

---

## Fix A — parked streak (the "horizontal streak")

**Diagnosis (measured):** the sweep travels `max_d + SHIMMER_BAND*2` then
parks for the rest phase (55% of the 6s cycle). The green streak trails by
`SHIMMER_STREAK_OFFSET = 10`, so at rest its crest sits at
`max_d − 4 = d_max − 1` (the stone's deepest real diagonal, the bottom-right
corner). Only the bottom ~2 rows reach those d values → the stranded streak
renders as a short **horizontal** green dash hugging the bottom-right
corner for ~3.3s every cycle. Owner saw "another shimmer, horizontal."

**Fix:** extend the travel budget by the streak offset so both bands fully
exit before stillness:

```rust
let pos = p * (max_d + SHIMMER_BAND * 2.0 + SHIMMER_STREAK_OFFSET) - SHIMMER_BAND;
```

Rest positions: silver max_d+16, green max_d+6; nearest live cell is
d_max = max_d−3 → both ≥9 cells clear → zero intensity. Sweep start
unchanged (both bands begin before d=0).

**Refactor for testability:** extract pure helpers `sweep_pos(p, max_d)`,
`streak_pos(sweep)`, `band_i(d, crest, band)` — the Shimmer arm and the
regression test share the same formulas.

**Regression test:** `shimmer_and_streak_fully_exit_before_rest` — across
rest-phase cycle fractions (p 0.5→1.0) both band intensities are 0 at the
deepest live cell; mid-sweep the crest genuinely crosses the stone
(coverage check at p=0.2).

## Fix B — braille margin strip ("whitespace before and after")

**Diagnosis (measured):** NO removable rows exist in either asset. All 32
rows (38 with motto) carry art — the perceived vertical whitespace is the
diamond taper (rows of 6-16 dots ≈ visually near-invisible). The real
padding is HORIZONTAL: ~12 leading + ~12 trailing U+2800 (braille blank,
U+2800) columns per row — invisible to `trim_end()` (not ASCII
whitespace). The motto's left offset is defined by these same leading
columns.

**Fix:** code-side strip in `art_lines` (assets stay pristine):
- Leading: remove the SHARED minimum blank-column run across all rows
  (uniform column removal → every relative offset, including the motto's
  indent against the stone, is mathematically preserved)
- Trailing: per-line blank-run trim (left-alignment unaffected)

Rendered block tightens 80 → ~56 (plain) / ~62 (motto) columns; the
watermark's left_pad centering then centers the visible stone exactly.

**Tests:** `strip_removes_shared_margins_but_keeps_relative_offsets`
(motto indent preserved) + `strip_handles_blank_and_empty_lines`.

## Commits

| # | Message |
|---|---|
| 1 | `officina: shimmer streak travel — exit fully before rest phase` |
| 2 | `officina: strip braille blank margins from watermark art` |
