# Fire rows: the two bottom rows are the same width (rn = 0 exemption)

**Date:** 2026-11-17 18:25
**Mode:** PLAN (research only — nothing changed)
**Owner report:** "the bottom two fire rows are the same width instead of tapering off."

## 1. The defect, in one line

`officina/tui/src/fire.rs:374`

```rust
let rn = ((rows - 1 - r) as f64) / (rows.saturating_sub(1).max(1) as f64);
let outside_taper = rn > 0.0 && !inside_tongue(edge, rn, taper);
```

`r` runs `0..rows` with **`r = rows-1` as the bottom body row** (the row that sits directly
on the rule). For that row `rn == 0`, and the gate `rn > 0.0` **exempts it from the taper**.
The rule row itself is drawn by `line_cells()`, which has no taper gate at all (it claims every
column with `column_light > 0` because the base must be unbroken).

⇒ **the rule row and the first body row are both full-envelope width.** That is exactly the
two rows the owner is looking at. `taper_scale(0, 0.55) == 1.0` by construction.

The trajectory of this bug is worth recording: the 2026-11-16 fix introduced `sqrt(rn)` because
"the rule row and the row above it are the same width" — and then exempted `rn == 0`, which
re-created the same symptom one row lower. The regression test was *written around* the defect:

```rust
// v.len()-2 is the body's bottom row, which is the taper's EXEMPT base
// (rn = 0) — the assert must look one row higher, at the first tapered row.
let above = |v: &[usize]| v[v.len() - 3];
```

It asserts the base row is *unchanged* by the taper (`assert_eq!(flat.last(), tapered.last())`)
and then skips a row to find a taper at all. So the suite is green while the bottom two rows
stay identical. **A test that encodes the bug is worse than no test**: it makes the defect look
intentional.

## 2. Why "just remove the exemption" is not the whole fix

With `rn = (rows-1-r)/(rows-1)` and the exemption removed, row `rows-1` would still be
`rn == 0` ⇒ `taper_scale == 1.0` ⇒ full width. The *normalisation itself* is wrong: it maps the
bottom body row to 0 instead of to "half a row above the rule".

Correct normalisation uses the same `from_base` the rest of the function already computes:

```rust
let frac_from_base = from_base / rows as f64;   // 0.5/rows at the bottom row
let rn = frac_from_base.clamp(0.0, 1.0);        // strictly > 0 for every row
```

With `rows = 6` that gives `rn = 0.083` for the first body row ⇒ `sqrt(rn) = 0.289` ⇒
`taper_scale = 0.841`. On a 120-column screen that is a ~6-cell inset per side for the first row
— visible, but only just, and it *shrinks as the strip gets taller* (the owner's real strips are
2–6 rows, and at 6 rows the first step is the smallest one).

So the second half of the fix is a **base inset** that guarantees a visible first step
regardless of row count, while staying monotone and continuous:

```rust
const TAPER_BASE_INSET: f64 = 0.35;   // the first row is already ~0.35·taper narrower

fn taper_scale(rn: f64, taper: f64) -> f64 {
    let r = rn.clamp(0.0, 1.0);
    1.0 - taper.clamp(0.0, 1.0) * (TAPER_BASE_INSET + (1.0 - TAPER_BASE_INSET) * r.sqrt())
}
```

- at the first body row (`rn = 0.083`): `1 - 0.55·(0.35 + 0.65·0.289) = 0.704` → ~12 cells narrower
  per side at `w = 120` — unmistakable;
- at the tip (`rn → 1`): `1 - taper` — same as today, so the outline is unchanged at the top;
- monotone, no step, and `taper = 0` still means rectangle (differential property preserved).

## 3. Invariants

- **I1 (the two bottom rows differ):** the first body row is strictly narrower than the rule row,
  for every row count 2…6 and every level ≥ `MIN_LEVEL`. Pinned by a new test that sweeps levels.
- **I2 (rule stays unbroken):** the rule row remains full-envelope width at/above
  `BASE_CONTINUOUS_LEVEL` (the existing behaviour must not be "fixed" into a gap).
- **I3 (monotone taper):** widths are non-increasing going up from the base, and strictly
  decreasing in the bottom third (so a 4-row strip still shows three distinct widths).
- **I4 (taper = 0 is a rectangle):** unchanged geometry with the knob at 0 — the differential
  control must keep failing if the feature is removed.
- **I5 (heat contract):** `rule_heat` stays strictly above the base row's density factor
  (`0.3 + 0.5·level`) so the rule is never a copy of the row above; changing `rn` moves the base
  row's *heat*, so re-check that inequality numerically.
- **I6 (no flicker regression):** the tint mask (`tint_rows`) follows the new outline; the
  row count still goes through `rows_for_stable` (dead band) so the strip cannot flap.

## 4. Test rewrite (the important part)

Replace `flames_taper_with_height`'s "look one row higher" logic with the property it should
have had:

1. **Strict first step:** `width(rule) > width(first body row)` — with the taper on.
2. **Differential:** the same two widths are equal when `taper = 0` (proves the assertion reads
   the taper, not the jitter).
3. **Level sweep:** for levels that yield 2, 3, 4, 5, 6 rows, every one satisfies 1–2.
4. **Fairness gate:** the frame budget guard (dropped first-row assertions). Covered by the
   budget test already in place.

Negative control: revert the `rn` line to `(rows-1-r)/(rows-1)` and confirm the new test fails —
if it passes, the test is again measuring the wrong row.

## 5. Wider issue the owner flagged: "several UI bugs lingering"

Both bugs found in the last hour share one shape: **a renderer whose test was written to match
the current output rather than the intended property**, so it can never fail.

- fire base taper (this plan) — the test skips the row that is broken;
- the composer-prediction widget rendering in the band between ctx and route, mistaken for the
  prompt (2026-11-17 INQUIRE turn) — no test asserts "band rows contain no operator input".

Proposal, as its own small stage after this fix: a `render-invariants` test module that asserts
*semantic* properties of the composed screen — band rows never contain the composer's text, the
ctx row never moves, the two bottom fire rows differ, the notice strip is not on the hints row.
Each with a negative control (flip the offending condition behind a `cfg(test)` knob and watch
it fail). That is the cheapest way to stop this class from recurring.

## 6. Stages

1. `fire.rs`: `rn` normalisation + `TAPER_BASE_INSET`; keep `line_cells` untapered.
2. Rewrite the taper test (strict first step, differential, level sweep); prove the negative
   control fails on the old code.
3. Manual verification on the live TUI at 2/4/6 rows and at `taper` 0 / 0.55 / 1.0 (`/fire taper`),
   plus a screenshot-level check that the rule stays unbroken.
4. Then the `render-invariants` module (batch 2), starting with the composer-prediction band row.

## 7. Files

- `officina/tui/src/fire.rs` — `taper_scale`, `rn` in `render`, `TAPER_BASE_INSET`
- `officina/tui/src/tui/layout.rs` — `flames_taper_with_height` (test), `screen()` helper
- later: `officina/tui/src/tui/tests.rs` (new `render_invariants` module)
