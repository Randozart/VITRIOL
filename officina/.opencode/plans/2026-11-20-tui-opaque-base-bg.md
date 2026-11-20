# TUI opaque base background (kitty transparency fix)

2026-11-20 · pipeline plan · owner-approved (option 3)

## Problem

kitty runs with `background_opacity 0.5` + `background_blur 1`. Kitty's alpha
applies **only to cells whose background is the terminal default**
(`Color::Reset` in ratatui terms); cells with an explicit bg render opaque.
The officina TUI leaves gap cells at `Reset` (ratatui `Clear` resets cells and
unpainted cells start as `Reset`). Aethership's `bg` (#1f1f28) now **equals**
kitty's configured default background, so the TUI's "background" is literally
the frosted desktop: gap cells go translucent, painted cells (panel/user_bg)
stay opaque — a patchy mix. Before the kitty include-chain fix the terminal
default was opaque stock black, so this never surfaced.

## Chosen design (owner-approved)

Two-part TUI-side fix; no kitty config change, no palette.json change:

1. **Off-by-one base background.** `Theme::base_bg()` derives `Rgb(r, g, b ^ 1)`
   from `bg` at runtime (blue channel ±1/255 ≈ imperceptible). Because the TUI
   never emits a cell whose bg equals the terminal default, kitty's
   background_opacity can never apply to any TUI cell.
2. **Paint-first + Clear-safe.** `BaseFill` (new widget) patches `bg` on every
   cell of the frame area — *without* touching symbols or fg — and is rendered
   before everything else. `ClearBase` (new widget) replaces the 14 ratatui
   `Clear` call sites: it performs the same `cell.reset()` blanking as `Clear`
   but lands the cell on `BASE_BG()` instead of `Reset`, preserving the
   blank-then-fill contract every existing call site documents.

Derived (not stored) keeps `theme/palette.json` — the declared single source
of truth — untouched, and cannot drift.

## Invariants

1. **No default-bg cells ever rendered.** After any `render()` pass, every
   cell in the frame buffer has `bg != Color::Reset`.
   Pinned by `rendered_frame_has_no_default_bg_cells` (layout.rs tests,
   TestBackend, multiple screens) — INV-1.
2. **Base is visually identical to bg.** For every theme in `ALL`,
   `base_bg()` differs from `bg` by exactly one bit and from `Color::Reset`.
   Pinned by `base_bg_is_off_by_one_bit` (theme.rs tests) — INV-2.
3. **Single source of truth untouched.** `palette.json` carries no base field;
   the generator emits only palette values. No new test — the existing
   `gen-desktop-theme.test.mjs` fidelity gate plus an absence check
   `base_bg_is_derived_not_stored` — INV-3.
4. **Clear semantics preserved.** Where `Clear` was used, the cell must end
   blank (`reset()` equivalent: default symbol, no modifier) but with base bg,
   never Reset. Pinned by `clear_base_blanks_cell_onto_base_bg` (base.rs
   unit test) — INV-4.
5. **BaseFill never clobbers content.** Painting `BaseFill` over a buffer must
   not change any symbol or fg. Pinned by `base_fill_preserves_symbols` — INV-5.
6. **All themes stay readable.** Existing gates (contrast, role completeness)
   must keep passing; `base_bg` is an accessor, not a role —
   `every_theme_defines_every_role` continues to guard roles — INV-6.

## Named tests

| Test | File | Proves |
|------|------|--------|
| `base_bg_is_off_by_one_bit` | `tui/src/theme.rs` | INV-2 |
| `base_bg_is_derived_not_stored` | `tui/src/theme.rs` | INV-3 |
| `base_fill_preserves_symbols` | `tui/src/tui/base.rs` | INV-5 |
| `clear_base_blanks_cell_onto_base_bg` | `tui/src/tui/base.rs` | INV-4 |
| `rendered_frame_has_no_default_bg_cells` | `tui/src/tui/layout.rs` | INV-1 |
| (existing) `every_theme_defines_every_role`, contrast gates | `tui/src/theme.rs` | INV-6 |

## Stages

- **S1** theme.rs: `pub fn base_bg(&self)` on `Theme` + `BASE_BG()` accessor
  (`#[allow(non_snake_case)]`, matching the table style). Tests INV-2/INV-3.
- **S2** tui/src/tui/base.rs: `BaseFill`, `ClearBase` widgets (INV-4/INV-5
  tests inline); `mod base;` declared in `tui/mod.rs`.
- **S3** layout.rs `render()`: render `BaseFill` first; swap the 13 `Clear`
  sites → `ClearBase`; settings.rs:252 likewise. Drop unused `Clear` imports.
- **S4** layout.rs test `rendered_frame_has_no_default_bg_cells` (INV-1),
  driven through the existing TestBackend harness.
- **VERIFY** `cargo test` in `tui/` (full suite incl. theme_suite), then
  `praetor validate --warn`. One commit per stage.

## Explicitly out of scope

- kitty config (stays frosted), palette.json, generator templates,
  vitriol-tui (separate tree), per-theme base overrides.
