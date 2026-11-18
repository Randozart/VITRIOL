//! Officina themes: **vitriol** (the original), **aether** (Kanagawa Wave) and
//! **alkahest** (neon alchemy).
//!
//! Palette history: ported from vitriol-tui/src/theme.rs (canonical Vitriolum
//! definition, this repo, Apache-2.0), derived from the Alka Officina dark
//! theme re-centered on the VITRIOL logo colours. Semantics map to alchemy:
//! solvent cyan = the process flowing; gold = work completed; green = the living
//! fire (healthy); red = substrate (crash).
//!
//! 2026-11-18: the palette became DATA. Every colour is a field of `Theme`, three
//! statics exist (`VITRIOL`, `AETHER`, `ALKAHEST`), and the accessors below keep
//! the historical constant names — `theme::GOLD()` became `theme::GOLD()` — so the
//! change is a mechanical rename at 700+ call sites rather than a rewiring of
//! every render function. `#[allow(non_snake_case)]` is deliberate: the names are
//! a palette table, and keeping them 1:1 with the old constants is what makes the
//! diff reviewable.

use ratatui::style::{Color, Modifier, Style};
use std::sync::RwLock;

/// One complete colour scheme.
///
/// `ramp_*` fields are gradients the UI interpolates (gauges, fire heat); they
/// live here because a theme that only swaps flat colours leaves its gauge ramps
/// and its flames in the old palette.
pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel: Color,
    pub user_bg: Color,
    pub border_dim: Color,
    pub green: Color,
    pub gold: Color,
    pub cyan: Color,
    pub orange: Color,
    pub red: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub violet: Color,
    pub cold_blue: Color,
    pub light_yellow: Color,
    pub deep_red: Color,
    pub dark_teal: Color,
    pub mercury: Color,
    pub silver: Color,
    pub watermark: Color,
    pub white: Color,
    /// Cold → hot gradient: index 0 is embers, the last stop is the hot end.
    ///
    /// Consumed by `BrailleRamp::Heat` (gauge instrumentation, `cfg(test)`) and
    /// asserted by `theme_suite`. The flames themselves draw from `FireStyle`,
    /// whose arcs are operator-selectable — keeping this field means the palette
    /// still describes its own heat, and a theme-aware flame can adopt it without
    /// another struct change.
    #[allow(dead_code)]
    pub fire: &'static [Color],
    /// syntect (fenced code) theme name; resolved in `markdown::highlight_theme`.
    pub syntax: &'static str,
}

const fn c_rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The original Officina scheme — gold on GitHub-dark.
pub static VITRIOL: Theme = Theme {
    name: "vitriol",
    bg: c_rgb(0x0D, 0x11, 0x17),
    panel: c_rgb(0x16, 0x1B, 0x22),
    user_bg: c_rgb(0x13, 0x1A, 0x22),
    border_dim: c_rgb(0x21, 0x26, 0x2D),
    green: c_rgb(0x3F, 0xB9, 0x50),
    gold: c_rgb(0xFF, 0xD7, 0x00),
    cyan: c_rgb(0x00, 0xFF, 0xFF),
    orange: c_rgb(0xFF, 0xA0, 0x00),
    red: c_rgb(0xF8, 0x51, 0x49),
    text: c_rgb(0xE0, 0xE0, 0xE0),
    muted: c_rgb(0x8B, 0x94, 0x9E),
    faint: c_rgb(0x6E, 0x76, 0x81),
    violet: c_rgb(0xB2, 0x94, 0xBB),
    cold_blue: c_rgb(0x2E, 0x5F, 0xA3),
    light_yellow: c_rgb(0xFF, 0xE0, 0x66),
    deep_red: c_rgb(0x8A, 0x15, 0x15),
    dark_teal: c_rgb(0x0B, 0x5E, 0x4C),
    mercury: c_rgb(0x55, 0x60, 0x6E),
    silver: c_rgb(0xC0, 0xC7, 0xCF),
    watermark: c_rgb(0x2A, 0x3A, 0x52),
    white: c_rgb(0xFF, 0xFF, 0xFF),
    fire: &[
        c_rgb(0x2E, 0x5F, 0xA3),
        c_rgb(0x00, 0xFF, 0xFF),
        c_rgb(0xFF, 0xD7, 0x00),
        c_rgb(0xFF, 0xA0, 0x00),
        c_rgb(0xF8, 0x51, 0x49),
    ],
    syntax: "base16-ocean.dark",
};

/// Kanagawa Wave ("Aether") — sumi ink ground, carp yellow accent.
///
/// Values taken from `themes/kanagawa-aetherskiff/vscode/.../kanagawa-wave-color-theme.json`
/// so the TUI matches the editor and the Plasma theme exactly.
pub static AETHER: Theme = Theme {
    name: "aether",
    bg: c_rgb(0x1F, 0x1F, 0x28),         // sumiInk1
    panel: c_rgb(0x16, 0x16, 0x1D),      // sumiInk0 (statusBar/sideBar)
    user_bg: c_rgb(0x24, 0x24, 0x2E),    // half-step toward sumiInk2
    border_dim: c_rgb(0x2A, 0x2A, 0x37), // sumiInk2
    green: c_rgb(0x98, 0xBB, 0x6C),      // springGreen (strings)
    gold: c_rgb(0xE6, 0xC3, 0x84),       // carpYellow — the accent, NOT gold
    cyan: c_rgb(0x7F, 0xB4, 0xCA),       // springBlue (streaming)
    orange: c_rgb(0xFF, 0xA0, 0x66),     // surimiOrange (warnings)
    red: c_rgb(0xE8, 0x24, 0x24),        // surfRed (critical)
    text: c_rgb(0xDC, 0xD7, 0xBA),       // fujiWhite
    muted: c_rgb(0xC8, 0xC0, 0x93),      // oldWhite — readable secondary
    faint: c_rgb(0x72, 0x71, 0x69),      // katanaGray — comments only
    violet: c_rgb(0x95, 0x7F, 0xB8),     // waveViolet (keywords)
    cold_blue: c_rgb(0x2D, 0x4F, 0x67),  // waveBlue2
    light_yellow: c_rgb(0xE6, 0xC3, 0x84),
    deep_red: c_rgb(0xC3, 0x40, 0x43),  // autumnRed
    dark_teal: c_rgb(0x22, 0x35, 0x3B), // waveBlue1-ish, dormant
    mercury: c_rgb(0x54, 0x54, 0x6C),
    silver: c_rgb(0x9C, 0xAB, 0xCA), // boatYellow2-adjacent pale blue
    watermark: c_rgb(0x2A, 0x2A, 0x37),
    white: c_rgb(0xFF, 0xF5, 0xD6),
    fire: &[
        c_rgb(0x2D, 0x4F, 0x67), // waves at rest
        c_rgb(0x95, 0x7F, 0xB8), // wave violet
        c_rgb(0xD2, 0x7E, 0x99), // sakura
        c_rgb(0xFF, 0x9E, 0x3B), // ronin yellow
        c_rgb(0xE6, 0xC3, 0x84), // carp — the hot end
    ],
    syntax: "kanagawa-wave",
};

/// Alkahest — Alka Officina's neon alchemy: same GitHub-dark ground, electric
/// accents (`themes/alka-officina`). Deliberately distinct from VITRIOL: the
/// brand colour is swirl yellow, not gold.
pub static ALKAHEST: Theme = Theme {
    name: "alkahest",
    bg: c_rgb(0x0D, 0x11, 0x17),
    panel: c_rgb(0x16, 0x1B, 0x22),
    user_bg: c_rgb(0x13, 0x1A, 0x22),
    border_dim: c_rgb(0x21, 0x26, 0x2D),
    green: c_rgb(0x39, 0xFF, 0x14),  // neon green
    gold: c_rgb(0xDF, 0xFF, 0x00),   // swirl yellow (brand)
    cyan: c_rgb(0x44, 0xAA, 0xFF),   // electric blue
    orange: c_rgb(0xFF, 0xD7, 0x00), // bright yellow (kitty color11)
    red: c_rgb(0xFF, 0x44, 0x44),
    text: c_rgb(0xE0, 0xE0, 0xE0),
    muted: c_rgb(0x8B, 0x94, 0x9E),
    faint: c_rgb(0x6E, 0x76, 0x81),
    violet: c_rgb(0xFF, 0x00, 0xFF), // magenta
    cold_blue: c_rgb(0x2E, 0x5F, 0xA3),
    light_yellow: c_rgb(0xFF, 0xF5, 0x66),
    deep_red: c_rgb(0x8A, 0x15, 0x15),
    dark_teal: c_rgb(0x0B, 0x5E, 0x4C),
    mercury: c_rgb(0x55, 0x60, 0x6E),
    silver: c_rgb(0xC0, 0xC7, 0xCF),
    watermark: c_rgb(0x1E, 0x2A, 0x1E),
    white: c_rgb(0xFF, 0xFF, 0xFF),
    fire: &[
        c_rgb(0x2E, 0x5F, 0xA3),
        c_rgb(0x44, 0xAA, 0xFF),
        c_rgb(0xDF, 0xFF, 0x00),
        c_rgb(0xFF, 0xD7, 0x00),
        c_rgb(0xFF, 0x79, 0xC6),
    ],
    syntax: "alka-neon",
};

/// Every theme, in menu order.
pub static ALL: [&Theme; 3] = [&VITRIOL, &AETHER, &ALKAHEST];

static CURRENT: RwLock<&'static Theme> = RwLock::new(&VITRIOL);

/// The theme in use.
pub fn current() -> &'static Theme {
    *CURRENT.read().unwrap_or_else(|e| e.into_inner())
}

/// Look a theme up by name (case-insensitive), including the two aliases the
/// asset directories use.
pub fn by_name(name: &str) -> Option<&'static Theme> {
    let n = name.trim().to_lowercase();
    let n = match n.as_str() {
        "kanagawa" | "aetherskiff" | "kanagawa-wave" => "aether",
        "alka" | "alka-officina" | "alkahest" => "alkahest",
        "vitriolum" | "default" => "vitriol",
        other => other,
    };
    ALL.iter().copied().find(|t| t.name == n)
}

/// Parse a persisted theme name: trim, lowercase, ignore empties.
pub fn parse_theme_name(text: &str) -> Option<String> {
    let name = text.trim().to_ascii_lowercase();
    (!name.is_empty()).then_some(name)
}

/// The palette's fire/tint gradient for the active theme.
#[allow(dead_code)] // see `Theme::fire`
pub fn fire() -> &'static [Color] {
    current().fire
}

/// Switch the active theme. Unknown names are refused (no silent fallback).
pub fn set(name: &str) -> Option<&'static Theme> {
    let theme = by_name(name)?;
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = theme;
    Some(theme)
}

// ── Accessors ────────────────────────────────────────────────────────────
//
// The uppercase names are intentional: they are the pre-2026-11-18 constants,
// and keeping them 1:1 made the migration a mechanical `CONST` → `CONST()`.
// No new code should need to add one; new colours belong in `Theme`.

#[allow(non_snake_case)]
pub fn BG() -> Color {
    current().bg
}
#[allow(non_snake_case)]
pub fn PANEL() -> Color {
    current().panel
}
#[allow(non_snake_case)]
pub fn USER_BG() -> Color {
    current().user_bg
}
#[allow(non_snake_case)]
pub fn BORDER_DIM() -> Color {
    current().border_dim
}
#[allow(non_snake_case)]
pub fn GREEN() -> Color {
    current().green
}
#[allow(non_snake_case)]
pub fn GOLD() -> Color {
    current().gold
}
#[allow(non_snake_case)]
pub fn CYAN() -> Color {
    current().cyan
}
#[allow(non_snake_case)]
pub fn ORANGE() -> Color {
    current().orange
}
#[allow(non_snake_case)]
pub fn RED() -> Color {
    current().red
}
#[allow(non_snake_case)]
pub fn TEXT() -> Color {
    current().text
}
#[allow(non_snake_case)]
pub fn MUTED() -> Color {
    current().muted
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; asserted by theme_suite
pub fn FAINT() -> Color {
    current().faint
}
#[allow(non_snake_case)]
pub fn VIOLET() -> Color {
    current().violet
}
#[allow(non_snake_case)]
pub fn COLD_BLUE() -> Color {
    current().cold_blue
}
#[allow(non_snake_case)]
pub fn LIGHT_YELLOW() -> Color {
    current().light_yellow
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; asserted by theme_suite
pub fn DEEP_RED() -> Color {
    current().deep_red
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; asserted by theme_suite
pub fn DARK_TEAL() -> Color {
    current().dark_teal
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; asserted by theme_suite
pub fn MERCURY() -> Color {
    current().mercury
}
#[allow(non_snake_case)]
pub fn SILVER() -> Color {
    current().silver
}
#[allow(non_snake_case)]
pub fn WATERMARK() -> Color {
    current().watermark
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; asserted by theme_suite
pub fn WHITE() -> Color {
    current().white
}

// ── Alchemical glyphs (verified: Unicode UCD nameslist, U+1F700 block) ──
// Font coverage is spotty beyond the four elements; fallbacks are one-line
// swaps here. See .opencode/plans/officina-tui-phase3-vitriolum-2026-09-02.md

/// The brand: ALCHEMICAL SYMBOL FOR VITRIOL (U+1F716). Fallback: ◈
pub const GLYPH_VITRIOL: &str = "🜖";
/// The living process: FIRE (U+1F702) — proven coverage (vitriol-tui).
pub const GLYPH_FIRE: &str = "🜂";
/// Compaction/distillation: ALEMBIC U+2697 (Misc Symbols, wide support).
pub const GLYPH_ALEMBIC: &str = "⚗";
/// Diagnostics: SULFUR (U+1F70D) — vitriolic. Fallback: ⚠
pub const GLYPH_SULFUR: &str = "🜍";
/// The AI correspondent: PHILOSOPHERS SULFUR (U+1F70E) — the stone's soul
/// (owner request 2026-09-02; no dedicated Philosopher's Stone codepoint
/// exists in Unicode 16 — verified by name scan). Fallback: 🜀 Quintessence.
pub const GLYPH_AI: &str = "🜎";
/// The human correspondent: MERCURY() ☿ (U+263F) — wide font coverage.
pub const GLYPH_USER: &str = "☿";
/// Tool running: CRUCIBLE (U+1F765) — matter being worked. Fallback: ⚙
pub const GLYPH_CRUCIBLE: &str = "🝥";
/// Reserved: QUINTESSENCE (U+1F700) — scratchpad label, when wired.
pub const GLYPH_QUINTESSENCE: &str = "🜀";

// ── Elemental glyphs (proven coverage via vitriol-tui) ──────────────────
pub const GLYPH_HERM: &str = "🜄"; // water, the sealed flask of memory

/// Neutral style for body text.
pub fn text() -> Style {
    Style::new().fg(TEXT()).bg(BG())
}

/// Style for muted / secondary text.
pub fn muted() -> Style {
    Style::new().fg(MUTED()).bg(BG())
}

/// Style for a panel title: green on the background.
pub fn title() -> Style {
    Style::new()
        .fg(GREEN())
        .bg(BG())
        .add_modifier(Modifier::BOLD)
}

/// Style for the top-level banner: gold, bold.
pub fn banner() -> Style {
    Style::new()
        .fg(GOLD())
        .bg(BG())
        .add_modifier(Modifier::BOLD)
}

/// Muted gold for non-port (logical-layer) accents.
#[allow(dead_code)] // kept: the notice strip replaced its only caller (2026-11-16)
pub fn gold_muted() -> Style {
    Style::new().fg(GOLD()).bg(BG())
}

/// Style for a live/streaming value (solvent cyan).
pub fn live() -> Style {
    Style::new().fg(CYAN()).bg(BG())
}

/// Informational emphasis.
pub fn info() -> Style {
    Style::new().fg(LIGHT_YELLOW()).bg(BG())
}

/// Substrate/critical emphasis.
pub fn crit() -> Style {
    Style::new().fg(RED()).bg(BG())
}

// ── Braille ramps ────────────────────────────────────────────────────────

/// A per-gauge semantic color ramp for braille gradient gauges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg(test)]
pub enum BrailleRamp {
    /// Memory capacity: white → light yellow → orange → red → deep red.
    Capacity,
    /// Compute activity: dormant teal → green → cyan at peak.
    Activity,
    /// Temperature: cold blue → cyan → gold → orange → red.
    Heat,
    /// Clock pulse: mercury silver → cyan.
    Pulse,
    /// Power draw: green → gold → over-limit red.
    Power,
    /// Decode velocity: red (slow) → gold → green (fast).
    Velocity,
}

#[cfg(test)]
impl BrailleRamp {
    /// Gradient stops, low (t=0) to high (t=1), in the ACTIVE theme.
    ///
    /// Owned: the stops are palette lookups now, so a `&'static` slice of them
    /// cannot exist (2026-11-18). Two-to-five colours per gauge.
    pub fn stops(self) -> Vec<Color> {
        match self {
            BrailleRamp::Capacity => {
                vec![current().white, LIGHT_YELLOW(), ORANGE(), RED(), DEEP_RED()]
            }
            BrailleRamp::Activity => vec![DARK_TEAL(), GREEN(), CYAN()],
            // The heat gauge and the flames share the theme's own gradient, so a
            // palette swap moves both together (identical values for `vitriol`).
            BrailleRamp::Heat => fire().to_vec(),
            BrailleRamp::Pulse => vec![MERCURY(), CYAN()],
            BrailleRamp::Power => vec![GREEN(), GOLD(), RED()],
            BrailleRamp::Velocity => vec![RED(), GOLD(), GREEN()],
        }
    }

    /// True when the ramp's high end is already a danger color.
    pub fn signals_danger(self) -> bool {
        matches!(
            self,
            BrailleRamp::Capacity | BrailleRamp::Heat | BrailleRamp::Power
        )
    }

    /// Ramp color at fraction `t` in `[0, 1]`.
    pub fn color(self, t: f64) -> Color {
        let stops = self.stops();
        ramp_color(&stops, t)
    }
}

/// Piecewise lerp across `stops` by fraction `t` in `[0, 1]`.
pub fn ramp_color(stops: &[Color], t: f64) -> Color {
    if stops.is_empty() {
        return MUTED();
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let t = t.clamp(0.0, 1.0);
    let scaled = t * (stops.len() - 1) as f64;
    let idx = (scaled.floor() as usize).min(stops.len() - 2);
    let frac = scaled - idx as f64;
    lerp_color(stops[idx], stops[idx + 1], frac)
}

/// Linear RGB interpolation between two colors.
pub fn lerp_color(a: Color, b: Color, t: f64) -> Color {
    let (ar, ag, ab) = rgb(a);
    let (br, bg, bb) = rgb(b);
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(
        (ar as f64 + (br as f64 - ar as f64) * t).round() as u8,
        (ag as f64 + (bg as f64 - ag as f64) * t).round() as u8,
        (ab as f64 + (bb as f64 - ab as f64) * t).round() as u8,
    )
}

/// Split a Color into RGB channels; non-RGB colors fall back to MUTED()'s values.
fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0x8B, 0x94, 0x9E),
    }
}

/// Value-text style for a gauge: bold orange past the warn threshold unless
/// the ramp already signals danger at its high end.
#[cfg(test)]
pub fn gauge_value_style(ramp: BrailleRamp, ratio: f64) -> Style {
    if ratio > 0.8 && !ramp.signals_danger() {
        Style::new()
            .fg(ORANGE())
            .bg(BG())
            .add_modifier(Modifier::BOLD)
    } else {
        live()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_stops_nonempty_and_monotonic() {
        for ramp in [
            BrailleRamp::Capacity,
            BrailleRamp::Activity,
            BrailleRamp::Heat,
            BrailleRamp::Pulse,
            BrailleRamp::Power,
            BrailleRamp::Velocity,
        ] {
            assert!(!ramp.stops().is_empty());
        }
    }

    #[test]
    fn ramp_color_endpoints() {
        assert_eq!(BrailleRamp::Pulse.color(0.0), MERCURY());
        assert_eq!(BrailleRamp::Pulse.color(1.0), CYAN());
        assert_eq!(BrailleRamp::Heat.color(0.0), COLD_BLUE());
        assert_eq!(BrailleRamp::Heat.color(1.0), RED());
        assert_eq!(BrailleRamp::Capacity.color(1.0), DEEP_RED());
        assert_eq!(BrailleRamp::Power.color(0.0), GREEN());
        assert_eq!(BrailleRamp::Power.color(1.0), RED());
    }

    #[test]
    fn velocity_ramp_low_red_high_green() {
        assert_eq!(BrailleRamp::Velocity.color(0.0), RED());
        assert_eq!(BrailleRamp::Velocity.color(1.0), GREEN());
        assert!(!BrailleRamp::Velocity.signals_danger());
    }

    #[test]
    fn ramp_color_clamps_out_of_range_t() {
        assert_eq!(BrailleRamp::Pulse.color(-1.0), MERCURY());
        assert_eq!(BrailleRamp::Pulse.color(2.0), CYAN());
    }

    #[test]
    fn ramp_color_midpoint_multi_stop() {
        let mid = BrailleRamp::Power.color(0.5);
        assert_eq!(mid, GOLD());
    }

    #[test]
    fn lerp_color_endpoints_and_mid() {
        assert_eq!(lerp_color(GREEN(), CYAN(), 0.0), GREEN());
        assert_eq!(lerp_color(GREEN(), CYAN(), 1.0), CYAN());
        assert_eq!(
            lerp_color(GREEN(), CYAN(), 0.5),
            Color::Rgb(0x20, 0xDC, 0xA8)
        );
    }

    #[test]
    fn lerp_color_clamps_t() {
        assert_eq!(lerp_color(GREEN(), CYAN(), -0.5), GREEN());
        assert_eq!(lerp_color(GREEN(), CYAN(), 1.5), CYAN());
    }

    #[test]
    fn signals_danger_only_for_red_ended_ramps() {
        assert!(BrailleRamp::Capacity.signals_danger());
        assert!(BrailleRamp::Heat.signals_danger());
        assert!(BrailleRamp::Power.signals_danger());
        assert!(!BrailleRamp::Activity.signals_danger());
        assert!(!BrailleRamp::Pulse.signals_danger());
    }

    #[test]
    fn warn_style_only_when_not_signalled() {
        let pulse_over = gauge_value_style(BrailleRamp::Pulse, 0.9);
        assert_eq!(pulse_over.fg, Some(ORANGE()));
        assert!(pulse_over.add_modifier.contains(Modifier::BOLD));
        let pulse_under = gauge_value_style(BrailleRamp::Pulse, 0.5);
        assert_eq!(pulse_under.fg, Some(CYAN()));

        let power_over = gauge_value_style(BrailleRamp::Power, 0.9);
        assert_eq!(power_over.fg, Some(CYAN()));
        let capacity_over = gauge_value_style(BrailleRamp::Capacity, 0.9);
        assert_eq!(capacity_over.fg, Some(CYAN()));
    }
}

#[cfg(test)]
mod theme_suite {
    use super::*;

    /// Every theme must define every role. A field left at `Reset` would render
    /// as the terminal's default colour — the exact "half-themed" failure this
    /// suite exists to catch.
    #[test]
    fn every_theme_defines_every_role() {
        for t in ALL {
            assert!(!t.name.is_empty());
            let colours = [
                t.bg,
                t.panel,
                t.user_bg,
                t.border_dim,
                t.green,
                t.gold,
                t.cyan,
                t.orange,
                t.red,
                t.text,
                t.muted,
                t.faint,
                t.violet,
                t.cold_blue,
                t.light_yellow,
                t.deep_red,
                t.dark_teal,
                t.mercury,
                t.silver,
                t.watermark,
                t.white,
            ];
            for (i, c) in colours.iter().enumerate() {
                assert_ne!(*c, Color::Reset, "{}: role {i} is unset", t.name);
                // Index 0 IS the background; every OTHER role must differ from it.
                if i > 0 {
                    assert_ne!(*c, t.bg, "{}: role {i} equals the background", t.name);
                }
            }
            assert!(!t.fire.is_empty(), "{}: no fire ramp", t.name);
            assert!(!t.syntax.is_empty(), "{}: no syntect theme", t.name);
        }
    }

    fn lum(c: Color) -> f64 {
        let (r, g, b) = match c {
            Color::Rgb(r, g, b) => (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0),
            _ => (0.0, 0.0, 0.0),
        };
        let f = |v: f64| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
    }

    fn contrast(a: Color, b: Color) -> f64 {
        let (la, lb) = (lum(a), lum(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// A theme that cannot be read is not a theme. Text 7:1, secondary 3.5:1.
    #[test]
    fn every_theme_is_readable() {
        for t in ALL {
            let body = contrast(t.text, t.bg);
            let secondary = contrast(t.muted, t.bg);
            assert!(body >= 7.0, "{}: body contrast {body:.1} < 7", t.name);
            assert!(
                secondary >= 3.5,
                "{}: secondary contrast {secondary:.1} < 3.5",
                t.name
            );
            // Panels must stay distinguishable from the ground, or borders vanish.
            assert_ne!(t.panel, t.bg, "{}: panel equals bg", t.name);
        }
    }

    #[test]
    fn names_and_aliases_resolve() {
        assert_eq!(by_name("vitriol").map(|t| t.name), Some("vitriol"));
        assert_eq!(by_name("AETHER").map(|t| t.name), Some("aether"));
        assert_eq!(by_name("kanagawa").map(|t| t.name), Some("aether"));
        assert_eq!(by_name("aetherskiff").map(|t| t.name), Some("aether"));
        assert_eq!(by_name("alkahest").map(|t| t.name), Some("alkahest"));
        assert_eq!(by_name("alka-officina").map(|t| t.name), Some("alkahest"));
        assert!(by_name("definitely-not-a-theme").is_none());
        assert!(by_name("").is_none());
    }

    #[test]
    fn persisted_names_parse_leniently() {
        assert_eq!(parse_theme_name("  Aether \n"), Some("aether".into()));
        assert_eq!(parse_theme_name("\n\t "), None);
    }

    /// Every fire ramp must be a gradient that gets WARMER toward the hot end.
    ///
    /// Luminance is the wrong measure: a heat ramp peaks in the middle (gold,
    /// carp) and darkens into red by design. What must hold is that the hot end
    /// is warmer in hue than the embers — `R − B` rising — and that no two stops
    /// are the same colour.
    #[test]
    fn fire_ramps_warm_toward_the_hot_end() {
        fn warmth(c: Color) -> i32 {
            let (r, _, b) = rgb(c);
            r as i32 - b as i32
        }
        for t in ALL {
            assert!(t.fire.len() >= 2, "{}: ramp too short", t.name);
            let first = t.fire[0];
            let last = *t.fire.last().unwrap();
            assert_ne!(first, last, "{}: ramp has no range", t.name);
            assert!(
                warmth(last) >= warmth(first),
                "{}: the hot end is cooler than the embers ({:?} → {:?})",
                t.name,
                first,
                last
            );
            for pair in t.fire.windows(2) {
                assert_ne!(pair[0], pair[1], "{}: ramp repeats a colour", t.name);
            }
        }
    }

    /// VITRIOL is the original palette and must never drift: these literals are
    /// the pre-2026-11-18 constants, pinned so a future edit to the palette is a
    /// deliberate act rather than a side effect of another change.
    #[test]
    fn vitriol_is_byte_identical_to_the_original_palette() {
        use Color::Rgb;
        assert_eq!(VITRIOL.bg, Rgb(0x0D, 0x11, 0x17));
        assert_eq!(VITRIOL.panel, Rgb(0x16, 0x1B, 0x22));
        assert_eq!(VITRIOL.user_bg, Rgb(0x13, 0x1A, 0x22));
        assert_eq!(VITRIOL.border_dim, Rgb(0x21, 0x26, 0x2D));
        assert_eq!(VITRIOL.green, Rgb(0x3F, 0xB9, 0x50));
        assert_eq!(VITRIOL.gold, Rgb(0xFF, 0xD7, 0x00));
        assert_eq!(VITRIOL.cyan, Rgb(0x00, 0xFF, 0xFF));
        assert_eq!(VITRIOL.orange, Rgb(0xFF, 0xA0, 0x00));
        assert_eq!(VITRIOL.red, Rgb(0xF8, 0x51, 0x49));
        assert_eq!(VITRIOL.text, Rgb(0xE0, 0xE0, 0xE0));
        assert_eq!(VITRIOL.muted, Rgb(0x8B, 0x94, 0x9E));
        assert_eq!(VITRIOL.violet, Rgb(0xB2, 0x94, 0xBB));
        assert_eq!(VITRIOL.cold_blue, Rgb(0x2E, 0x5F, 0xA3));
        assert_eq!(VITRIOL.light_yellow, Rgb(0xFF, 0xE0, 0x66));
        assert_eq!(VITRIOL.deep_red, Rgb(0x8A, 0x15, 0x15));
        assert_eq!(VITRIOL.dark_teal, Rgb(0x0B, 0x5E, 0x4C));
        assert_eq!(VITRIOL.mercury, Rgb(0x55, 0x60, 0x6E));
        assert_eq!(VITRIOL.silver, Rgb(0xC0, 0xC7, 0xCF));
        assert_eq!(VITRIOL.watermark, Rgb(0x2A, 0x3A, 0x52));
        assert_eq!(VITRIOL.white, Rgb(0xFF, 0xFF, 0xFF));
        assert_eq!(VITRIOL.syntax, "base16-ocean.dark");
    }

    /// No colour literals outside this module's palette table.
    ///
    /// Every renderer must take its colours from the active theme; a literal
    /// `Color::Rgb(..)` anywhere else is a spot that cannot follow a theme
    /// switch (2026-11-18: ANSI text, the watermark tint and the Emerald fire arc
    /// all bypassed the palette). Source scan, like the const-read gate, because
    /// the failure mode is invisible in a screenshot of the default theme.
    #[test]
    fn no_colour_literals_outside_the_palette() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders: Vec<String> = Vec::new();
        for entry in walk(&root) {
            if entry.file_name().is_some_and(|n| n == "theme.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            // Test modules may use literals on purpose (they assert exact
            // colours); the gate is about production renderers.
            let production = match text.find("#[cfg(test)]") {
                Some(cut) => &text[..cut],
                None => text.as_str(),
            };
            for (i, line) in production.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                // Only LITERAL colours are the problem. `Color::Rgb(r, g, b)`
                // built from variables is legitimate: the ANSI 256-colour cube
                // carries exact RGB from the extension, and the watermark/syntect
                // build theirs by mixing palette entries.
                let banned = [
                    "Color::Rgb(0x",
                    "Color::Rgb(0X",
                    "Color::Rgb(0,",
                    "Color::Rgb(1,",
                    "Color::Rgb(2,",
                    "Color::Rgb(3,",
                    "Color::Rgb(4,",
                    "Color::Rgb(5,",
                    "Color::Rgb(6,",
                    "Color::Rgb(7,",
                    "Color::Rgb(8,",
                    "Color::Rgb(9,",
                ];
                if banned.iter().any(|b| code.contains(b)) {
                    offenders.push(format!("{}:{}", entry.display(), i + 1));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "colour literals outside theme.rs (take them from the palette): {offenders:#?}"
        );
    }

    /// The whole point: the three themes are actually different.
    #[test]
    fn themes_are_distinct() {
        assert_ne!(VITRIOL.bg, AETHER.bg);
        assert_ne!(VITRIOL.gold, AETHER.gold); // gold vs carp
        assert_ne!(VITRIOL.gold, ALKAHEST.gold); // gold vs swirl
        assert_ne!(VITRIOL.green, ALKAHEST.green); // calm vs neon
        assert_ne!(AETHER.text, ALKAHEST.text);
    }

    /// No call site may still read a colour as a `const`: the failure mode is a
    /// silently unthemed widget that no rendering test would notice.
    #[test]
    fn no_uppercase_theme_access_without_parens() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders: Vec<String> = Vec::new();
        for entry in walk(&root) {
            let text = std::fs::read_to_string(&entry).unwrap_or_default();
            for (line_no, line) in text.lines().enumerate() {
                let mut rest = line;
                while let Some(pos) = rest.find("theme::") {
                    rest = &rest[pos + "theme::".len()..];
                    let ident: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_uppercase() || *c == '_' || c.is_ascii_digit())
                        .collect();
                    if ident.is_empty() {
                        continue;
                    }
                    // Strings (glyphs) and the theme list are not colours.
                    if ident.starts_with("GLYPH_") || ident == "ALL" {
                        continue;
                    }
                    let after = &rest[ident.len()..];
                    if !after.starts_with('(') {
                        offenders.push(format!(
                            "{}:{}: theme::{ident}",
                            entry.display(),
                            line_no + 1
                        ));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "unthemed const reads: {offenders:#?}");
    }

    fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        out
    }
}
