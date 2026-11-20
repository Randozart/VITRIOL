# Fire fix — inward gradient not visible + bottom band single-colour (2026-11-20, PLAN mode)

## Root cause (traced, quantified)

The S5 edit added `cell_heat(base_heat, frac, edge_t)` but the BODY render
loop calls it with **`edge_t` hardcoded to `0.0`** (fire.rs:560):

```rust
let heat = cell_heat(base_heat, frac, 0.0);   // <-- BUG: edge term dead
```

The `edge`/`edge_t` values are only computed *later* in the loop (edge
softening at :567) and only fed into the text-tint mask (fire.rs:640), not the
drawn flame. Quantified (w=100): edge ranges 0.0 (centre) → 1.0 (screen edge),
so `inward = 1 − edge_t·0.55` would spread heat 1.0→0.45 across the width —
but with the hardcoded 0 every column gets identical heat. That is BOTH
complaints:

1. **"Not inside-out"** — the inward term is a no-op in the drawn flame; only
   the text tint (which IS wired) shows it, and that's subtle.
2. **"Bottom layers same color"** — the rule row (layout.rs:806) uses
   `ignition_color` = `style.color(base_heat)` with frac=0, edge=0 — the exact
   same input as the body's bottom row (frac clamps to 0 at the base). Rule and
   first body row are again pixel-identical in colour.

## Fix (BUILD)

1. **fire.rs `render`**: hoist the `half`/`edge_t` computation ABOVE the heat
   call and pass the real value: `cell_heat(base_heat, frac, edge_t)`. The
   edge-softening block keeps using the same binding (no double compute).
2. **fire.rs `ignition_color`** → replace with **`rule_column_color(x, width,
   phase_ms, level, style)`**: per-column heat via `cell_heat(base_heat, 0.0,
   edge_t)` — the rule inherits the same inward gradient as the body (owner:
   "the rule is where the fire is born" — it must participate in the gradient,
   not sit outside it).
3. **layout.rs:806**: loop the rule row per column instead of one colour for
   all (`ignition_spans` already takes per-column `lit` cells; extend it to
   take the colour fn or compute colours inline per column).
4. **Mask**: already correct (uses real edge_t) — untouched.
5. **Tests**: fire.rs — new pin: two columns at same frac, different edge_t
   give different colours through `render`'s heat path (behavioural, via
   `rule_column_color` + `cell_heat`); update the `cell_heat(…, 0.0)`-era test
   if any assert the dead value.
6. **Ratchet**: fire.rs +~15, layout.rs +~10 — raise in-commit with reason.

## Invariants
- I-A: flame shape/density/flicker untouched — colour mapping only.
- I-B: rule stays hotter than the row above (`rule_heat` ≥ base taper) —
  existing test `the_rule_is_still_hotter_than_the_row_above` must pass
  unchanged (it tests rule_heat fn, not colour — safe).
- I-C: text tint continues using the smooth (non-dithering) falloff.

Ready for BUILD.
