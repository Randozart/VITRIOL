//! VITRIOL dashboard themes — **vitriol**, **aether**, **alkahest**.
//!
//! 2026-11-18: the palette became DATA, the same shape Officina's chat TUI uses.
//! Values come from `officina/theme/palette.json` — the single source of truth
//! shared by the chat TUI, this dashboard, the Vitriolum JS palette and the
//! generated desktop themes — and tests assert equality, so the two TUIs can
//! never drift apart again (the old `ORANGE #FF5F1F` was exactly that).
//!
//! Accessors keep the historical constant names (`theme::GOLD()`), which made the
//! migration a mechanical rename rather than a rewiring of every render call.

use ratatui::style::{Color, Modifier, Style};
use std::sync::RwLock;

/// One complete colour scheme.
/// Fields the dashboard does not draw yet are kept: the struct mirrors
/// `officina/theme/palette.json` role for role, so a missing field would mean a
/// palette that cannot be compared across surfaces.
#[allow(dead_code)]
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
    /// Cold to hot gradient for heat gauges; index 0 is the cold end.
    pub fire: &'static [Color],
    /// Reserved for a syntax-highlighted surface; kept for parity with the chat TUI.
    #[allow(dead_code)]
    pub syntax: &'static str,
}

const fn c_rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The original palette — gold on GitHub-dark.
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
    faint: c_rgb(0x5F, 0x66, 0x72),
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

/// Kanagawa Wave — sumi ink ground, carp-yellow accent.
pub static AETHER: Theme = Theme {
    name: "aether",
    bg: c_rgb(0x1F, 0x1F, 0x28),
    panel: c_rgb(0x16, 0x16, 0x1D),
    user_bg: c_rgb(0x24, 0x24, 0x2E),
    border_dim: c_rgb(0x2A, 0x2A, 0x37),
    green: c_rgb(0x98, 0xBB, 0x6C),
    gold: c_rgb(0xE6, 0xC3, 0x84),
    cyan: c_rgb(0x7F, 0xB4, 0xCA),
    orange: c_rgb(0xFF, 0xA0, 0x66),
    red: c_rgb(0xE8, 0x24, 0x24),
    text: c_rgb(0xDC, 0xD7, 0xBA),
    muted: c_rgb(0xC8, 0xC0, 0x93),
    faint: c_rgb(0x72, 0x71, 0x69),
    violet: c_rgb(0x95, 0x7F, 0xB8),
    cold_blue: c_rgb(0x2D, 0x4F, 0x67),
    light_yellow: c_rgb(0xE6, 0xC3, 0x84),
    deep_red: c_rgb(0xC3, 0x40, 0x43),
    dark_teal: c_rgb(0x22, 0x35, 0x3B),
    mercury: c_rgb(0x54, 0x54, 0x6C),
    silver: c_rgb(0x9C, 0xAB, 0xCA),
    watermark: c_rgb(0x2A, 0x2A, 0x37),
    white: c_rgb(0xFF, 0xF5, 0xD6),
    fire: &[
        c_rgb(0x2D, 0x4F, 0x67),
        c_rgb(0x95, 0x7F, 0xB8),
        c_rgb(0xD2, 0x7E, 0x99),
        c_rgb(0xFF, 0x9E, 0x3B),
        c_rgb(0xE6, 0xC3, 0x84),
    ],
    syntax: "kanagawa-wave",
};

/// Alka Officina's neon alchemy on the same dark ground.
pub static ALKAHEST: Theme = Theme {
    name: "alkahest",
    bg: c_rgb(0x0D, 0x11, 0x17),
    panel: c_rgb(0x16, 0x1B, 0x22),
    user_bg: c_rgb(0x13, 0x1A, 0x22),
    border_dim: c_rgb(0x21, 0x26, 0x2D),
    green: c_rgb(0x39, 0xFF, 0x14),
    gold: c_rgb(0xDF, 0xFF, 0x00),
    cyan: c_rgb(0x44, 0xAA, 0xFF),
    orange: c_rgb(0xFF, 0xD7, 0x00),
    red: c_rgb(0xFF, 0x44, 0x44),
    text: c_rgb(0xE0, 0xE0, 0xE0),
    muted: c_rgb(0x8B, 0x94, 0x9E),
    faint: c_rgb(0x5F, 0x66, 0x72),
    violet: c_rgb(0xFF, 0x00, 0xFF),
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

/// Look a theme up by name (case-insensitive), with the asset-directory aliases.
pub fn by_name(name: &str) -> Option<&'static Theme> {
    let n = name.trim().to_lowercase();
    let n = match n.as_str() {
        "kanagawa" | "aetherskiff" | "kanagawa-wave" => "aether",
        "alka" | "alka-officina" => "alkahest",
        "vitriolum" | "default" => "vitriol",
        other => other,
    };
    ALL.iter().copied().find(|t| t.name == n)
}

/// Switch the active theme. Unknown names are refused (no silent fallback).
pub fn set(name: &str) -> Option<&'static Theme> {
    let theme = by_name(name)?;
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = theme;
    Some(theme)
}

/// Persisted dashboard theme: `~/.vitriol/tui-theme`.
///
/// Deliberately NOT the chat TUI's file (`~/.vitriol/officina/tui-theme`): the
/// dashboard is a different surface and may want a different palette. The shared
/// lever is `VITRIOL_TUI_THEME`, which both binaries honour.
pub fn theme_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        std::path::PathBuf::from(home)
            .join(".vitriol")
            .join("tui-theme"),
    )
}

/// `VITRIOL_TUI_THEME` then the saved file then the compiled default.
pub fn init_from_environment() -> &'static Theme {
    if let Ok(name) = std::env::var("VITRIOL_TUI_THEME") {
        if !name.trim().is_empty() {
            if let Some(t) = set(name.trim()) {
                return t;
            }
        }
    }
    if let Some(path) = theme_path() {
        if let Ok(text) = std::fs::read_to_string(path) {
            let name = text.trim().to_lowercase();
            if !name.is_empty() {
                if let Some(t) = set(&name) {
                    return t;
                }
            }
        }
    }
    current()
}

/// Remember the active theme for the next launch.
pub fn persist() -> bool {
    let Some(path) = theme_path() else {
        return false;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    std::fs::write(path, format!("{}\n", current().name)).is_ok()
}

/// Advance to the next theme in `ALL` and persist it.
pub fn cycle() -> &'static Theme {
    let idx = ALL
        .iter()
        .position(|t| t.name == current().name)
        .unwrap_or(0);
    let next = ALL[(idx + 1) % ALL.len()];
    let _ = set(next.name);
    let _ = persist();
    next
}

// Accessors: the uppercase names are the pre-2026-11-18 constants, kept 1:1 so
// the migration was mechanical. New colours belong in `Theme`.

#[allow(non_snake_case)]
pub fn BG() -> Color {
    current().bg
}
#[allow(non_snake_case)]
pub fn PANEL() -> Color {
    current().panel
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
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
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
pub fn FAINT() -> Color {
    current().faint
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
pub fn VIOLET() -> Color {
    current().violet
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
pub fn COLD_BLUE() -> Color {
    current().cold_blue
}
#[allow(non_snake_case)]
pub fn LIGHT_YELLOW() -> Color {
    current().light_yellow
}
#[allow(non_snake_case)]
pub fn DEEP_RED() -> Color {
    current().deep_red
}
#[allow(non_snake_case)]
pub fn DARK_TEAL() -> Color {
    current().dark_teal
}
#[allow(non_snake_case)]
pub fn MERCURY() -> Color {
    current().mercury
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
pub fn SILVER() -> Color {
    current().silver
}
#[allow(non_snake_case)]
#[allow(dead_code)] // palette role kept complete; pinned by the palette.json test
pub fn WATERMARK() -> Color {
    current().watermark
}
#[allow(non_snake_case)]
pub fn WHITE() -> Color {
    current().white
}

// ── Styles ───────────────────────────────────────────────────────────────

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

/// Style for the top-level VITRIOL banner: gold, bold.
pub fn banner() -> Style {
    Style::new()
        .fg(GOLD())
        .bg(BG())
        .add_modifier(Modifier::BOLD)
}

/// Muted gold for non-port (logical-layer) glyphs.
pub fn gold_muted() -> Style {
    Style::new().fg(GOLD()).bg(BG())
}

/// Border style for a service panel, coloured by liveness: green when up, red
/// when down, dim when unknown.
pub fn panel_border(up: bool) -> Style {
    let c = if up { GREEN() } else { RED() };
    Style::new().fg(c)
}

/// Live/streaming emphasis (the solvent cyan).
pub fn live() -> Style {
    Style::new().fg(CYAN()).bg(BG())
}

/// Informational emphasis.
pub fn info() -> Style {
    Style::new().fg(LIGHT_YELLOW()).bg(BG())
}

/// Warning emphasis (the antidote amber).
pub fn warn() -> Style {
    Style::new().fg(ORANGE()).bg(BG())
}

/// Gauge value text: bold amber past the warn threshold unless the ramp already
/// signals danger at its high end, else the live cyan.
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

/// Gauge label text: same threshold, muted otherwise.
pub fn gauge_label_style(ramp: BrailleRamp, ratio: f64) -> Style {
    if ratio > 0.8 && !ramp.signals_danger() {
        Style::new()
            .fg(ORANGE())
            .bg(BG())
            .add_modifier(Modifier::BOLD)
    } else {
        muted()
    }
}

// ── Glyphs (identity, not colour: they do not change with the palette) ───

/// Generation glyph — fire.
pub const GLYPH_GEN: &str = "🜂";
/// Memory glyph — water.
pub const GLYPH_HERM: &str = "🜄";
/// Embed glyph — air, spirit.
pub const GLYPH_EMBED: &str = "🜁";
/// GPU glyph — earth, silicon matter.
pub const GLYPH_GPU: &str = "🜃";

// ── Braille ramps ────────────────────────────────────────────────────────

/// A per-gauge semantic colour ramp for braille gradient gauges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrailleRamp {
    /// Memory capacity: white, light yellow, orange, red, deep red.
    Capacity,
    /// Compute activity: dormant teal, green, cyan at peak.
    Activity,
    /// Temperature, thermal scale: cold blue, cyan, gold, orange, red.
    Heat,
    /// Clock pulse: mercury silver, cyan.
    Pulse,
    /// Power draw: green, gold, over-limit red.
    Power,
    /// Decode velocity: red (slow), gold, green (fast).
    Velocity,
}

impl BrailleRamp {
    /// Gradient stops, low (t=0) to high (t=1), in the ACTIVE theme.
    pub fn stops(self) -> Vec<Color> {
        match self {
            BrailleRamp::Capacity => vec![WHITE(), LIGHT_YELLOW(), ORANGE(), RED(), DEEP_RED()],
            BrailleRamp::Activity => vec![DARK_TEAL(), GREEN(), CYAN()],
            // The heat gauge shares the palette's own cold-to-hot gradient.
            BrailleRamp::Heat => fire().to_vec(),
            BrailleRamp::Pulse => vec![MERCURY(), CYAN()],
            BrailleRamp::Power => vec![GREEN(), GOLD(), RED()],
            BrailleRamp::Velocity => vec![RED(), GOLD(), GREEN()],
        }
    }

    /// True when the ramp's high end is already a danger colour.
    pub fn signals_danger(self) -> bool {
        matches!(
            self,
            BrailleRamp::Capacity | BrailleRamp::Heat | BrailleRamp::Power
        )
    }

    /// Ramp colour at fraction `t` in `[0, 1]`.
    pub fn color(self, t: f64) -> Color {
        let stops = self.stops();
        ramp_color(&stops, t)
    }
}

/// The palette's fire/heat gradient for the active theme.
pub fn fire() -> &'static [Color] {
    current().fire
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

/// Linear RGB interpolation between two colours.
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

/// Split a Colour into RGB channels; non-RGB colours fall back to MUTED's values.
fn rgb(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0x8B, 0x94, 0x9E),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dashboard palette must equal `officina/theme/palette.json` — the same
    /// file the chat TUI and the JS palette are pinned to.
    #[test]
    fn palette_matches_the_shared_file() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("officina")
            .join("theme")
            .join("palette.json");
        let text = std::fs::read_to_string(&path).expect("palette.json");
        let doc: serde_json::Value = serde_json::from_str(&text).expect("palette.json parses");
        let palettes = doc["palettes"].as_object().expect("palettes");
        for theme in ALL {
            let want = palettes
                .get(theme.name)
                .unwrap_or_else(|| panic!("palette.json has no '{}'", theme.name));
            let roles: [(&str, Color); 21] = [
                ("bg", theme.bg),
                ("panel", theme.panel),
                ("user_bg", theme.user_bg),
                ("border_dim", theme.border_dim),
                ("green", theme.green),
                ("gold", theme.gold),
                ("cyan", theme.cyan),
                ("orange", theme.orange),
                ("red", theme.red),
                ("text", theme.text),
                ("muted", theme.muted),
                ("faint", theme.faint),
                ("violet", theme.violet),
                ("cold_blue", theme.cold_blue),
                ("light_yellow", theme.light_yellow),
                ("deep_red", theme.deep_red),
                ("dark_teal", theme.dark_teal),
                ("mercury", theme.mercury),
                ("silver", theme.silver),
                ("watermark", theme.watermark),
                ("white", theme.white),
            ];
            for (role, value) in roles {
                let (r, g, b) = rgb(value);
                assert_eq!(
                    format!("#{r:02x}{g:02x}{b:02x}"),
                    want[role].as_str().unwrap_or_default().to_lowercase(),
                    "{}: {role} differs from palette.json",
                    theme.name
                );
            }
        }
    }

    /// Warning must not read as failure: the accent has to sit far from red.
    #[test]
    fn warning_and_critical_are_distinguishable() {
        for theme in ALL {
            let (or_, og, ob) = rgb(theme.orange);
            let (rr, rg, rb) = rgb(theme.red);
            assert_ne!(theme.orange, theme.red, "{}: orange equals red", theme.name);
            let distance = (or_ as i32 - rr as i32).abs()
                + (og as i32 - rg as i32).abs()
                + (ob as i32 - rb as i32).abs();
            assert!(
                distance > 60,
                "{}: warning and critical are {distance} apart (too close)",
                theme.name
            );
        }
    }

    #[test]
    fn names_and_profiles_resolve() {
        assert_eq!(by_name("aether").map(|t| t.name), Some("aether"));
        assert_eq!(by_name("KANAGAWA").map(|t| t.name), Some("aether"));
        assert_eq!(by_name("alkahest").map(|t| t.name), Some("alkahest"));
        assert_eq!(by_name("vitriolum").map(|t| t.name), Some("vitriol"));
        assert!(by_name("nope").is_none());
        assert_eq!(ALL.len(), 3);
    }

    #[test]
    fn ramps_are_gradients_from_the_active_palette() {
        for ramp in [
            BrailleRamp::Capacity,
            BrailleRamp::Activity,
            BrailleRamp::Heat,
            BrailleRamp::Pulse,
            BrailleRamp::Power,
            BrailleRamp::Velocity,
        ] {
            let stops = ramp.stops();
            assert!(stops.len() >= 2, "{ramp:?}");
            assert_ne!(stops[0], *stops.last().unwrap(), "{ramp:?}");
        }
    }
}
