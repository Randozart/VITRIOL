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

/// Fill style for a btop-style gauge.
pub fn gauge_fill() -> Style {
    Style::new().fg(GREEN).bg(BG)
}

/// Fill style for a gauge nearing its limit (ratio above the warn threshold).
pub fn gauge_fill_warn() -> Style {
    Style::new().fg(ORANGE).bg(BG)
}

/// Style for a decode sparkline.
pub fn sparkline() -> Style {
    Style::new().fg(GREEN).bg(BG)
}

/// Style for a live/streaming value (solvent cyan).
pub fn live() -> Style {
    Style::new().fg(CYAN).bg(BG)
}
