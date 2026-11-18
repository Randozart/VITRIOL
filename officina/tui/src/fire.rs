//! Composer flames — braille alchemical fire climbing the prompt box.
//!
//! Owner request 2026-09-02: as inference draws more power from the GPU the
//! fire gets more intense and "decolors into alchemical colors" — silver
//! ember → gold → antidote orange → substrate red (the nigredo→rubedo arc),
//! hot at the base, cooler at the ragged tips. Shaped to CLIMB THE SIDES
//! (owner refinement, same session): the envelope hugs the composer's left
//! and right edges, the middle stays clear, and little embers pop loose
//! and rise above the flame columns. Load arrives via the `engine-fire`
//! widget (nvidia-smi power draw through _shared/engine.ts, activity proxy
//! fallback — see decode.ts `fireLoad`); the run loop low-passes it so the
//! fire breathes instead of strobing.
//!
//! Same discipline as watermark.rs: everything derives from (cell, phase)
//! hashes — no RNG state, any frame reproducible from the clock. The burn
//! OVERLAYS the chat's bottom rows (owner decision): the density ramp stays
//! airy even at full blaze so the text stays hinted through the flames.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;
use crate::watermark::cell_hash;

/// Alchemical ramps — the fire's color arc with rising load. Three voices,
/// owner-configurable via /fire (2026-09-02): `prismatic` (the whole
/// palette drifting through the flames — default), `emerald` (the
/// Vitriolum living fire), and `alchemy` (silver→gold→orange→red, the
/// nigredo→rubedo arc).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FireStyle {
    /// The full palette, cycling with time — hue travels up each column.
    /// Default (owner request 2026-09-02: "the prismatic fire").
    Pulse,
    /// Vitriolum green — the living fire.
    Emerald,
    /// Silver → gold → antidote → substrate (the classic arc).
    Alchemy,
}

impl FireStyle {
    pub const ALL: [FireStyle; 3] = [FireStyle::Pulse, FireStyle::Emerald, FireStyle::Alchemy];

    pub fn label(self) -> &'static str {
        match self {
            FireStyle::Pulse => "prismatic",
            FireStyle::Emerald => "emerald",
            FireStyle::Alchemy => "alchemy",
        }
    }

    /// "pulse" kept as an alias of "prismatic" (early persistence files).
    pub fn parse(s: &str) -> Option<FireStyle> {
        if s == "pulse" {
            return Some(FireStyle::Pulse);
        }
        FireStyle::ALL.iter().copied().find(|m| m.label() == s)
    }

    pub fn next(self) -> FireStyle {
        let i = FireStyle::ALL.iter().position(|m| *m == self).unwrap_or(0);
        FireStyle::ALL[(i + 1) % FireStyle::ALL.len()]
    }

    /// Color at `t` in [0,1] — static arcs for Emerald/Alchemy (heat-mapped),
    /// the drifting palette for Pulse.
    fn color(self, t: f64, phase_ms: u128) -> ratatui::style::Color {
        match self {
            FireStyle::Pulse => {
                // Slow full-palette drift. `t` offsets the cycle so heat
                // and height spread hues spatially: the base leads, the
                // tips lag — color waves travel up the flame.
                let pos = (phase_ms as f64 / (PULSE_PERIOD_S * 1000.0) + t * 0.35).fract();
                cycle_color(&pulse_stops(), pos)
            }
            style => {
                let stops = style.stops();
                theme::ramp_color(&stops, t)
            }
        }
    }

    /// Gradient stops for this style, in the ACTIVE theme.
    ///
    /// Owned rather than `&'static`: the stops are palette lookups now, and a
    /// borrowed slice of a temporary cannot escape (2026-11-18). Three-to-seven
    /// colours per call — the allocation is noise next to the frame it paints.
    fn stops(self) -> Vec<ratatui::style::Color> {
        match self {
            // Deep emerald → Vitriolum green → solvent mint.
            // Deep tint → the theme's green → a mint mixed toward its light
            // role. Derived instead of literal so the arc follows the palette.
            FireStyle::Emerald => vec![
                theme::lerp_color(theme::BG(), theme::GREEN(), 0.45),
                theme::GREEN(),
                theme::lerp_color(theme::GREEN(), theme::WHITE(), 0.45),
            ],
            // Silver → gold → antidote → substrate (nigredo→rubedo arc).
            FireStyle::Alchemy => {
                vec![
                    theme::SILVER(),
                    theme::GOLD(),
                    theme::ORANGE(),
                    theme::RED(),
                ]
            }
            FireStyle::Pulse => pulse_stops().to_vec(),
        }
    }
}

/// Pulse palette — the full alchemical set, ordered for a smooth loop
/// (red wraps back to silver through the shared brightness band).
///
/// A function, not a `const`: the stops come from the ACTIVE theme, and a const
/// cannot call the palette accessors (2026-11-18).
fn pulse_stops() -> [ratatui::style::Color; 7] {
    [
        theme::SILVER(),
        theme::GREEN(),
        theme::GOLD(),
        theme::CYAN(),
        theme::VIOLET(),
        theme::ORANGE(),
        theme::RED(),
    ]
}

/// Seconds for one full palette loop in Pulse mode.
const PULSE_PERIOD_S: f64 = 9.0;

/// Default voice (owner request 2026-09-02): prismatic.
impl Default for FireStyle {
    fn default() -> Self {
        FireStyle::Pulse
    }
}

/// Wraparound piecewise lerp — `t` in [0,1) loops seamlessly from the last
/// stop back to the first (ramp_color clamps; pulse must not).
fn cycle_color(stops: &[ratatui::style::Color], t: f64) -> ratatui::style::Color {
    let n = stops.len();
    if n == 0 {
        return theme::MUTED();
    }
    if n == 1 {
        return stops[0];
    }
    let t = t.fract();
    let scaled = t * n as f64;
    let idx = scaled.floor() as usize % n;
    let frac = scaled - scaled.floor();
    theme::lerp_color(stops[idx], stops[(idx + 1) % n], frac)
}

/// Flicker slot length — ~8 Hz, flame tempo, not strobe tempo.
const FLICKER_STEP_MS: u128 = 120;
/// Ember respawn window — each ember lives a few slots, then rerolls.
const EMBER_SLOTS: u32 = 4;
/// Probability a side column carries a rising ember per window.
const EMBER_ODDS: f64 = 0.09;
/// Below this the fire is absent entirely.
pub const MIN_LEVEL: f64 = 0.04;

/// Braille density ramp, ember → blaze. The ceiling is ⣷, never ⣿ — full
/// blocks would punch out the chat text underneath; this stays readable
/// (owner decision: overlay, "still readable"). Embers use the sparse end.
///
/// FILL DIRECTION (owner report 2026-09-03: "the individual braille blocks
/// have their directions inverted"): a rising flame fills each cell from
/// the BOTTOM — left column lowest-first (dot7 → dot3 → dot2 → dot1 =
/// ⡀ ⡄ ⡆ ⡇), then the right column bottom-up (+dot8 = ⣇, +dot6 = ⣧),
/// ending at the full block. The old ramp anchored ⠁⠃⠇ at the cell TOP —
/// half-filled cells read as dripping downward. All glyphs are common
/// braille coverage.
const DENSITY: [char; 8] = ['⠀', '⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷'];

fn hash01(x: u32, y: u32, step: u32) -> f64 {
    (cell_hash(x, y, step) % 1024) as f64 / 1024.0
}

/// Flame rows for a load level in [0,1]: 2 at embers, 6 at full draw.
pub fn rows_for(level: f64) -> usize {
    if level < MIN_LEVEL {
        return 0;
    }
    (2.0 + level * 4.0).round().clamp(2.0, 6.0) as usize
}

/// Half a row in level space (4 rows per unit of level).
const ROW_STEP_LEVEL: f64 = 0.125;
/// Extra slack before the strip changes height. Without it, a breathing
/// `level` crosses a rounding boundary repeatedly and adds/removes the tint on
/// a whole row of transcript text each time (operator: "text now occasionally
/// flickers", 2026-11-16).
const ROW_DEADBAND: f64 = 0.05;

/// Flame rows with a dead band: the strip changes height only once the level
/// has clearly left the current step, so a breathing signal cannot flap it.
/// Ignition (`previous == 0`) and extinction are immediate — the fire must
/// appear and vanish when the load does.
pub fn rows_for_stable(level: f64, previous: usize) -> usize {
    let raw = rows_for(level);
    if raw == 0 || previous == 0 {
        return raw;
    }
    let prev = previous.clamp(2, 6);
    let centre = (prev as f64 - 2.0) / 4.0;
    let keep = (level - centre).abs() <= ROW_STEP_LEVEL + ROW_DEADBAND;
    if keep {
        prev
    } else {
        raw
    }
}

/// Column envelope — the fire CLIMBS THE SIDES: dead in the middle band, full
/// burn at the edges, soft shoulder so the inner falloff isn't a hard cliff.
fn column_edge(x: usize, w: usize) -> f64 {
    let u = if w <= 2 {
        0.5
    } else {
        x as f64 / (w - 1) as f64
    };
    (((2.0 * u - 1.0).abs() - 0.12).max(0.0) / 0.88).powf(0.65)
}

/// A column's **unanimated** flame reach in [0, 1] of the strip height. This is
/// the flame's *shape*; `render` multiplies it by a per-frame hash for the
/// ragged tips. Quadruple-bookkeeping: shape drives the stable consumers (text
/// tint, ignition line), noise drives only the glyph field.
pub fn column_light(edge: f64, level: f64) -> f64 {
    (edge * level * 1.15).clamp(0.0, 1.0)
}

/// Height of the flame's bottom row inside a column of `rows` cells, in the
/// same fraction space `render` uses (`from_base / rows`).
fn base_frac(rows: usize) -> f64 {
    0.5 / rows.max(1) as f64
}

/// How far a tongue narrows by the top of the strip (0 = a rectangle, the
/// pre-2026-11-16 behaviour; owner: "the row above is the same width rather than
/// tapering off").
pub const DEFAULT_TAPER: f64 = 0.55;
/// Mid-point of a tongue in `edge` space: 0 at the inner boundary (next to the
/// dead middle band), 1 at the screen edge.
const TONGUE_CENTER: f64 = 0.55;

/// Envelope width kept at height fraction `rn` (0 base → 1 tip).
fn taper_scale(rn: f64, taper: f64) -> f64 {
    // sqrt: the FIRST row above the base must already be narrower, or the taper
    // is invisible exactly where the owner looks ("the rule row and the row
    // above it are the same width", 2026-11-16). A linear ramp spends its effect
    // on the top rows, which are 1-2 cells wide anyway.
    //
    // 2026-11-17 (owner: "the bottom two fire rows are the same width"): sqrt
    // alone was not enough. At 6 rows the first step was 1 - 0.55*sqrt(1/6) =
    // 0.78, and with the bottom row exempt (rn = 0) the strip still opened with
    // two identical rows. BASE_INSET spends part of the taper immediately, so
    // the first row above the rule is visibly narrower at EVERY strip height,
    // while the tip still lands exactly on 1 - taper.
    const BASE_INSET: f64 = 0.18;
    let r = rn.clamp(0.0, 1.0);
    1.0 - taper.clamp(0.0, 1.0) * (BASE_INSET + (1.0 - BASE_INSET) * r.sqrt())
}

/// Glyph-density multiplier by distance from the tongue's boundary.
///
/// `t` is 0 at the tongue's midline and 1 at the cut. Inside the body the flame
/// keeps its full weight; over the outer ~20 % the weight falls so the edge ends
/// in single dots instead of a full bar followed by nothing — that cliff is what
/// read as a STAIRCASE (owner: "the edges are blocky rather than tapered",
/// 2026-11-17; still visible 2026-11-18 because only the geometry had been fixed).
///
/// Quadratic shoulder on purpose: a linear ramp puts most of its change at the
/// cut, which merely moves the stair one cell inward.
fn edge_falloff(t: f64) -> f64 {
    const KNEE: f64 = 0.78;
    let t = t.clamp(0.0, 1.0);
    if t <= KNEE {
        return 1.0;
    }
    let x = (t - KNEE) / (1.0 - KNEE);
    1.0 - 0.85 * x * x
}

/// Where the outermost sliver starts: above this the glyph is thinned enough
/// that dithering it breaks the stair instead of making it sparkle.
const EDGE_DITHER_FROM: f64 = 0.88;

/// Does the outermost sliver draw on THIS flicker step?
///
/// Extracted so the anti-stair behaviour is testable without a frame: the point
/// is that the boundary column alternates between frames, so successive rows
/// stop stepping in phase.
fn edge_dither_culls(x: u32, edge_t: f64, step: u32) -> bool {
    if edge_t <= EDGE_DITHER_FROM {
        return false;
    }
    let p = (edge_t - EDGE_DITHER_FROM) / (1.0 - EDGE_DITHER_FROM);
    hash01(x, 913, step) < p
}

/// Position of a glyph in the density ramp (test helper).
#[cfg(test)]
pub fn density_index(c: char) -> Option<usize> {
    DENSITY.iter().position(|d| *d == c)
}

/// The glyph this cell draws, given its density and its distance from the
/// tongue's boundary.
///
/// Extracted from the render loop so the fade can be asserted without the
/// per-cell hash noise: at equal density, a cell nearer the edge must draw a
/// LIGHTER glyph. That is the property that removes the stair-step, and it is
/// invisible in a whole-frame assertion because the hash dominates there.
pub fn glyph_at(dens: f64, edge_t: f64) -> char {
    let d = (dens * edge_falloff(edge_t)).clamp(0.0, 1.0);
    let idx = (d * (DENSITY.len() - 1) as f64).round() as usize;
    DENSITY[idx.min(DENSITY.len() - 1)]
}

/// Test probe: the dither decision (see `edge_dither_culls`).
#[cfg(test)]
pub fn edge_dither_culls_probe(x: u32, edge_t: f64, step: u32) -> bool {
    edge_dither_culls(x, edge_t, step)
}
/// Test probe: where the dither band starts.
#[cfg(test)]
pub const EDGE_DITHER_FROM_PROBE: f64 = EDGE_DITHER_FROM;

/// Is this column inside the tapered tongue at this height?
///
/// The window SHRINKS around the tongue's midline, which is how a flame narrows
/// toward its own tip. Dropping an end instead would either detach the flame
/// from the centre (inner) or leave the screen edge dark (outer).
pub fn inside_tongue(edge: f64, rn: f64, taper: f64) -> bool {
    (edge - TONGUE_CENTER).abs() <= 0.5 * taper_scale(rn, taper)
}

/// At and above this load the rule is the flame's UNBROKEN base; below it the
/// fire is embers and the rule keeps `_` between flamelets.
const BASE_CONTINUOUS_LEVEL: f64 = 0.25;

/// How hot the rule row runs, relative to the field's base row.
///
/// Must stay above `0.3 + 0.5*level` (the base-row taper in `render`): the rule
/// is where the fire is born, so it is the hottest cell of the column, and the
/// strict inequality also guarantees the rule is never a copy of the row above.
fn rule_heat(level: f64) -> f64 {
    0.55 + 0.45 * level.clamp(0.0, 1.0)
}

/// Cells of the DIVIDER row: the fire's own bottom row, drawn on the line
/// (owner decision 2026-11-16: *"take the current fire, and start it fully on
/// the underscore row … once the flame gets started, it consumes whatever part
/// of the underscore row it needs"*).
///
/// `None` keeps the underscore. A claimed column returns the glyph the flame
/// field would draw on its own bottom row — same ramp, same hash seed, so the
/// line and the body are one flame — except that the base density is scaled by
/// `level_scale` so first embers read as dots on the line rather than a bar.
/// `rows` must be the height of the flame field above the line.
pub fn line_cells(
    width: usize,
    rows: usize,
    phase_ms: u128,
    level: f64,
    taper: f64,
) -> Vec<Option<char>> {
    let mut out = vec![None; width];
    if width == 0 || level < MIN_LEVEL {
        return out;
    }
    let step = (phase_ms / FLICKER_STEP_MS) as u32;
    let last = rows.saturating_sub(1) as u32;
    for (x, slot) in out.iter_mut().enumerate() {
        let col_light = column_light(column_edge(x, width), level);
        if col_light <= 0.0 {
            continue;
        }
        // The rule IS the field's bottom row, so it takes the same BASE
        // fraction (0 after the 2026-11-16 remap) and the same hash seed: one
        // density field, scaled differently (rule hotter than the taper above
        // it, so the fire is born here and the rows cannot come out identical).
        let base_dens = (0.7 + 0.6 * hash01(x as u32, last, step)).clamp(0.0, 1.0);
        // THE FIRE SPAWNS ON THE RULE (owner decision 2026-11-16): the rule
        // cell is REPLACED by the flame's own base, not covered by a dot.
        // `rule_heat` sits deliberately above the field's base-row taper
        // (0.3 + 0.5*level) so the rule is the hottest row of the flame — and so
        // the two rows can never come out identical. At low load the scaled
        // value falls under the smoulder threshold in most columns, which is
        // what leaves the underscores *between* the flamelets.
        // THE BASE IS UNBROKEN AT WORKING LOAD (owner report 2026-11-16: "the
        // bottom divider row has gaps, rather than truly being the start of the
        // fire"). Below the threshold the fire is embers, so `_` survives
        // between flamelets; at/above it every column inside the envelope is
        // claimed and the rule is the flame's continuous base.
        if level < BASE_CONTINUOUS_LEVEL {
            let light = (col_light * 1.5).clamp(0.0, 1.0);
            if light <= 0.0 || hash01(x as u32, 313, step) >= light {
                continue;
            }
        }
        // The rule thins toward its own ends (2026-11-18). It is claimed across
        // the whole width and had no edge term, so it ended in a hard shelf while
        // the rows above it faded inward. The falloff changes WHICH glyph a
        // column draws, never WHETHER it draws: the base stays unbroken.
        let half = 0.5 * taper_scale(0.0, taper);
        let edge_t =
            ((column_edge(x, width) - TONGUE_CENTER).abs() / half.max(1e-6)).clamp(0.0, 1.0);
        let dens = base_dens * rule_heat(level) * edge_falloff(edge_t);
        let idx = (dens * (DENSITY.len() - 1) as f64).round() as usize;
        *slot = Some(DENSITY[idx.clamp(1, DENSITY.len() - 1)]);
    }
    out
}

/// Colour of the ignition dots — the flame's base heat.
pub fn ignition_color(phase_ms: u128, level: f64, style: FireStyle) -> ratatui::style::Color {
    let base_heat = (0.2 + 0.8 * level).clamp(0.0, 1.0);
    style.color(base_heat, phase_ms)
}

/// Draw the fire inside `area` (anchored so its bottom row sits on the
/// DIVIDER row the caller draws above the status band). `level` in [0,1]
/// drives height, density and heat; `style` picks the color arc.
///
/// Returns the **tint mask**: per-cell colours for text glyphs sitting over the
/// flames (owner request 2026-09-02: "user text discolors based on the fire
/// beneath it"). Since 2026-11-16 the mask follows the flame's *shape*
/// (`column_light`), not its per-frame noise: a glyph's tint changes when the
/// envelope crosses its row, not on every hash reroll — the flames keep
/// flickering, the text they lend colour to stops doing so. Embers are
/// decoration and never tint. Empty vec when nothing burns.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    phase_ms: u128,
    level: f64,
    style: FireStyle,
    taper: f64,
) -> Vec<Vec<Option<ratatui::style::Color>>> {
    let rows = rows_for(level).min(area.height as usize);
    let mut fire_map: Vec<Vec<Option<ratatui::style::Color>>> = Vec::new();
    if rows == 0 || area.width < 8 {
        return fire_map;
    }
    let w = area.width as usize;
    let step = (phase_ms / FLICKER_STEP_MS) as u32;
    let ember_step = step / EMBER_SLOTS;
    let ember_sub = (step % EMBER_SLOTS) as f64;
    // Overall heat: the whole fire runs hotter as the GPU draws more.
    let base_heat = (0.2 + 0.8 * level).clamp(0.0, 1.0);

    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for r in 0..rows {
        // Fraction of the flame column height, 0 at the base (bottom row,
        // touching the composer) → 1 at the tips.
        let from_base = (rows - 1 - r) as f64 + 0.5;
        let mut spans: Vec<Span> = Vec::with_capacity(w);
        for x in 0..w {
            let edge = column_edge(x, w);
            // Unanimated reach = the flame's shape; the hash only roughens the
            // glyphs drawn on top of it.
            let col_light = column_light(edge, level);
            // Clamped to the base row: a burning column always reaches the
            // rule, so the body cannot float above its own line (2026-11-16).
            let floor = if col_light > 0.0 {
                base_frac(rows)
            } else {
                0.0
            };
            let col_h = (col_light * (0.7 + 0.6 * hash01(x as u32, 0, step))).max(floor);
            let frac_from_base = from_base / rows as f64;
            // Taper (2026-11-16): each tongue narrows with height. The BASE row
            // (rn == 0) is exempt, so the rule keeps the envelope's full width
            // and stays unbroken.
            // Height fraction 0..1 where 0 is the RULE, not the bottom body
            // row. Deriving rn from the row index made the bottom body row
            // rn == 0, which the taper gate below exempts — so the rule row and
            // the row above it came out identical, the exact defect the owner
            // reported twice (2026-11-16 and again 2026-11-17). `from_base`
            // starts at half a row, so every body row tapers.
            let rn = (from_base / rows as f64).clamp(0.0, 1.0);
            let outside_taper = rn > 0.0 && !inside_tongue(edge, rn, taper);
            if outside_taper || frac_from_base > col_h {
                // Above the flame — a rising ember? (owner request: little
                // embers flare up alongside the columns.) Side columns
                // only; each ember lives EMBER_SLOTS flicker slots, drifting
                // up one row-ish per slot, then the hash rerolls it. Embers
                // are decoration: they never enter the tint mask.
                let near_side = edge > 0.3;
                if near_side && hash01(x as u32, 777, ember_step) < EMBER_ODDS {
                    let e_top = col_h + 0.12 + ember_sub * 0.14;
                    if (frac_from_base - e_top).abs() < 0.08 {
                        // Sparks are cool: silver/gold, barely there.
                        let heat = 0.25 + 0.2 * hash01(x as u32, 888, ember_step);
                        let c = style.color(heat, phase_ms);
                        spans.push(Span::styled(
                            "⠁".to_string(),
                            Style::new().fg(c).bg(theme::BG()),
                        ));
                        continue;
                    }
                }
                spans.push(Span::raw(" "));
                continue;
            }
            // Position within this column's flame: 0 base → 1 tip.
            //
            // Remapped so the LOWEST VISIBLE row is the base. Without this the
            // clamp `col_h >= base_frac` puts the bottom row at frac = 1 — its
            // own tip — so the flame's sparse end touched the rule and the
            // dense end floated a row above it: two detached bands (owner
            // report 2026-11-16).
            let span = (col_h - base_frac(rows)).max(1e-3);
            let frac = ((frac_from_base - base_frac(rows)) / span).clamp(0.0, 1.0);
            // Heat: base hotter, tips cooler; whole arc shifts hot with load.
            let heat = (base_heat * (1.0 - frac * 0.75)).clamp(0.0, 1.0);
            // Density: dots pile toward the base, thin at the tips, with
            // per-cell flicker keeping the surface alive. The BOTTOM row is
            // capped so it blends into the smouldering rule instead of butting
            // against it (a dense base next to thin embers read as two
            // detached bands — owner report 2026-11-16).
            let mut dens = ((1.0 - frac * 0.8) * (0.7 + 0.6 * hash01(x as u32, r as u32, step)))
                .clamp(0.0, 1.0);
            if r + 1 == rows {
                // Base row: TAPERED, not clamped — a clamp would flatten every
                // column to the same glyph and that uniform band is its own
                // kind of edge. Scaling keeps the per-column variation, so the
                // hand-over from the smouldering rule to the flame is a
                // gradient (owner report 2026-11-16).
                // Level-aware: embers keep the base close to the rule, a full
                // draw lets it thicken upward.
                dens *= 0.3 + 0.5 * level.clamp(0.0, 1.0);
            }
            // Edge softening (2026-11-18): thin the glyph toward the tongue's
            // boundary, then let the outer sliver come and go per flicker step.
            // Only the GLYPH field dithers; the text-tint mask takes the smooth
            // falloff alone, because text must never flicker.
            let half = 0.5 * taper_scale(rn, taper);
            let edge_t = ((edge - TONGUE_CENTER).abs() / half.max(1e-6)).clamp(0.0, 1.0);
            dens *= edge_falloff(edge_t);
            if edge_dither_culls(x as u32, edge_t, step) {
                spans.push(Span::raw(" "));
                continue;
            }
            let ch = glyph_at(dens, edge_t);
            let c = style.color(heat, phase_ms);
            spans.push(Span::styled(
                ch.to_string(),
                Style::new().fg(c).bg(theme::BG()),
            ));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
    fire_map = tint_rows(rows, w, phase_ms, level, style, taper);
    fire_map
}

/// The tint mask as a pure function (2026-11-16): which cells carry flame
/// colour for text sitting over them. Derived from the flame's SHAPE
/// (`column_light`, unanimated), never from the per-frame hash, so the tinted
/// set is identical for every frame at a given level — the fire may flicker,
/// the text it colours may not. `None` colours mean "no flame here".
pub fn tint_rows(
    rows: usize,
    width: usize,
    phase_ms: u128,
    level: f64,
    style: FireStyle,
    taper: f64,
) -> Vec<Vec<Option<ratatui::style::Color>>> {
    let base_heat = (0.2 + 0.8 * level).clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let from_base = (rows - 1 - r) as f64 + 0.5;
        let frac_from_base = from_base / rows.max(1) as f64;
        let mut row: Vec<Option<ratatui::style::Color>> = vec![None; width];
        for (x, cell) in row.iter_mut().enumerate() {
            let edge = column_edge(x, width);
            let light = column_light(edge, level);
            // Same silhouette the field draws, so text is never tinted outside
            // the visible flame (2026-11-16). Must use the SAME height fraction
            // as `render` (base-relative), or the mask and the drawn flame
            // disagree on the bottom rows (2026-11-17).
            let rn = (from_base / rows.max(1) as f64).clamp(0.0, 1.0);
            if rn > 0.0 && !inside_tongue(edge, rn, taper) {
                continue;
            }
            if frac_from_base > light {
                continue;
            }
            let frac = (frac_from_base / light.max(1e-3)).clamp(0.0, 1.0);
            // The mask follows the glyph's SHAPE but not its flicker: the smooth
            // edge falloff is applied, the dither is not. Text tinted by a
            // dithering mask would shimmer (owner report 2026-11-16).
            let half = 0.5 * taper_scale(rn, taper);
            let edge_t = ((edge - TONGUE_CENTER).abs() / half.max(1e-6)).clamp(0.0, 1.0);
            let heat = (base_heat * (1.0 - frac * 0.75) * edge_falloff(edge_t)).clamp(0.0, 1.0);
            *cell = Some(style.color(heat, phase_ms));
        }
        out.push(row);
    }
    out
}

#[cfg(test)]
mod density_tests {
    use super::*;

    /// Owner report 2026-09-03: individual braille blocks were
    /// direction-inverted — the ramp anchored at the cell TOP. A rising
    /// flame fills each cell from the bottom: lowest dot first, monotone
    /// bit growth, ending at the full block.
    #[test]
    fn density_ramp_fills_cells_bottom_up() {
        let bits = |c: char| (c as u32) - 0x2800;
        assert_eq!(bits(DENSITY[0]), 0x00); // empty
        assert_eq!(bits(DENSITY[1]), 0x40); // dot 7 — the lowest-left dot
        assert_eq!(bits(DENSITY[2]), 0x44); // + dot 3
        assert_eq!(bits(DENSITY[3]), 0x46); // + dot 2
        assert_eq!(bits(DENSITY[4]), 0x47); // full left column
        assert_eq!(bits(DENSITY[5]), 0xC7); // + dot 8 (bottom-right)
        assert_eq!(bits(DENSITY[6]), 0xE7); // + dot 6
                                            // Ceiling: ⣷ = 0xF7 — every dot except the top-right (dot 4). The
                                            // original readability rule stands: never the full ⣿ block.
        assert_eq!(bits(DENSITY[7]), 0xF7);
        // Monotone: each step only ADDS dots.
        for w in DENSITY.windows(2) {
            assert_eq!(
                bits(w[0]) & bits(w[1]),
                bits(w[0]),
                "{:?} → {:?}",
                w[0],
                w[1]
            );
            assert_ne!(bits(w[0]), bits(w[1]));
        }
        assert_eq!(*DENSITY.last().unwrap(), '⣷');
    }
}

#[cfg(test)]
mod stability_tests {
    use super::*;

    /// The strip height must survive a breathing level: without the dead band
    /// a level hovering on a rounding boundary flipped the row count (and the
    /// tint on a whole row of transcript text) every frame.
    #[test]
    fn rows_for_stable_holds_through_level_jitter() {
        for prev in 2..=6 {
            let centre = (prev as f64 - 2.0) / 4.0;
            for jitter in [-0.04, -0.01, 0.0, 0.01, 0.04] {
                let level = (centre + jitter).clamp(MIN_LEVEL, 1.0);
                assert_eq!(
                    rows_for_stable(level, prev),
                    prev,
                    "level {level} near the centre {centre} must hold {prev} rows"
                );
            }
        }
    }

    #[test]
    fn rows_for_stable_follows_a_clear_move() {
        assert_eq!(rows_for_stable(0.95, 2), rows_for(0.95));
        assert_eq!(rows_for_stable(0.10, 6), rows_for(0.10));
    }

    #[test]
    fn rows_for_stable_ignites_and_dies_without_lag() {
        // Embers appear the moment there is any fire, and the strip vanishes
        // the moment the level drops below MIN_LEVEL — no dead band there.
        assert_eq!(rows_for_stable(0.10, 0), 2);
        assert_eq!(rows_for_stable(0.0, 4), 0);
    }

    /// The tint mask must not churn with the flame's per-frame hash: the set
    /// of cells lending colour to text is identical on consecutive frames.
    #[test]
    fn tint_mask_is_identical_across_frames_at_one_level() {
        let pattern = |phase: u128| -> Vec<Vec<bool>> {
            tint_rows(6, 80, phase, 0.6, FireStyle::Pulse, DEFAULT_TAPER)
                .iter()
                .map(|row| row.iter().map(Option::is_some).collect())
                .collect()
        };
        let base = pattern(0);
        for phase in [120u128, 240, 360, 480, 600] {
            assert_eq!(
                pattern(phase),
                base,
                "tinted cell set changed at {phase}ms — text would flicker"
            );
        }
        assert!(base.iter().flatten().any(|b| *b), "expected tint at 0.6");
        assert!(
            base.iter().flatten().any(|b| !*b),
            "expected clear cells (the middle band never tints)"
        );
    }

    #[test]
    fn tint_mask_is_empty_without_fire() {
        let rows = tint_rows(4, 40, 0, 0.0, FireStyle::Emerald, DEFAULT_TAPER);
        assert!(rows.iter().flatten().all(Option::is_none));
    }

    /// More load ⇒ the envelope covers more rows, so the tinted set grows.
    #[test]
    fn tint_mask_grows_with_load() {
        let count = |level: f64| {
            tint_rows(6, 80, 0, level, FireStyle::Alchemy, DEFAULT_TAPER)
                .iter()
                .flatten()
                .filter(|c| c.is_some())
                .count()
        };
        let mut prev = 0;
        for level in [0.1, 0.3, 0.6, 1.0] {
            let now = count(level);
            assert!(now >= prev, "tint area shrank at {level}: {now} < {prev}");
            prev = now;
        }
    }

    /// The fire climbs the sides: the rule's middle band is never consumed.
    #[test]
    fn line_never_claims_the_middle_band() {
        let w = 120;
        for level in [0.1, 0.5, 1.0] {
            let row = line_cells(w, 4, 500, level, DEFAULT_TAPER);
            for (x, cell) in row.iter().enumerate() {
                if cell.is_some() {
                    assert!(
                        column_edge(x, w) > 0.0,
                        "column {x} is inside the dead middle band"
                    );
                }
            }
        }
    }

    #[test]
    fn line_claims_more_as_the_fire_grows() {
        let claimed = |level: f64| {
            line_cells(200, 4, 500, level, DEFAULT_TAPER)
                .iter()
                .filter(|c| c.is_some())
                .count()
        };
        let mut prev = 0;
        for level in [0.06, 0.15, 0.3, 0.5, 0.75, 1.0] {
            let now = claimed(level);
            assert!(now >= prev, "claimed cells fell at {level}: {now} < {prev}");
            prev = now;
        }
        assert!(prev > 0);
    }

    #[test]
    fn line_is_untouched_without_fire() {
        for phase in [0u128, 700, 1234] {
            let row = line_cells(80, 4, phase, 0.0, DEFAULT_TAPER);
            assert_eq!(row.len(), 80);
            assert!(row.iter().all(Option::is_none), "no fire => plain rule");
        }
        assert!(line_cells(80, 4, 0, MIN_LEVEL * 0.5, DEFAULT_TAPER)
            .iter()
            .all(Option::is_none));
        assert!(line_cells(0, 4, 0, 1.0, DEFAULT_TAPER).is_empty());
    }

    /// Owner decision 2026-11-16: at first embers the rule is only
    /// *intermittently* broken up by the flame.
    #[test]
    fn first_embers_only_dot_the_line() {
        let w = 160;
        let row = line_cells(w, 4, 500, 0.06, DEFAULT_TAPER);
        let claimed: Vec<usize> = row
            .iter()
            .enumerate()
            .filter(|(x, c)| c.is_some() && column_light(column_edge(*x, w), 0.06) > 0.0)
            .map(|(x, _)| x)
            .collect();
        assert!(!claimed.is_empty(), "embers must touch the line somewhere");
        let lit_zone: Vec<usize> = (0..w)
            .filter(|x| column_light(column_edge(*x, w), 0.06) > 0.0)
            .collect();
        assert!(
            claimed.len() < lit_zone.len(),
            "underscores must survive inside the lit zone"
        );
        // Sparse *presence* is the ember property (a minority of columns);
        // the glyph itself may already carry a few dots, because the rule is
        // where the fire is born.
        for (x, cell) in row.iter().enumerate() {
            if let Some(g) = cell {
                let idx = DENSITY.iter().position(|d| d == g).expect("ramp glyph");
                assert!(idx <= 5, "column {x} too dense for embers: {g:?}");
            }
        }
    }

    /// F-B1 (owner report 2026-11-16): at working load the rule is the flame's
    /// UNBROKEN base — no `_` anywhere inside the envelope.
    #[test]
    fn base_is_unbroken_at_working_load() {
        let w = 160;
        for level in [0.25, 0.3, 0.6, 1.0] {
            let row = line_cells(w, 4, 500, level, DEFAULT_TAPER);
            for x in 0..w {
                let light = column_light(column_edge(x, w), level);
                if light <= 0.0 {
                    assert!(
                        row[x].is_none(),
                        "middle band must keep the rule at {level}"
                    );
                    continue;
                }
                assert!(
                    row[x].is_some(),
                    "level {level}: gap in the base at column {x}"
                );
            }
        }
    }

    /// F-B2: below the ember level the smoulder survives — `_` between
    /// flamelets, because a fully claimed rule with nothing above it reads as a
    /// bare bright line.
    #[test]
    fn base_still_smoulders_below_the_ember_level() {
        let w = 200;
        let row = line_cells(w, 4, 500, 0.1, DEFAULT_TAPER);
        let lit_zone: Vec<usize> = (0..w)
            .filter(|x| column_light(column_edge(*x, w), 0.1) > 0.0)
            .collect();
        let claimed = lit_zone.iter().filter(|x| row[**x].is_some()).count();
        assert!(claimed > 0, "embers must light something");
        assert!(
            claimed < lit_zone.len(),
            "underscores must survive between embers ({claimed}/{})",
            lit_zone.len()
        );
    }

    /// Once it is burning, the flame consumes the line where it burns.
    ///
    /// Presence is probabilistic (`light = col_light * 1.5`), so only columns
    /// whose reach guarantees it are expected to be claimed: at full load that
    /// is `col_light > 0.67`.
    #[test]
    fn a_burning_flame_consumes_its_line() {
        let w = 160;
        let row = line_cells(w, 4, 500, 1.0, DEFAULT_TAPER);
        for x in 0..w {
            let light = column_light(column_edge(x, w), 1.0);
            if light > 0.67 {
                assert!(row[x].is_some(), "column {x} burns but left its underscore");
            } else if light == 0.0 {
                // The dead middle band always keeps the underscore.
                assert!(row[x].is_none(), "middle band must keep the rule");
            }
        }
    }

    /// The rule follows the fire's own clock (120 ms), not a calm 1 s slot.
    #[test]
    fn line_follows_the_flame_clock() {
        let base = line_cells(120, 4, 0, 0.8, DEFAULT_TAPER);
        assert_eq!(
            line_cells(120, 4, 60, 0.8, DEFAULT_TAPER),
            base,
            "same flicker slot"
        );
        assert_eq!(
            line_cells(120, 4, 119, 0.8, DEFAULT_TAPER),
            base,
            "same flicker slot"
        );
    }

    #[test]
    fn line_glyphs_come_from_the_flame_ramp() {
        let row = line_cells(120, 4, 0, 1.0, DEFAULT_TAPER);
        for g in row.iter().flatten() {
            assert!(DENSITY.contains(g), "{g:?} is not a ramp glyph");
            // The shipped ceiling is ⣷ (0xF7); the full block ⣿ stays banned
            // so transcript text underneath the line never disappears.
            assert_ne!(*g, '⣿', "never the full block");
            assert!((*g as u32) - 0x2800 <= 0xF7, "{g:?} exceeds the ceiling");
        }
    }
}

#[cfg(test)]
mod edge_softening_tests {
    use super::*;

    /// The text-tint mask takes the smooth fade and NEVER the dither: the same
    /// state must produce a byte-identical mask on every flicker step, or the
    /// transcript would shimmer (owner report 2026-11-16).
    #[test]
    fn tint_mask_does_not_flicker() {
        let a = tint_rows(6, 100, 0, 1.0, FireStyle::Alchemy, DEFAULT_TAPER);
        let b = tint_rows(6, 100, 137, 1.0, FireStyle::Alchemy, DEFAULT_TAPER);
        let c = tint_rows(6, 100, 4_000, 1.0, FireStyle::Alchemy, DEFAULT_TAPER);
        assert_eq!(
            format!("{a:?}"),
            format!("{b:?}"),
            "tint changed between frames"
        );
        assert_eq!(
            format!("{b:?}"),
            format!("{c:?}"),
            "tint changed between frames"
        );
    }

    /// The fade must soften the edge without erasing it: even at the cut the cell
    /// still draws something.
    #[test]
    fn the_cut_still_draws() {
        let cut = glyph_at(1.0, 1.0);
        assert_ne!(cut, '⠀', "the boundary draws blank braille");
        assert!(
            density_index(cut).is_some_and(|i| i >= 1 && i <= 3),
            "{cut:?}"
        );
    }

    /// At embers the fire must stay visible: the fade multiplies an already small
    /// density, so this guards against the bar disappearing entirely.
    #[test]
    fn embers_survive_the_fade() {
        let row: Vec<char> = (0..80)
            .map(|x| {
                let edge = (x as f64 / 79.0 - 0.5).abs() * 2.0; // 0 centre → 1 edge
                glyph_at(0.35, edge)
            })
            .collect();
        let lit = row
            .iter()
            .filter(|c| density_index(**c).is_some_and(|i| i >= 1))
            .count();
        assert!(lit > 20, "the ember row went dark: {lit}/80 columns lit");
    }
}

#[cfg(test)]
mod rule_fade_tests {
    use super::*;

    fn index(c: char) -> usize {
        density_index(c).expect("density glyph")
    }

    /// The rule used to be a uniformly heavy band that stopped only because the
    /// frame did (owner: "not quite yet on the rule row"). Its ends must thin.
    ///
    /// Compared as min-vs-max of the claimed columns: the densest column sits at
    /// TONGUE_CENTER in *edge* space, which maps to ~26 %/~74 % of the width, so
    /// locating it by a width fraction would be wrong.
    #[test]
    fn the_rule_fades_toward_its_ends() {
        let w = 120usize;
        let row = line_cells(w, 4, 0, 1.0, DEFAULT_TAPER);
        let claimed: Vec<usize> = row.iter().filter_map(|c| c.map(index)).collect();
        assert!(!claimed.is_empty(), "the rule draws nothing at full load");
        let dense = *claimed.iter().max().unwrap();
        let thin = *claimed.iter().min().unwrap();
        // min<max alone is satisfied by the per-cell hash noise, so pin the SHAPE:
        // the outermost claimed column must be a single-dot glyph, and the body
        // must still be heavy. With the fade removed the outermost column draws
        // whatever the hash gives (5-7) and this fails.
        let outermost = {
            let mut best = 0usize;
            for (x, c) in row.iter().enumerate() {
                if let Some(c) = c {
                    best = x;
                    let _ = c;
                }
            }
            index(row[best].expect("outermost claimed column"))
        };
        assert!(
            outermost <= 2,
            "the outermost rule column is still heavy: index {outermost}"
        );
        assert!(
            dense >= 5,
            "the rule's body is too thin to be a base: {dense}"
        );
        assert!(
            thin < dense,
            "the rule still ends flat: thin={thin} dense={dense}"
        );
        assert!(thin >= 1, "the outermost rule column draws blank braille");
    }

    /// I1: the fade changes WHICH glyph, never WHETHER one is drawn — the base is
    /// unbroken at working load (owner invariant, 2026-11-16).
    #[test]
    fn the_rule_stays_unbroken_at_working_load() {
        let w = 100usize;
        let row = line_cells(w, 4, 0, 0.6, DEFAULT_TAPER);
        let drawn = row.iter().filter(|c| c.is_some()).count();
        assert!(drawn > w * 3 / 4, "the rule has gaps: {drawn}/{w} columns");
    }

    /// I4: a line that blinks is a GAP. The glyphs on the rule flicker by design
    /// (the same per-cell hash the body uses), but whether a column is claimed
    /// must not change between frames.
    #[test]
    fn the_rule_does_not_blink() {
        let presence = |phase: u128| -> Vec<bool> {
            line_cells(80, 4, phase, 0.8, DEFAULT_TAPER)
                .iter()
                .map(|c| c.is_some())
                .collect()
        };
        assert_eq!(presence(0), presence(137), "the rule blinked");
        assert_eq!(presence(0), presence(5_000), "the rule blinked");
    }

    /// I2: in the body the rule is still the hottest row — the fade is 1.0 there,
    /// so `rule_heat` must stay above the base-row factor.
    #[test]
    fn the_rule_is_still_hotter_than_the_row_above() {
        for level in [0.3, 0.6, 1.0] {
            let base_row = 0.3 + 0.5 * level;
            assert!(
                rule_heat(level) > base_row,
                "level {level}: rule_heat {} <= base row {base_row}",
                rule_heat(level)
            );
        }
    }
}
