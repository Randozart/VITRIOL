//! Watermark — the VITRIOL braille logo, faint, on fresh screens.
//!
//! Ported from vitriol-tui/src/watermark.rs (this repo, Apache-2.0) with one
//! change: the logo is embedded at compile time via include_str! so the
//! binary works from any working directory. The stone rises from the bottom
//! of the chat area on a fresh screen (zero entries) in a barely-there blue
//! lift (#1c2634 on #0d1117). Missing asset or too-small area = silently
//! nothing — a cut braille stone looks broken.

use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::theme;

const LOGO_RAW: &str = include_str!("../../../assets/braille-logo-80c.txt");

fn logo_lines() -> &'static Vec<String> {
    use std::sync::OnceLock;
    static LOGO: OnceLock<Vec<String>> = OnceLock::new();
    LOGO.get_or_init(|| LOGO_RAW.lines().map(|l| l.to_string()).collect())
}

/// Draw the FULL logo, horizontally centered, bottom-anchored inside `area` —
/// but only if the area fits every row. No partial reveal.
pub fn render(frame: &mut Frame, area: Rect) {
    let lines = logo_lines();
    if area.height < 4 || area.width < 20 {
        return;
    }
    if (area.height as usize) < lines.len() {
        return;
    }
    let show = lines.len();
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let left_pad = area.width.saturating_sub(width as u16) / 2;
    let y = area.y + area.height - show as u16;
    let rect = Rect {
        x: area.x,
        y,
        width: area.width,
        height: show as u16,
    };
    let style = Style::default()
        .fg(theme::WATERMARK)
        .add_modifier(Modifier::DIM);
    let pad = " ".repeat(left_pad as usize);
    let spans: Vec<Line> = lines
        .iter()
        .map(|l| Line::from(Span::styled(format!("{}{}", pad, l), style)))
        .collect();
    frame.render_widget(Paragraph::new(spans).alignment(Alignment::Left), rect);
}
