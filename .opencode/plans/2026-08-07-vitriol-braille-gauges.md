# Braille-dot gradient gauges

Date: 2026-08-07.

## 1. Goal

Replace ratatui `Gauge` bars in the TUI with braille-dot gauges. Each cell is
one 6-dot braille glyph (2 cols × 3 rows), each dot is one "percentage point"
(six per block). A gauge fills from the bottom-left dot upward, block by block,
left to right, with a per-gauge semantic neon color gradient. No per-cell warn
tint — the warn signal moves to the value text (bold + orange) where a ramp
does not already signal danger.

## 2. Design

### 2.1 Braille geometry

Unicode Braille block U+2800..U+28FF; glyph = `0x2800 | mask`. 6-dot mask bits:

- dot1 = bit0 (top-left), dot2 = bit1 (mid-left), dot3 = bit2 (bottom-left)
- dot4 = bit3 (top-right), dot5 = bit4 (mid-right), dot6 = bit5 (bottom-right)

Fill order within a block, bottom-left first, rising:

1. dot3 `0x04` → 2. dot6 `0x20` → 3. dot2 `0x02` → 4. dot5 `0x10` →
   5. dot1 `0x01` → 6. dot4 `0x08`

### 2.2 Layout

- Adaptive width: `cells = area.width`, total dots `= cells × 6`.
- `filled = round(ratio × cells × 6)` clamped to `[0, cells×6]`.
- Cell `i` lit count `= clamp(filled − i×6, 0, 6)`; mask = OR of the first `lit`
  fill-order bits.
- Empty cell (`mask == 0`) renders `U+2800` (blank in most fonts) styled muted.
- Cell color = `ramp_color(i / (cells−1))`, left = ramp start, right = ramp end.

### 2.3 Per-gauge ramps (tech alchemy, neon)

| Gauge | Ramp | Stops | Metaphor |
|---|---|---|---|
| VRAM | `Capacity` | `#FFFFFF` → `#FFE066` → `#FF5F1F` → `#FF4444` → `#8A1515` | white/light-yellow at empty → space running out (orange → red → deep red) |
| UTIL | `Activity` | `#0B5E4C` → `#39FF14` → `#00FFFF` | solvent flowing; cyan at peak |
| TEMP | `Heat` | `#2E5FA3` → `#00FFFF` → `#FFD700` → `#FF5F1F` → `#FF4444` | thermal: cold blue/cyan → white-hot gold → orange → red |
| SM CLK | `Pulse` | `#55606E` → `#00FFFF` | quicksilver pulse |
| MEM CLK | `Pulse` | `#55606E` → `#00FFFF` | shared ramp |
| POWER | `Power` | `#39FF14` → `#FFD700` → `#FF4444` | living fire; over-limit red |

`ramp_color` = piecewise lerp across stops by `t ∈ [0,1]`. Multi-stop ramps
lerp between adjacent stops.

`signals_danger()` = ramp end is red/deep-red (`Capacity`, `Heat`, `Power`) —
the ramp itself signals the top; no warn text needed.

### 2.4 Warn signal (value text)

Only ramps whose end does NOT signal danger get warn text: `Activity`,
`Pulse`. When `ratio > 0.8` their value text becomes `ORANGE` + `BOLD`.
`Capacity`/`Heat`/`Power` ramps already end red; text stays normal. In
`render_gauge_row` (Dashboard) the label carries the percentage, so the label
takes the warn style.

## 3. Modules

- `vitriol-tui/src/braille.rs` (new): `FILL_ORDER` bits, `cell_mask(lit)`,
  `glyph(mask)`, `bar(ratio, cells)` → `Vec<BarCell>` (glyph + cell t + lit).
  First-class module, not inline in the render loop (AGENTS §3.10).
- `theme.rs`: `BrailleRamp` enum (5 variants) with `stops()`,
  `signals_danger()`, `color(t)`; `ramp_color(stops, t)` lerp; new color consts;
  `gauge_value_style(ramp, ratio)`.
- `ui.rs`: `render_braille_bar(frame, area, ratio, ramp)`; both
  `render_gauge_row` and `render_metric_row` migrate (DRY, AGENTS §3.9); drop
  `Gauge` import.

## 4. Baseline

No perf impact expected (bar width ≤ terminal width, one `Line` of `Span`s).
Visual-only change; gate is test + clippy + fmt + praetor.

## 5. Risks

- Braille glyphs depend on terminal font rendering U+2800. Verify on a real
  TTY; the non-interactive shell cannot preview.
- Empty `U+2800` renders blank in most fonts; styled muted if a font draws
  faint dots.

## 6. Results

(fill after implementation)
