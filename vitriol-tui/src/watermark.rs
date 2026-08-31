//! Watermark — the VITRIOL braille logo, faint, on screens with room.
//!
//! 2026-09-01 (owner request): the stone rises from the bottom of main
//! screens (Controls when idle, Logs, Dashboard leftover space) in a
//! barely-there blue lift (#1c2634 on #0d1117) — same treatment as the
//! Officina startup watermark. Asset: assets/braille-logo-80c.txt.
//! Missing asset or too-small area = silently nothing.
//!
//! Provenance: original work, this repo; art is the owner's braille logo.

use std::path::Path;
use std::sync::OnceLock;

use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

const TINT: Color = Color::Rgb(0x1c, 0x26, 0x34);

fn logo_lines(repo_root: &Path) -> Option<&'static Vec<String>> {
    static LOGO: OnceLock<Option<Vec<String>>> = OnceLock::new();
    LOGO
        .get_or_init(|| {
            // canonical location only: the VITRIOL repo's assets dir
            let p = repo_root.join("assets").join("braille-logo-80c.txt");
            std::fs::read_to_string(p)
                .ok()
                .map(|text| text.lines().map(|l| l.to_string()).collect())
        })
        .as_ref()
}

/// Draw the logo bottom-aligned and horizontally centered inside `area`,
/// showing as many rows as fit. Partial reveal is intentional: on small
/// areas the stone "rises".
pub fn render(frame: &mut Frame, area: Rect, repo_root: &Path) {
    let Some(lines) = logo_lines(repo_root) else { return };
    if area.height < 4 || area.width < 20 {
        return;
    }
    let show = lines.len().min(area.height as usize);
    let skip = lines.len() - show;
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let left_pad = area.width.saturating_sub(width as u16) / 2;
    let y = area.y + area.height - show as u16;
    let rect = Rect { x: area.x, y, width: area.width, height: show as u16 };
    let style = Style::default().fg(TINT).add_modifier(Modifier::DIM);
    let spans: Vec<Line> = lines[skip..]
        .iter()
        .map(|l| {
            let mut s = Span::styled(" ".repeat(left_pad as usize), style);
            s.content = format!("{}{}", s.content, l).into();
            Line::from(s)
        })
        .collect();
    frame.render_widget(Paragraph::new(spans).alignment(Alignment::Left), rect);
}
