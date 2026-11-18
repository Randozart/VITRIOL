# Fire: kill the stair-step at the tongue's edge (glyph falloff + dither)

**Date:** 2026-11-18 15:05
**Mode:** PLAN (research only — nothing changed)
**Owner:** "It's the stair stepping" (confirmed after the rn/BASE_INSET fix; the edge fix was
planned on 2026-11-17 and never built).

## 1. Why the edge steps

`officina/tui/src/fire.rs`:

```rust
const DENSITY: [char; 8] = ['⠀','⡀','⡄','⡆','⡇','⣇','⣧','⣷'];   // dot-count ramp
...
let idx  = (dens * (DENSITY.len() - 1) as f64).round() as usize;
let ch   = DENSITY[idx.min(DENSITY.len() - 1)];
```

and the taper gate above it:

```rust
let outside_taper = rn > 0.0 && !inside_tongue(edge, rn, taper);
if outside_taper || frac_from_base > col_h { /* … */ spans.push(Span::raw(" ")); continue; }
```

Three properties combine into a staircase:

1. **The gate is binary.** A column is inside the tongue or it is a space. There is no cell between
   "full braille" and "nothing".
2. **Density does not depend on distance from the edge.** `dens` is a function of height
   (`frac_from_base`) and the per-cell hash only — so the boundary column is as dense as the centre
   column, and then the *next* column is blank. That one-cell cliff is the stair.
3. **The boundary is deterministic per row.** `inside_tongue` uses only `rn`, so successive rows step
   at the same phase; the eye reads a diagonal of aligned right angles.

(Note also that `DENSITY[0]` is `⠀` — blank braille, not a space — so the ramp's bottom is
indistinguishable from a culled cell. The contrast has to come from *where* the ramp lands, not from
its lowest glyph.)

## 2. Fix: fade the glyph index toward the boundary, dither only the outermost sliver

In `render`, after `dens` is computed and before the glyph index is taken:

```rust
// Distance to the tongue's boundary, 0 at the midline → 1 at the cut.
let half = 0.5 * taper_scale(rn, taper);
let t = ((edge - TONGUE_CENTER).abs() / half.max(1e-6)).clamp(0.0, 1.0);
dens *= edge_falloff(t);              // smooth: 1.0 inside, ~0.15 at the cut
```

```rust
/// Glyph density multiplier by distance from the tongue's boundary.
/// 1.0 across the body, falling over the outer ~20 % so the edge ends in single
/// dots instead of a full bar followed by nothing.
fn edge_falloff(t: f64) -> f64 {
    const KNEE: f64 = 0.78;           // where the fade starts
    if t <= KNEE { return 1.0; }
    let x = ((t - KNEE) / (1.0 - KNEE)).clamp(0.0, 1.0);
    1.0 - 0.85 * x * x                // quadratic shoulder: gentle, then decisive
}
```

and the dither, applied **only** where the fade has already thinned the glyph:

```rust
// The last few percent of the tongue draws on some flicker steps only, so the
// boundary column alternates with its neighbour instead of holding a hard edge.
if t > 0.88 && hash01(x as u32, 913, step) < (t - 0.88) / 0.12 { /* cull this step */ }
```

Why this shape:

- **Falloff, not geometry.** Cells cannot be split; the only way to make a diagonal in a character
  grid is to drop the glyph's *weight* as the boundary approaches.
- **Quadratic shoulder.** A linear fade puts most of its change at the cut, which just moves the
  stair one cell in; the quadratic spends the change where the eye is.
- **Dither only above 0.88.** Dithering a fully dense cell makes the edge *sparkle* rather than
  fade; dithering an already-thin cell breaks the phase alignment that creates the stair.
- **`TONGUE_CENTER` unchanged (0.55)** — the envelope's asymmetry is deliberate (the flames lean
  away from the dead middle band).

### Must NOT dither: the tint mask

`tint_rows()` paints text backgrounds from the same silhouette. Text must never flicker (the owner
reported exactly that on 2026-11-16), so the mask takes the **smooth falloff** (heat × `edge_falloff`)
and **not** the dither. Two consumers, one shape, two temporal behaviours — state it in a comment.

### Keep the rule unbroken

`line_cells()` (the underscore row) has no taper gate and must keep none: the base is the flame's
foundation, and a dither there would read as a broken rule.

## 3. Tests

1. **`edge_falloff` (pure):** `1.0` for `t ≤ 0.78`; `< 1.0` above; monotonically non-increasing;
   `≥ 0.1` at `t = 1` (the edge still draws *something*).
2. **Edge is lighter than the centre** — the assertion that would have failed before this change:
   render at full load, take the bottom two body rows, compare the glyph *index in `DENSITY`* of the
   outermost lit column against the centre column; the edge must be strictly lighter (or blank on
   that frame, which the dither makes legal).
3. **The stair is broken (phase test):** render three consecutive flicker steps at the same level;
   the outermost lit column index must differ in at least one step — i.e. the boundary moves.
   Negative control: with the dither disabled (`t > 0.88` guard removed) all three frames agree, so
   the test fails.
4. **Tint does not flicker:** the mask (`tint_rows`) for the same three steps is identical — pinned
   separately, because this is the regression the owner already reported once.
5. **Existing invariants stay green:** body ⊆ rule, rule unbroken at full load, widths monotone going
   up from the base, single activity line.

## 4. Files & risk

- `officina/tui/src/fire.rs` — `edge_falloff`, the fade in `render`, the dither guard, the mask fade
  in `tint_rows`, and the new tests.
- Risk: the fire is decoration, so the blast radius is visual. The two real hazards are (a) making
  the flames *too* sparse at low load — the fade multiplies an already-small `dens`, so verify at
  `level = 0.1` that the body is still visible; and (b) tint bleeding outside the glyphs — the mask
  fade keeps them together.
- Not in scope: the pedestal question (rule wider than the first body row). That is a separate
  decision about `BASE_INSET`, and the falloff may make it moot.
