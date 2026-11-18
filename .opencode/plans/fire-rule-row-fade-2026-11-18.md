# Fire: the rule row still ends in a hard shelf

**Date:** 2026-11-18 15:40
**Mode:** PLAN (research only — nothing changed)
**Owner:** "It now looks correctly near the bottom, but not quite yet on the rule row."

## 1. Why the rule row is different from the rows above it

The body rows and the rule row are drawn by two different functions:

| row | function | edge behaviour |
|---|---|---|
| body rows | `render()` | taper window + `edge_falloff` (added today) + dither in the outer sliver |
| the rule | `line_cells()` | **no taper gate at all**, and no falloff |

`line_cells()` claims every column with `col_light > 0`:

```rust
let col_light = column_light(column_edge(x, width), level);
if col_light <= 0.0 { continue; }
...
let dens = base_dens * rule_heat(level);      // ← no edge term
let idx = (dens * (DENSITY.len() - 1) as f64).round() as usize;
*slot = Some(DENSITY[idx.clamp(1, DENSITY.len() - 1)]);
```

`column_light(edge, level) = edge * level * 1.15` and `column_edge` is **distance from the centre**
(0 mid-screen → 1 at the sides). So at working load the rule is claimed across the *whole width*
with **uniform** density, and stops only because the frame stops. Above it, the body fades inward
(today's fix). The result is a flat, uniformly heavy band whose ends are hard right angles — the
shelf the owner still sees, one row lower than the fixed one.

Two separate defects live in that:

1. **No density falloff on the rule**, so its ends cannot soften.
2. **The rule is wider than the flame above it.** The first body row is inset by `BASE_INSET`
   (18 %), and `inside_tongue` then culls its outer columns, so the rule overhangs its own fire.

## 2. Two ways to fix it

### A. Thin the rule (recommended)
Multiply the rule's density by the same `edge_falloff`, measured against the **base window**
(`taper_scale(rn = 0, taper)`, i.e. the widest the tongue ever is):

```rust
let half = 0.5 * taper_scale(0.0, taper);
let t = ((edge - TONGUE_CENTER).abs() / half.max(1e-6)).clamp(0.0, 1.0);
let dens = base_dens * rule_heat(level) * edge_falloff(t);
```

The rule then runs from dense at the tongue's centre to single dots (`⡀`) at its ends — still
**continuous** (no gaps: the owner's 2026-11-16 invariant), just lighter. The overhang disappears
visually because a one-dot column next to nothing is not a shelf.

No dither on the rule: dithering a line reads as gaps, and "the rule is unbroken at working load" is
an invariant with a test behind it.

### B. Match the rule to the flame's width
Mask the rule with the window the first body row uses (`inside_tongue(edge, rn_first, taper)`), so the
base ends exactly where the fire above it ends.

Cleaner geometry, but it *narrows* the base — the opposite of the 2026-11-16 decision that the rule is
the flame's foundation and must reach as wide as the fire is born. It also interacts with
`BASE_INSET`: with both, the rule would shrink to `1 − 0.18·taper` of its current width, and the
"spawns on the rule" look gets thin.

**Recommendation: A**, optionally followed by lowering `BASE_INSET` if the pedestal still reads.

## 3. Invariants that must survive

- **I1 (unbroken base):** at/above `BASE_CONTINUOUS_LEVEL`, every claimed column draws a glyph — the
  falloff changes *which* glyph, never *whether*. Pinned by the existing continuity test, extended to
  compare glyph presence at the outermost claimed column.
- **I2 (rule hotter than the row above):** in the body (`t` small, falloff ≈ 1) `rule_heat` must stay
  strictly above the base-row factor `0.3 + 0.5·level`. Pinned by a direct comparison of the two
  densities at the same column.
- **I3 (soft ends):** the outermost claimed column's glyph index is strictly lighter than the tongue
  centre's — the mirror of today's `fire_edge_glyphs_fade_instead_of_stepping`, applied to
  `line_cells`.
- **I4 (no dithered line):** the rule's glyph set is identical across flicker steps at the same level
  (a line that blinks is a gap). Pinned by comparing `line_cells` at three phases.

## 4. Test plan for `line_cells`

1. `rule_fades_toward_its_ends`: at level 1.0, the density index at the outermost claimed column is
   `<` the index at the tongue centre (and `>= 1`, i.e. still a dot).
2. `rule_stays_unbroken_at_working_load`: at level ≥ 0.25 every column inside the base window has
   `Some(_)` — the existing invariant, restated for the fade.
3. `rule_does_not_flicker`: three phases → byte-identical vectors.
4. Negative control for (1): with the `edge_falloff` term removed, edge index == centre index.
5. Regression: `flame_meets_its_line` (body ⊆ rule) and `flames_taper_with_height` stay green.

## 5. Files

- `officina/tui/src/fire.rs` — one line in `line_cells`, plus five tests.
- Risk: low and local. The only visible risk is thinning the rule too much at low load, where
   `rule_heat` is already small; check level 0.3 and 0.5 by eye before calling it done.
