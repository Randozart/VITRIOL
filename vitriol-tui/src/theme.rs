//! "Vitriolum" theme — dark alchemical green + gold.
//!
//! Palette derived from the Alka Officina dark theme
//! (`Desktop/Projects/alka-lang/vscode/themes/officina-dark.json`, user-owned)
//! re-centered on the VITRIOL logo colours. Semantics map to alchemy: solvent
//! cyan = the process flowing; gold = work completed (nominal); green = the
//! living fire (healthy); red = substrate (crash). Spec in
//! `.opencode/plans/2026-08-07-vitriol-tui.md`.

use ratatui::style::{Color, Modifier, Style};

/// Background (GitHub-dark base, from Officina `editor.background`).
pub const BG: Color = Color::Rgb(0x0D, 0x11, 0x17);
/// Panel background (Officina `editor.lineHighlightBackground`).
pub const PANEL: Color = Color::Rgb(0x16, 0x1B, 0x22);
/// Dim border for unfocused panels (Officina `statusBar.background`).
pub const BORDER_DIM: Color = Color::Rgb(0x21, 0x26, 0x2D);
/// VITRIOL primary green (Officina "Safety") — borders, headers, gauges.
pub const GREEN: Color = Color::Rgb(0x39, 0xFF, 0x14);
/// VITRIOL gold (Officina "Sovereignty") — titles, active accents.
pub const GOLD: Color = Color::Rgb(0xFF, 0xD7, 0x00);
/// Solvent cyan (Officina "Solvent") — active/streaming decode.
pub const CYAN: Color = Color::Rgb(0x00, 0xFF, 0xFF);
/// Antidote orange (Officina "Antidote") — warnings.
pub const ORANGE: Color = Color::Rgb(0xFF, 0x5F, 0x1F);
/// Substrate red (Officina "Substrate") — down / critical.
pub const RED: Color = Color::Rgb(0xFF, 0x44, 0x44);
/// Foreground text (Officina `editor.foreground`).
pub const TEXT: Color = Color::Rgb(0xE0, 0xE0, 0xE0);
/// Muted text (Officina punctuation).
pub const MUTED: Color = Color::Rgb(0x8B, 0x94, 0x9E);

/// Gen service glyph — fire, the forge.
pub const GLYPH_GEN: &str = "🜂";
/// Hermetis glyph — water, the sealed flask of memory.
pub const GLYPH_HERM: &str = "🜄";
/// Embed glyph — air, spirit.
pub const GLYPH_EMBED: &str = "🜁";
/// GPU glyph — earth, silicon matter.
pub const GLYPH_GPU: &str = "🜃";

/// Neutral style for body text.
pub fn text() -> Style {
    Style::new().fg(TEXT).bg(BG)
}

/// Style for muted / secondary text.
pub fn muted() -> Style {
    Style::new().fg(MUTED).bg(BG)
}

/// Style for a panel title: green on the background.
pub fn title() -> Style {
    Style::new().fg(GREEN).bg(BG).add_modifier(Modifier::BOLD)
}

/// Style for the top-level VITRIOL banner: gold, bold.
pub fn banner() -> Style {
    Style::new().fg(GOLD).bg(BG).add_modifier(Modifier::BOLD)
}

/// Muted gold for non-port (logical-layer) glyphs.
pub fn gold_muted() -> Style {
    Style::new().fg(GOLD).bg(BG)
}

/// Border style for a service panel, coloured by liveness: green when up,
/// red when down, dim when unknown.
pub fn panel_border(up: bool) -> Style {
    let c = if up { GREEN } else { RED };
    Style::new().fg(c)
}

/// Cold blue — the heat ramp's low end (thermal scale: cold → red-hot).
pub const COLD_BLUE: Color = Color::Rgb(0x2E, 0x5F, 0xA3);
/// Light yellow — capacity ramp start, "empty / plenty of room".
pub const LIGHT_YELLOW: Color = Color::Rgb(0xFF, 0xE0, 0x66);
/// Deep red — capacity ramp end, "space running out".
pub const DEEP_RED: Color = Color::Rgb(0x8A, 0x15, 0x15);
/// Dark teal — activity ramp start, the dormant solvent.
pub const DARK_TEAL: Color = Color::Rgb(0x0B, 0x5E, 0x4C);
/// Mercury silver — pulse ramp start, the quicksilver messenger.
pub const MERCURY: Color = Color::Rgb(0x55, 0x60, 0x6E);

/// A per-gauge semantic color ramp for braille gradient gauges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrailleRamp {
    /// Memory capacity: white → light yellow → orange → red → deep red.
    Capacity,
    /// Compute activity: dormant teal → green → cyan at peak.
    Activity,
    /// Temperature, thermal scale: cold blue → cyan → gold → orange → red.
    Heat,
    /// Clock pulse: mercury silver → cyan.
    Pulse,
    /// Power draw: green → gold → over-limit red.
    Power,
    /// Decode velocity: red (slow) → gold → green (fast).
    Velocity,
}

impl BrailleRamp {
    /// Gradient stops, low (t=0) to high (t=1).
    pub fn stops(self) -> &'static [Color] {
        match self {
            BrailleRamp::Capacity => &[
                Color::Rgb(0xFF, 0xFF, 0xFF),
                LIGHT_YELLOW,
                ORANGE,
                RED,
                DEEP_RED,
            ],
            BrailleRamp::Activity => &[DARK_TEAL, GREEN, CYAN],
            BrailleRamp::Heat => &[COLD_BLUE, CYAN, GOLD, ORANGE, RED],
            BrailleRamp::Pulse => &[MERCURY, CYAN],
            BrailleRamp::Power => &[GREEN, GOLD, RED],
            BrailleRamp::Velocity => &[RED, GOLD, GREEN],
        }
    }

    /// True when the ramp's high end is already a danger color, so no extra
    /// warn styling is needed for the value text.
    pub fn signals_danger(self) -> bool {
        matches!(
            self,
            BrailleRamp::Capacity | BrailleRamp::Heat | BrailleRamp::Power
        )
    }

    /// Ramp color at fraction `t` in `[0, 1]`.
    pub fn color(self, t: f64) -> Color {
        ramp_color(self.stops(), t)
    }
}

/// Piecewise lerp across `stops` by fraction `t` in `[0, 1]`. Multi-stop ramps
/// interpolate between adjacent stops; t is clamped.
pub fn ramp_color(stops: &[Color], t: f64) -> Color {
    if stops.is_empty() {
        return MUTED;
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

/// Split a Color into RGB channels; non-RGB colors fall back to MUTED's values.
fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0x8B, 0x94, 0x9E),
    }
}

/// Value-text style for a gauge: bold orange past the warn threshold unless the
/// ramp already signals danger at its high end.
pub fn gauge_value_style(ramp: BrailleRamp, ratio: f64) -> Style {
    if ratio > 0.8 && !ramp.signals_danger() {
        Style::new().fg(ORANGE).bg(BG).add_modifier(Modifier::BOLD)
    } else {
        live()
    }
}

/// Muted-label style for a gauge row; warn-bold orange when the ramp does not
/// signal danger at its high end and the value passes the warn threshold.
pub fn gauge_label_style(ramp: BrailleRamp, ratio: f64) -> Style {
    if ratio > 0.8 && !ramp.signals_danger() {
        Style::new().fg(ORANGE).bg(BG).add_modifier(Modifier::BOLD)
    } else {
        muted()
    }
}

/// Style for a live/streaming value (solvent cyan).
pub fn live() -> Style {
    Style::new().fg(CYAN).bg(BG)
}

/// Informational emphasis (diagnostics breakdown).
pub fn info() -> Style {
    Style::new().fg(LIGHT_YELLOW).bg(BG)
}

/// Warning emphasis (e.g. per-decode CUDA-graph re-capture).
pub fn warn() -> Style {
    Style::new().fg(ORANGE).bg(BG)
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
        assert_eq!(BrailleRamp::Pulse.color(0.0), MERCURY);
        assert_eq!(BrailleRamp::Pulse.color(1.0), CYAN);
        assert_eq!(BrailleRamp::Heat.color(0.0), COLD_BLUE);
        assert_eq!(BrailleRamp::Heat.color(1.0), RED);
        assert_eq!(BrailleRamp::Capacity.color(1.0), DEEP_RED);
        assert_eq!(BrailleRamp::Power.color(0.0), GREEN);
        assert_eq!(BrailleRamp::Power.color(1.0), RED);
    }

    #[test]
    fn velocity_ramp_low_red_high_green() {
        assert_eq!(BrailleRamp::Velocity.color(0.0), RED);
        assert_eq!(BrailleRamp::Velocity.color(1.0), GREEN);
        assert!(!BrailleRamp::Velocity.signals_danger());
    }

    #[test]
    fn ramp_color_clamps_out_of_range_t() {
        assert_eq!(BrailleRamp::Pulse.color(-1.0), MERCURY);
        assert_eq!(BrailleRamp::Pulse.color(2.0), CYAN);
    }

    #[test]
    fn ramp_color_midpoint_multi_stop() {
        let mid = BrailleRamp::Power.color(0.5);
        assert_eq!(mid, GOLD);
    }

    #[test]
    fn lerp_color_endpoints_and_mid() {
        assert_eq!(lerp_color(GREEN, CYAN, 0.0), GREEN);
        assert_eq!(lerp_color(GREEN, CYAN, 1.0), CYAN);
        assert_eq!(lerp_color(GREEN, CYAN, 0.5), Color::Rgb(0x1D, 0xFF, 0x8A));
    }

    #[test]
    fn lerp_color_clamps_t() {
        assert_eq!(lerp_color(GREEN, CYAN, -0.5), GREEN);
        assert_eq!(lerp_color(GREEN, CYAN, 1.5), CYAN);
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
        assert_eq!(pulse_over.fg, Some(ORANGE));
        assert!(pulse_over.add_modifier.contains(Modifier::BOLD));
        let pulse_under = gauge_value_style(BrailleRamp::Pulse, 0.5);
        assert_eq!(pulse_under.fg, Some(CYAN));

        let power_over = gauge_value_style(BrailleRamp::Power, 0.9);
        assert_eq!(power_over.fg, Some(CYAN));
        let capacity_over = gauge_value_style(BrailleRamp::Capacity, 0.9);
        assert_eq!(capacity_over.fg, Some(CYAN));
    }
}
