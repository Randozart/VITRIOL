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
                            style = style.fg(Color::Rgb(r, g, b));
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
                            style = style.bg(Color::Rgb(r, g, b));
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

fn basic_color(n: u8) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        _ => Color::Reset,
    }
}

fn bright_color(n: u8) -> Color {
    match n {
        0 => Color::DarkGray,
        1 => Color::LightRed,
        2 => Color::LightGreen,
        3 => Color::LightYellow,
        4 => Color::LightBlue,
        5 => Color::LightMagenta,
        6 => Color::LightCyan,
        7 => Color::White,
        _ => Color::Reset,
    }
}

/// Visible cell width of an ANSI-styled string (escapes removed).
pub fn visible_len(s: &str) -> usize {
    s.replace('\x1b', "").chars().filter(|c| !c.is_control()).count()
}
