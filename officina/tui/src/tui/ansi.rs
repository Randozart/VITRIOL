// ANSI SGR parser — converts ANSI-styled strings (from extension widget
// lines) into ratatui styled spans. Extension lines arrive with truecolor
// SGR (38;2;r;g;b) and braille glyphs; this preserves the Vitriolum visual
// language instead of stripping it.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

/// Parse an ANSI-styled string into a ratatui Line with styled spans.
pub fn parse_line(s: &str) -> Line<'static> {
    let mut spans: Vec<Span> = Vec::new();
    let mut style = Style::default();
    let mut text = String::new();

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() {
            // Flush pending text
            if !text.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut text), style));
            }
            let rest = &s[i..];
            if let Some((seq_len, new_style)) = parse_escape(rest, style) {
                style = new_style;
                i += seq_len;
            } else {
                // Unknown escape — skip ESC + next byte
                i += 2;
            }
        } else {
            let ch = s[i..].chars().next().unwrap();
            text.push(ch);
            i += ch.len_utf8();
        }
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
    Line::from(spans)
}

/// Parse an escape sequence at the start of `rest` (which begins with ESC).
/// Returns (sequence_length, resulting_style). Style carries forward SGR state.
fn parse_escape(rest: &str, current: Style) -> Option<(usize, Style)> {
    let bytes = rest.as_bytes();
    if bytes.len() < 2 {
        return None;
    }
    match bytes[1] {
        b'[' => {
            // CSI: ESC [ params letter
            let mut end = 2;
            while end < bytes.len() && !bytes[end].is_ascii_alphabetic() {
                end += 1;
            }
            if end >= bytes.len() {
                return None;
            }
            let final_byte = bytes[end];
            let params = &rest[2..end];
            let total = end + 1;
            if final_byte == b'm' {
                Some((total, apply_sgr(params, current)))
            } else {
                Some((total, current)) // cursor moves etc — skip
            }
        }
        b']' | b'_' | b'P' => {
            // OSC / APC / DCS: skip until BEL or ST (ESC \)
            let mut prev = 0u8;
            for (idx, b) in bytes.iter().enumerate().skip(2) {
                if *b == 0x07 || (prev == 0x1b && *b == b'\\') {
                    return Some((idx + 1, current));
                }
                prev = *b;
            }
            None
        }
        _ => Some((2, current)), // two-byte escape
    }
}

/// Apply an SGR parameter string (e.g. "38;2;46;95;163" or "0;1") to a style.
fn apply_sgr(params: &str, base: Style) -> Style {
    let mut style = base;
    let parts: Vec<&str> = params.split(';').collect();
    let mut i = 0;
    while i < parts.len() {
        let n: u8 = parts[i].parse().unwrap_or(0);
        match n {
            0 => style = Style::default(),
            1 => style = style.add_modifier(Modifier::BOLD),
            2 => style = style.add_modifier(Modifier::DIM),
            3 => style = style.add_modifier(Modifier::ITALIC),
            4 => style = style.add_modifier(Modifier::UNDERLINED),
            7 => style = style.add_modifier(Modifier::REVERSED),
            22 => style = style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style = style.remove_modifier(Modifier::ITALIC),
            24 => style = style.remove_modifier(Modifier::UNDERLINED),
            27 => style = style.remove_modifier(Modifier::REVERSED),
            30..=37 => {
                style = style.fg(basic_color(n - 30));
            }
            38 => {
                // Extended fg: 38;5;n (256) or 38;2;r;g;b (truecolor)
                if i + 1 < parts.len() {
                    match parts[i + 1] {
                        "5" if i + 2 < parts.len() => {
                            let c: u8 = parts[i + 2].parse().unwrap_or(0);
                            style = style.fg(Color::Indexed(c));
                            i += 2;
                        }
                        "2" if i + 4 < parts.len() => {
                            let r: u8 = parts[i + 2].parse().unwrap_or(0);
                            let g: u8 = parts[i + 3].parse().unwrap_or(0);
                            let b: u8 = parts[i + 4].parse().unwrap_or(0);
                            style = style.fg(legacy_rgb(r, g, b));
                            i += 4;
                        }
                        _ => {}
                    }
                }
            }
            39 => style = style.fg(Color::Reset),
            40..=47 => {
                style = style.bg(basic_color(n - 40));
            }
            48 => {
                if i + 1 < parts.len() {
                    match parts[i + 1] {
                        "5" if i + 2 < parts.len() => {
                            let c: u8 = parts[i + 2].parse().unwrap_or(0);
                            style = style.bg(Color::Indexed(c));
                            i += 2;
                        }
                        "2" if i + 4 < parts.len() => {
                            let r: u8 = parts[i + 2].parse().unwrap_or(0);
                            let g: u8 = parts[i + 3].parse().unwrap_or(0);
                            let b: u8 = parts[i + 4].parse().unwrap_or(0);
                            style = style.bg(legacy_rgb(r, g, b));
                            i += 4;
                        }
                        _ => {}
                    }
                }
            }
            49 => style = style.bg(Color::Reset),
            90..=97 => {
                style = style.fg(bright_color(n - 90));
            }
            100..=107 => {
                style = style.bg(bright_color(n - 100));
            }
            _ => {}
        }
        i += 1;
    }
    style
}

/// ANSI 0-7 in the ACTIVE theme.
///
/// Extension widgets and tool output arrive as ANSI text; resolving the codes to
/// the terminal's own named colours left that text on the default palette while
/// everything around it followed the theme (2026-11-18). The hue is preserved
/// (red stays red), the exact colour belongs to the theme.

/// Translate a legacy VITRIOL truecolour triplet into the ACTIVE palette.
///
/// Why this exists (2026-11-18): extension-sidebars emit `\x1b[38;2;r;g;bm`
/// built from `_shared/vitriol.ts` (the original palette), so the right sidebar
/// kept VITRIOL's gold and grey inside Aether and Alkahest. The parity gate
/// already guarantees that palette is a subset of `Theme::VITRIOL`, so the
/// translation table is exact — and any colour that is NOT one of those roles
/// (e.g. the 256-colour cube, or a tool's own output) passes through untouched.
fn legacy_rgb(r: u8, g: u8, b: u8) -> Color {
    use crate::theme as t;
    match (r, g, b) {
        (0x0D, 0x11, 0x17) => t::BG(),
        (0x16, 0x1B, 0x22) => t::PANEL(),
        (0x13, 0x1A, 0x22) => t::USER_BG(),
        (0x21, 0x26, 0x2D) => t::BORDER_DIM(),
        (0x3F, 0xB9, 0x50) => t::GREEN(),  // safety
        (0xFF, 0xD7, 0x00) => t::GOLD(),   // sovereignty
        (0x00, 0xFF, 0xFF) => t::CYAN(),   // solvent
        (0xFF, 0xA0, 0x00) => t::ORANGE(), // antidote
        (0xF8, 0x51, 0x49) => t::RED(),    // substrate
        (0xE0, 0xE0, 0xE0) => t::TEXT(),
        (0x8B, 0x94, 0x9E) => t::MUTED(), // gray
        (0x6E, 0x76, 0x81) => t::FAINT(), // dimGray
        (0xB2, 0x94, 0xBB) => t::VIOLET(),
        (0x2E, 0x5F, 0xA3) => t::COLD_BLUE(),
        (0xFF, 0xE0, 0x66) => t::LIGHT_YELLOW(),
        (0x8A, 0x15, 0x15) => t::DEEP_RED(),
        (0x0B, 0x5E, 0x4C) => t::DARK_TEAL(),
        (0x55, 0x60, 0x6E) => t::MERCURY(),
        (0xC0, 0xC7, 0xCF) => t::SILVER(),
        (0x2A, 0x3A, 0x52) => t::WATERMARK(),
        (0xFF, 0xFF, 0xFF) => t::WHITE(),
        other => Color::Rgb(other.0, other.1, other.2),
    }
}

fn basic_color(n: u8) -> Color {
    match n {
        0 => crate::theme::BG(), // black → the ground
        1 => crate::theme::RED(),
        2 => crate::theme::GREEN(),
        3 => crate::theme::GOLD(),
        4 => crate::theme::COLD_BLUE(),
        5 => crate::theme::VIOLET(),
        6 => crate::theme::CYAN(),
        7 => crate::theme::MUTED(), // "white"/gray → secondary text
        _ => Color::Reset,
    }
}

/// ANSI 8-15 in the ACTIVE theme: the same hues at the bright end of the palette.
fn bright_color(n: u8) -> Color {
    match n {
        0 => crate::theme::FAINT(),
        1 => crate::theme::RED(),
        2 => crate::theme::GREEN(),
        3 => crate::theme::LIGHT_YELLOW(),
        4 => crate::theme::SILVER(), // bright blue reads as pale metal here
        5 => crate::theme::VIOLET(),
        6 => crate::theme::CYAN(),
        7 => crate::theme::TEXT(),
        _ => Color::Reset,
    }
}

#[cfg(test)]
mod palette_follows_theme {
    use super::*;

    /// ANSI text must resolve through the ACTIVE palette, not the terminal's
    /// named colours — otherwise extension and tool output ignore the theme.
    #[test]
    fn ansi_codes_resolve_to_theme_roles() {
        assert_eq!(basic_color(0), crate::theme::BG());
        assert_eq!(basic_color(1), crate::theme::RED());
        assert_eq!(basic_color(2), crate::theme::GREEN());
        assert_eq!(basic_color(3), crate::theme::GOLD());
        assert_eq!(basic_color(4), crate::theme::COLD_BLUE());
        assert_eq!(basic_color(5), crate::theme::VIOLET());
        assert_eq!(basic_color(6), crate::theme::CYAN());
        assert_eq!(basic_color(7), crate::theme::MUTED());
        assert_eq!(bright_color(7), crate::theme::TEXT());
        assert_eq!(bright_color(3), crate::theme::LIGHT_YELLOW());
    }

    /// A themed 16-colour map means plain ANSI text is tinted by the palette:
    /// the same escape sequence renders differently under another theme.
    #[test]
    fn ansi_text_is_palette_not_terminal_defaults() {
        let red = basic_color(1);
        assert_ne!(red, ratatui::style::Color::Red, "still the terminal colour");
        let line = parse_line("\u{1b}[31mRED\u{1b}[0m");
        let fg = line.spans.first().and_then(|s| s.style.fg);
        assert_eq!(fg, Some(crate::theme::RED()));
    }
}

#[cfg(test)]
mod sidebar_truecolor_tests {
    use super::*;

    /// The right sidebar emits truecolour escapes built from the JS palette.
    /// Those must resolve to the ACTIVE theme's role, not to VITRIOL's literal
    /// (2026-11-18: the sidebar stayed gold inside Aether and Alkahest).
    #[test]
    fn sidebar_truecolor_follows_the_theme() {
        // 38;2;255;215;0 == VITRIOL gold (sovereignty).
        let line = parse_line("\u{1b}[38;2;255;215;0m◈ coupling\u{1b}[0m");
        let fg = line.spans.first().and_then(|s| s.style.fg);
        assert_eq!(fg, Some(crate::theme::GOLD()));
        // Note: under `vitriol` GOLD *is* that literal, so the equality above is
        // the assertion that survives every theme; the mapping is a role lookup,
        // which the Aether/Alkahest palettes prove by construction.

        // Safety green, solvent cyan, gray and violet all map too.
        for (rgb, want) in [
            ((0x3Fu8, 0xB9u8, 0x50u8), crate::theme::GREEN()),
            ((0x00, 0xFF, 0xFF), crate::theme::CYAN()),
            ((0x8B, 0x94, 0x9E), crate::theme::MUTED()),
            ((0xB2, 0x94, 0xBB), crate::theme::VIOLET()),
        ] {
            assert_eq!(legacy_rgb(rgb.0, rgb.1, rgb.2), want, "{rgb:?}");
        }
    }

    /// Anything that is not a legacy role keeps its exact colour — the 256-colour
    /// cube and a tool's own output must not be recoloured by guesswork.
    #[test]
    fn unknown_truecolor_passes_through() {
        let c = legacy_rgb(0x12, 0x34, 0x56);
        assert_eq!(c, ratatui::style::Color::Rgb(0x12, 0x34, 0x56));
    }
}
