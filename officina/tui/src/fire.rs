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
                cycle_color(&PULSE_STOPS, pos)
            }
            style => theme::ramp_color(style.stops(), t),
        }
    }

    fn stops(self) -> &'static [ratatui::style::Color] {
        match self {
            // Deep emerald → Vitriolum green → solvent mint.
            FireStyle::Emerald => &[
                ratatui::style::Color::Rgb(0x0B, 0x3D, 0x2B),
                theme::GREEN,
                ratatui::style::Color::Rgb(0x7D, 0xFF, 0xB0),
            ],
            // Silver → gold → antidote → substrate (nigredo→rubedo arc).
            FireStyle::Alchemy => &[theme::SILVER, theme::GOLD, theme::ORANGE, theme::RED],
            FireStyle::Pulse => &PULSE_STOPS,
        }
    }
}

/// Pulse palette — the full alchemical set, ordered for a smooth loop
/// (red wraps back to silver through the shared brightness band).
const PULSE_STOPS: [ratatui::style::Color; 7] = [
    theme::SILVER,
    theme::GREEN,
    theme::GOLD,
    theme::CYAN,
    theme::VIOLET,
    theme::ORANGE,
    theme::RED,
];

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
        return theme::MUTED;
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

/// Draw the fire inside `area` (anchored so its bottom row touches the
/// composer's top edge). `level` in [0,1] drives height, density and heat;
/// `style` picks the color arc.
///
/// Returns the fire's per-cell colors (rows × cols, `None` = not burning)
/// so the chat renderer can tint text glyphs sitting over flames (owner
/// request 2026-09-02: "user text discolors based on the fire beneath it —
/// do the same for AI text"). Empty vec when nothing burns.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    phase_ms: u128,
    level: f64,
    style: FireStyle,
) -> Vec<Vec<Option<ratatui::style::Color>>> {
    let mut fire_map: Vec<Vec<Option<ratatui::style::Color>>> = Vec::new();
    let rows = rows_for(level).min(area.height as usize);
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
        let mut map_row: Vec<Option<ratatui::style::Color>> = vec![None; w];
        let mut spans: Vec<Span> = Vec::with_capacity(w);
        for x in 0..w {
            // Column envelope — the fire CLIMBS THE SIDES (owner request):
            // dead in the middle band, full burn at the edges, soft
            // shoulder so the inner falloff isn't a hard cliff.
            let u = if w <= 2 { 0.5 } else { x as f64 / (w - 1) as f64 };
            let edge = (((2.0 * u - 1.0).abs() - 0.12).max(0.0) / 0.88).powf(0.65);
            // Column height in [0,1] of the strip + per-column flicker so
            // the tips stay ragged and alive; the 1.15 reach lets a full
            // load run the whole strip height along the edges.
            let col_h = (edge * level * 1.15).clamp(0.0, 1.0) * (0.7 + 0.6 * hash01(x as u32, 0, step));
            let frac_from_base = from_base / rows as f64;
            if frac_from_base > col_h {
                // Above the flame — a rising ember? (owner request: little
                // embers flare up alongside the columns.) Side columns
                // only; each ember lives EMBER_SLOTS flicker slots, drifting
                // up one row-ish per slot, then the hash rerolls it.
                let near_side = edge > 0.3;
                if near_side && hash01(x as u32, 777, ember_step) < EMBER_ODDS {
                    let e_top = col_h + 0.12 + ember_sub * 0.14;
                    if (frac_from_base - e_top).abs() < 0.08 {
                        // Sparks are cool: silver/gold, barely there.
                        let heat = 0.25 + 0.2 * hash01(x as u32, 888, ember_step);
                        let c = style.color(heat, phase_ms);
                        map_row[x] = Some(c);
                        spans.push(Span::styled(
                            "⠁".to_string(),
                            Style::new().fg(c).bg(theme::BG),
                        ));
                        continue;
                    }
                }
                spans.push(Span::raw(" "));
                continue;
            }
            // Position within this column's flame: 0 base → 1 tip.
            let frac = (frac_from_base / col_h.max(1e-3)).clamp(0.0, 1.0);
            // Heat: base hotter, tips cooler; whole arc shifts hot with load.
            let heat = (base_heat * (1.0 - frac * 0.75)).clamp(0.0, 1.0);
            // Density: dots pile toward the base, thin at the tips, with
            // per-cell flicker keeping the surface alive.
            let dens = ((1.0 - frac * 0.8) * (0.7 + 0.6 * hash01(x as u32, r as u32, step)))
                .clamp(0.0, 1.0);
            let idx = (dens * (DENSITY.len() - 1) as f64).round() as usize;
            let ch = DENSITY[idx.min(DENSITY.len() - 1)];
            let c = style.color(heat, phase_ms);
            map_row[x] = Some(c);
            spans.push(Span::styled(
                ch.to_string(),
                Style::new().fg(c).bg(theme::BG),
            ));
        }
        fire_map.push(map_row);
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), area);
    fire_map
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
            assert_eq!(bits(w[0]) & bits(w[1]), bits(w[0]), "{:?} → {:?}", w[0], w[1]);
            assert_ne!(bits(w[0]), bits(w[1]));
        }
        assert_eq!(*DENSITY.last().unwrap(), '⣷');
    }
}
