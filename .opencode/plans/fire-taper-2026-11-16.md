# Fire silhouette: taper the flames, not just their density (2026-11-16, PLAN)

Owner: *"the rule row and the row above that for the fire are currently the same
width, rather than tapering off."*

## Why it happens (traced)

The flame's **horizontal extent** comes from one place — the column envelope
`column_edge(x)`, which is a function of `x` only:

```rust
let edge = column_edge(x, w);              // fire.rs:331
let col_light = column_light(edge, level); // edge * level * 1.15
let col_h = (col_light * (0.7 + 0.6 * hash01(x, 0, step))).max(floor);
if frac_from_base > col_h { /* not lit */ }  // the only vertical test
```

So a column is lit up to some height determined by `col_h`, and `col_h` has no
*row* term: it is `level × edge × jitter`. Two consequences:

1. **The base row is clamped to `floor = base_frac` for every column with
   `col_light > 0`** — so at/above `BASE_CONTINUOUS_LEVEL` the rule is lit across
   the *entire* envelope (that is the previous fix, working as asked).
2. The row above is culled only where `col_h` happens to be below `base + 1/rows`.
   At high load `col_light ≈ 1.15·edge` is near 1 across the whole tongue, and the
   jitter floor is 0.7, so **almost no column is culled** → the row above is the
   same width as the rule. Density varies with height; **width does not**.

The tint mask (`tint_rows`) uses the same `column_light`, so it is equally
rectangular — the recolouring follows the (non-)taper.

## The fix: make the envelope height-dependent

Introduce a per-row squeeze of the *tongue* (the fire already lives in two
tongues at the screen edges, with a dead middle band):

```rust
/// How far the envelope narrows by the top of the strip (0 = today).
const TAPER: f64 = 0.55;
/// Mid-point of a tongue, in `edge` space (edge = 0 at the inner boundary,
/// 1 at the screen edge).
const TONGUE_CENTER: f64 = 0.55;

/// Envelope width kept at height fraction `rn` (0 base → 1 tip).
fn taper_scale(rn: f64) -> f64 {
    1.0 - TAPER * rn.clamp(0.0, 1.0)
}

/// Is this column inside the tapered tongue at this height?
fn inside_tongue(edge: f64, rn: f64) -> bool {
    (edge - TONGUE_CENTER).abs() <= 0.5 * taper_scale(rn)
}
```

`render` culls with `!inside_tongue(edge, r_from_base/rows)` in addition to the
existing density test; `tint_rows` uses the same predicate so the text tint stops
at the visible flame edge.

Why squeeze about `TONGUE_CENTER` rather than dropping an end: the tongue spans
`edge ∈ [0, 1]` between the dead middle and the screen edge. Dropping the *inner*
end detaches the flame from the centre; dropping the *outer* end makes it lean
inward and leaves the screen edge dark. Shrinking a window around the tongue's
midline is what a real flame does — it narrows toward its own tip.

## What stays true

- **Base continuity** (the last fix): `rn = 0` ⇒ `taper_scale = 1` ⇒ the base row
  keeps the full envelope. The rule is still unbroken.
- **Density/heat**: unchanged. The rule stays the densest row, the body thins
  upward as it does now.
- **Middle band**: `edge = 0` is outside the window (`|0 − 0.55| = 0.55 > 0.5`) ⇒
  the dead band stays dead at the base too. **Careful**: that is a behaviour
  change at the base unless the base uses the untapered predicate. Solution: the
  base row (`rn < 1/rows`) keeps *today's* rule (`col_light > 0`), and the taper
  applies from the second row up. Stated as an invariant below.

## Invariants / tests

- **F-C1** at `level ≥ 0.6`, the lit width of each row strictly decreases from
  the row above the base to the top (measured as "columns lit at this row"),
  with the base row exempt.
- **F-C2** the base row is unchanged by the taper: same width, unbroken, `_` only
  in the dead middle band (the existing `base_is_unbroken_at_working_load` must
  stay green).
- **F-C3** `tint_rows` and `render` agree: a tinted cell is always inside the
  lit silhouette (new test compares the two predicates row by row).
- **F-C4** `/fire off`, `level < MIN_LEVEL`, and the middle band are untouched.

## Trade-offs / knobs

- **TAPER** (0.55): 0 restores today's rectangle; 0.8 gives a sharp cone that the
  outer 3 rows of a 6-row strip barely survive. Tune by dump.
- **TONGUE_CENTER** (0.55): shifting it outward makes the flame hug the screen
  edge; inward makes it lean toward the transcript.
- The two end-drop variants (drop inner / drop outer) are one-line changes to
  `inside_tongue` if the squeeze does not read well; BUILD will dump all three at
  levels 0.35 / 0.7 / 1.0 so the choice is visual, not theoretical.

## Files

- `officina/tui/src/fire.rs` — `TAPER`, `TONGUE_CENTER`, `taper_scale`,
  `inside_tongue`; use in `render` (cull) and `tint_rows` (mask); tests F-C1..C4.
- Nothing else: layout, state, theme untouched.

---

# As landed (2026-11-16, installed `e9fc8144`)

`inside_tongue(edge, rn, taper)` shrinks a window around the tongue's midline
(`TONGUE_CENTER = 0.55`) as height grows; `render` culls with it and `tint_rows`
uses the same predicate so the text tint stops at the visible flame edge. The
**base row is exempt** (`rn == 0`), so the rule keeps the envelope's full width.

## Two corrections found by measuring

1. **Linear taper was invisible where it mattered.** With `1 − taper·rn`, the
   first row above the base lost only a sliver (measured widths
   `[2,18,41,54,62,70,80]` vs `[2,6,21,38,52,70,80]` — row 5 unchanged). The
   ramp now uses `sqrt(rn)`, which bites immediately and spends less on the
   1–2-cell top rows.
2. **My first test could not fail.** It asserted "top row narrower than base",
   which the pre-existing jitter already satisfied: the negative control (taper
   removed) *passed*. It is now **differential** — the same state rendered with
   `fire_taper = 0.0` and `0.55` — and the control fails correctly. Same class of
   mistake as the hints-row test earlier today; measuring beats asserting.

## Dumps (100 cols)

```
level 1.0                       level 0.7
          ⡆                          ⡄⡄ ⡆
       ⡆⡆⡄⡇  ⡄⡀                   ⣧⡇⡇⡇⡇⣇⡆⡄⡆⡆⡆⡇ ⡄
      ⡇⡇⡇⡇⡇⡆⡄⡇⡆⡇⡇⡄⡄ ⡄        ⣇⡆⡇⡇⡇⣇⡇⡇⣇⣇⣇⣇⣇⡇⣇⣇⣇⡆⡆⡆⣇⡇⡇⣇⡇⡇
    ⣇⣷⣇⣇⣧⣧⣷⣧⣇⣇⣧⣧⣧⡆⡆⡄⣇⡆⡆        ⣧⡇⣇⣇⣇⣧⣇⣇⣧⣧⣧⣧⣧⣇⣧⣧⣧⣇⣇⡇⣧⣧⣇⣧⣇⣇
⡇⣧⡇⣧⣇⣧⣇⡇⣧⣧⣇⣧⣧⣧⣧⣧⣧⡇⣇⣧⣇⣧⣧⣧⣧⣧      ⣇⣆⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⣇⡇⣇⣇⣇⣇⡇⣇⡇⡇⣇⡇⡇⣇⡇⣇⣇
⣇⣷⣧⣷⣧⣷⣷⣇⣷⣷⣧⣷⣷⣷⣷⣷⣷⣇⣧⣷⣧⣷⣷⣷⣷⣷______⣧⣧⣷⣧⣷⣇⣷⣷⣧⣷⣷⣷⣷⣧⣷⣧⣧⣧⣷⣷⣧⣷⣷⣷⣧⣷
```

(Bottom row = the unbroken rule; each row above it is narrower than the one
below, and the dead middle band is untouched.)

## Knobs

- `DEFAULT_TAPER = 0.55`, settable live with `/fire taper <0..1>` — `0` restores
  the old rectangle, so the operator can A/B it without a rebuild.
- `TONGUE_CENTER = 0.55` (one const) shifts the squeeze outward/inward.

328 tests, fmt clean, warning-free build.
