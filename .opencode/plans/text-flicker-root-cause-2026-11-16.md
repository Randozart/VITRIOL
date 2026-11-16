# Text flicker after the band fix — root cause research (2026-11-16 17:05)

Owner: *"the text now occasionally flickers. I don't suppose this all has a
shared root cause?"* — **Yes, it does.** Both complaints are the same class of
bug: the bottom strip derives a *discrete* visual quantity (row count, tinted
cell set) from a *noisy continuous* signal, and it does so on top of live
transcript text.

## Findings

### The only per-frame writers of text cells

`tui/layout.rs` mutates the frame buffer after the widgets paint in exactly two
places:

| line | what | when |
|---|---|---|
| 778–802 | **fire text-tint pass** — `cell.set_fg(*color)` for every glyph inside the flame strip | whenever `fire_map` is non-empty |
| 812–860 | scroll gauge (right-edge column, `set_symbol`+`set_fg`) | only while scrolled back |

So the flicker is the tint pass. Defaults confirm it affects everyone:
`fire_on: true` (`tui/state.rs:1264`), `fire_tint: true` (`:1266`, and a test
pins it as "the beloved behavior", `:7271`).

### Why it flickers — two mechanisms, both in `fire.rs`

1. **Per-cell mask churn (dominant).** `render()` computes per-column reach as

   ```rust
   let col_h = (edge * level * 1.15).clamp(0.0, 1.0) * (0.7 + 0.6 * hash01(x, 0, step));
   ```
   `step = phase_ms / FLICKER_STEP_MS` with `FLICKER_STEP_MS = 120`
   (`fire.rs:138`), and the idle/streaming redraw cadence is 90 ms
   (`tui/mod.rs:555-557`, `Duration::from_millis(90)` while anything animates). The hash therefore rerolls on most frames and multiplies
   the column height by a uniform factor in **0.7 … 1.3** (±23 %). A flame row
   spans `1/rows ≈ 0.17–0.5` of `frac_from_base`, so the ±23 % jitter is larger
   than one row: cells near the tip flip between `Some(colour)` (tinted) and
   `None` (untinted) **every frame**. The tint pass then sets and un-sets the
   glyph colour at ~11 fps.

2. **Row-count flapping (block-level).** `rows_for(level) = round(2 + 4*level)`
   clamped to `[2, 6]` (`fire.rs:164`). `level` is a low-passed breathing signal
   (`step_fire`, `state.rs:3675`: target `0.35 + 0.55*stream_heat`, `τ = 0.33 s`)
   that keeps wobbling with stream deltas. `d(rows)/d(level) = 4`, so a level
   wobble of 0.125 — routinely crossed — changes the strip height by a whole
   row, which adds/removes the tint on an **entire row of transcript text** as a
   block.

Per-cell colours also drift (`style.color(heat, phase_ms)`, `PULSE_PERIOD_S = 9 s`),
but that is slow and intended; the on/off flips are what read as flicker.

### Shared root cause with the "bumps upwards"

Both symptoms are the same shape:

| symptom | discrete quantity | derived from |
|---|---|---|
| band bumped upwards | status-band row count | ctx fill + belowEditor widget lines (event-driven, spiky) |
| text flickers | flame row count + per-cell tinted set | `fire_level` (breathing) and `hash01(x, 0, step)` (rerolled every 120 ms) |

In both cases a *layout/visibility decision* tracks a signal that is noisy at
frame rate, while the pixels underneath are text the operator is reading. The
band fix made the band's row count settle (700 ms dwell); the flames were never
given the same treatment.

### Why it surfaced "now"

The band fix shortened the strip above the composer for sessions without engine
gauges, and (before it) the band's own row flapping masked the flame flapping by
moving everything at once. With the band still, the flame's 1-row flapping and
per-cell tint churn are the only motion left in that region — it stopped hiding
behind a bigger jitter.

## Confirmation available to the operator (no code change)

1. `/fire tint off` → the tint pass is skipped entirely (`layout.rs:783`). If the
   flicker stops, the diagnosis is confirmed.
2. `/fire off` → no flames at all (also stops it, but loses the fire).

Both are reversible and already implemented; neither touches state.

### One secondary suspect, for honesty

The streaming tail line is *intended* animation: the spinner glyph changes every
120 ms and `working… <n>s` ticks every second (`chat_lines`, appended when
`is_streaming`). That only affects the **last line** of the transcript, while the
fire tint affects the **bottom 1–5 rows at once and can flip individual
characters**, which is the pattern described. If `/fire tint off` does not stop
the flicker, the tail line is the next candidate and the fix there is cosmetic
(slower spinner, no per-second seconds).

## Proposed fix (two small, display-only changes)

### F1 — give the flame height a dead band

Replace the raw rounding with a hysteretic step:

```rust
/// Flame rows for a load level, with a dead band: the strip only changes
/// height once the level has clearly left the current step (±0.08 ≈ half a
/// row), so a breathing level cannot flap the row count.
pub fn rows_for_stable(level: f64, previous: usize) -> usize { … }
```

A pure function; `render()` passes the last row count through `AppState`
(`fire_rows` is already computed in the renderer, so the state carries one
`usize`).

### F2 — tint from the flame's *shape*, not its noise

The flame's look (braille density, ragged tips, embers) must not change. Only
the **tint mask** should be calm:

- return a second, smooth mask from `fire::render` — `Some(colour)` where the
  *unanimated* column envelope covers the cell
  (`edge * level * 1.15 ≥ frac_from_base`, no `hash01` factor, embers excluded);
- the colour comes from the cell's heat with the same `style.color(heat, phase)`,
  so glyphs still sit in the fire's palette and still drift slowly.

Effect: a glyph's tint changes only when the (smooth) column envelope crosses its
row — a deliberate event at the breathing rate, not a per-frame coin flip.

### F3 — regression tests (the part that stops this coming back)

1. `rows_for_stable`: sweeping level up and down by ±0.05 around every step
   boundary must not oscillate; a clear move must change the row count once.
2. **Frame-stability test** (TestBackend): render the same state twice with
   `phase_ms` advanced by `FLICKER_STEP_MS`, and assert the *set* of tinted cell
   positions is identical. Today this fails; after F2 it passes.
3. Keep the golden for the glyph field: with a fixed phase and level, the fire's
   rendered braille rows must be byte-identical before/after F2 (F2 must not
   touch the flames' appearance).

## Invariants

- **I1 — no fire, no tint**: `fire_on == false` or `level < MIN_LEVEL` ⇒ no
  tint, no flame rows; rendering identical to today.
- **I2 — flames unchanged**: F2 changes only the returned tint colours; the
  braille glyphs, envelope, embers and palette ramps stay as they are (pinned by
  a golden comparison at fixed phase/level).
- **I3 — text outside the strip is untouched**: the tint never reaches above the
  flame rect (existing behaviour; pinned by the frame test bounding the tinted
  positions to the strip).
- **I4 — no per-frame tint churn**: at constant `level`, advancing the phase must
  not add or remove a single tinted position (the frame-stability test).
- **I5 — band stability preserved**: no change to `stabilize_band_rows`; the
  band test suite must stay green.

## Trade-offs / alternatives considered

- **Drop the tint entirely.** Simplest and flicker-free, but it throws away an
  owner-requested effect ("text discolors based on the fire beneath it"). Not
  proposed.
- **Dwell the tint mask (hold it N frames).** Cheap, but a held mask lags the
  flames and can look stuck. F2 achieves the same stability without lag.
- **Slow `FLICKER_STEP_MS` (e.g. 120 → 400 ms).** Calms the mask but makes the
  fire itself chunky; the fire should stay lively. F2 isolates the two.
- **Draw the tint *before* the text** (tint the fire's bg, let text inherit). Not
  possible for markdown-styled spans (they carry explicit fg), which is exactly
  why the pass exists.

## Files that would change

- `officina/tui/src/fire.rs` — `rows_for_stable`, smooth tint mask in `render`.
- `officina/tui/src/tui/state.rs` — carry the last flame row count.
- `officina/tui/src/tui/layout.rs` — use the stable row count and the new mask.
- tests in `fire.rs` / `layout.rs` (frame-stability + stability of row count).

Effort: small (≈80 lines incl. tests). Risk: display-only, no state, no
routing, no engine interaction; rollback = revert two commits.

---

# F4 — the fire ignites ON the divider line (owner request 2026-11-16)

Owner: *"can we make sure it starts on the underscore line. If there's no fire
it's all underscores. If there is a little fire, like embers, the underscore
line is (partially or selectively) replaced by a thin braille line the rest of
the fire can expand from."*

## Why this is a natural fit (and what is wrong today)

Geometry per frame (`render_chat_and_editor`):

```
chat_area   [chat_top .. chat_top+h)
divider     y = chat_top + h                 <-- the "_" row (painted AFTER the fire)
band_area   [divider+1 .. divider+1+band_h)
input_area  [band_top+band_h .. +input_h)

fire rect   y in [fire_base_y - fire_rows, fire_base_y)      fire_base_y = band_area.y = divider+1
            => the fire's LAST row == the divider row
```

So the flame's densest base row is painted first and then **erased by the
divider** (`layout.rs:676-696` runs after `fire::render` at `:653-671`). The
flames are effectively floating one row above their own base, and embers (the
2-row case) show only one visible row.

Making the divider the ignition line therefore *fixes* a latent bug: the base
stops being swallowed.

## Design

### 1. New pure helper in `fire.rs`

```rust
/// Per-column ignition for the divider row (the flame's base). `None` keeps
/// the divider glyph; `Some(ch)` replaces it with a thin braille dot.
/// The selection uses the SLOW hash (1 s) and the unanimated envelope, so the
/// line cannot flicker (same discipline as the tint mask, F2).
pub fn ignition_row(width: usize, phase_ms: u128, level: f64) -> Vec<Option<char>>
```

Rules:
- `level < MIN_LEVEL` or `width == 0` ⇒ all `None` (divider stays `_`).
- Per column: reuse the fire's envelope `edge(x)`; a column is *ignitable* only
  where `edge > 0` (the sides climb, the middle stays dark — same silhouette as
  the flames).
- Ignition probability scales with the **unanimated** base reach
  `light = edge * level * 1.15`; a slow hash
  (`step = phase_ms / IGNITION_STEP_MS`, `IGNITION_STEP_MS = 1000`) decides
  which of those columns light this second ⇒ embers light a few, a full draw
  lights nearly every side column.
- Glyph is `DENSITY[1]` = `⡀` (single bottom dot: "thin braille line"), colour
  from the existing ramp at base heat (`style.color(base_heat, phase_ms)`).

`column_edge()` and `column_light()` are extracted from `render()` so the
flames and the ignition line cannot drift apart.

### 2. Renderer (`layout.rs`)

- `fire_base_y` = **the divider row** (`area.y + chat_area.height`), not
  `band_area.y`.
- Flames are drawn in `[divider - fire_rows', divider)` with
  `fire_rows' = max(1, rows_for(level) - 1)` — one row less than today, because
  that row is now the ignition line. Net strip footprint unchanged (the row was
  previously hidden anyway).
- The divider block paints per column: `⡀` in flame colour where
  `ignition_row` says so, `_` in `BORDER_DIM` elsewhere; background stays
  `theme::BG` (the fire never gets a panel plate).
- The tint pass's `fire_y` must be recomputed from the new rect top (today it
  assumes `input_area.y - fire_rows`).

### 3. Interaction with F1–F3 (flicker)

The ignition line is *inside* the same strip, so it must obey the same rules:
slow hash (1 s), unanimated envelope, no per-frame membership churn. Invariant
I6 below pins it.

## Invariants

- **I6 — no fire, plain divider**: `fire_on == false` or `level < MIN_LEVEL` ⇒
  the divider is exactly `"_" × width` in `BORDER_DIM` on `BG` (byte-identical
  to today; golden screen test).
- **I7 — the divider row carries only two glyphs**: `_` (unlit) and `⡀` (lit).
  No flame braille ever lands on it; flames occupy rows strictly above.
- **I8 — selective ignition**: at a low level only a minority of columns light;
  the count is monotone non-decreasing in level at a fixed phase; columns in the
  middle band (`edge == 0`) never light.
- **I9 — the line cannot flicker**: two frames with the phase advanced by
  < 1 s produce the identical lit-column set.

## Tests

- `ignition_row`: all-`None` below `MIN_LEVEL`; ≥1 lit at level 0.1; monotone
  count over a level sweep at fixed phase; middle columns always `None`;
  identical set for phase steps of 120 ms (< 1 s).
- `layout` screen dump: divider is all `_` with `/fire off`; contains `⡀` and
  only `_`/`⡀` with fire on; no `⡀` above the divider.
- Golden parity: `fire_on = false` renders byte-identically to the pre-change
  build (guards the "no fire" path).

## Trade-offs

- The fire reaches one row *higher* than its visible height today unless the
  braille row count drops by one (proposed: `max(1, rows_for - 1)`), so the
  composer's clear space is unchanged.
- Embers at the lowest levels now show a lit base with 1 row of flame above,
  which reads better than the current single floating row.
- The ignition glyph is one dot wide; a two-dot variant (`⡄`) is a one-line
  change if the operator wants a stronger line at high load.
