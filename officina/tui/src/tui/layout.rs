// TUI layout — Vitriolum dashboard styling.
//
// Anatomy (mirrors vitriol-tui/src/ui.rs):
//   header row   — 🜖 officina gold banner + model id + session name
//   body         — chat │ coldBlue divider │ sidebar (rounded panels)
//   footer row   — muted keybar + working indicator / engine warning
//
// Fresh screen (zero entries) → the VITRIOL braille watermark rises in the
// chat area (watermark.rs, bottom-anchored, no partial reveal).

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::ansi;
use super::state::AppState;
use crate::theme;

const SIDEBAR_W: u16 = 42;

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(0),    // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    render_header(frame, rows[0], state);
    render_body(frame, state, rows[1]);
    render_footer(frame, rows[1 + 1], state);
}

// ── Header ────────────────────────────────────────────────────────────────

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut left = vec![Span::styled(
        format!("{} officina", theme::GLYPH_VITRIOL),
        theme::banner(),
    )];
    if state.is_streaming {
        left.push(Span::styled(
            format!("  {} working", theme::GLYPH_FIRE),
            theme::warn(),
        ));
    }

    let model = state
        .model
        .as_ref()
        .map(|m| m.id.clone())
        .unwrap_or_default();
    let right = state
        .session_name
        .clone()
        .unwrap_or_else(|| if state.session_id.is_empty() { String::new() } else { format!("#{}", &state.session_id[..state.session_id.len().min(8)]) });

    // Right-align the session label; model id sits before it.
    let right_w = (right.chars().count() + 2) as u16;
    let mid_w = if right.is_empty() { area.width } else { area.width.saturating_sub(right_w) };
    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(right_w.max(0))])
        .split(Rect { width: area.width, ..area });

    let mut mid_spans = left;
    if !model.is_empty() {
        let avail = mid_w.saturating_sub(24) as usize;
        mid_spans.push(Span::styled(
            format!("  {}", trunc(&model, avail.max(8))),
            theme::live(),
        ));
    }
    let mut line = Line::from(mid_spans);
    // pad to push right span flush
    let _ = &mut line;

    frame.render_widget(Paragraph::new(line), Rect { width: mid_w, ..cols[0] });
    if !right.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(right, theme::muted())))
                .alignment(Alignment::Right),
            Rect {
                x: area.x + area.width.saturating_sub(right_w),
                width: right_w,
                ..area
            },
        );
    }
}

// ── Body ──────────────────────────────────────────────────────────────────

fn render_body(frame: &mut Frame, state: &mut AppState, area: Rect) {
    // Two columns, no gap divider — the sidebar panel's own rounded border
    // separates it from the chat column.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(SIDEBAR_W)])
        .split(area);

    render_main(frame, state, cols[0]);
    render_sidebar(frame, state, cols[1]);
}

fn render_main(frame: &mut Frame, state: &mut AppState, area: Rect) {
    if state.show_diag {
        render_diag_overlay(frame, state, area);
        return;
    }
    if state.resume_open {
        render_chat_and_editor(frame, state, area);
        render_resume_modal(frame, state, area);
        return;
    }
    render_chat_and_editor(frame, state, area);
}

fn render_chat_and_editor(frame: &mut Frame, state: &mut AppState, area: Rect) {

    // Fresh screen → the stone rises, lifted four rows clear of the editor.
    if state.entries.is_empty() {
        crate::watermark::render(
            frame,
            Rect {
                height: area.height.saturating_sub(4),
                ..area
            },
        );
        // still render the editor below the watermark area
    }

    let input_h = if state.input.chars().count() + 8 > area.width as usize {
        5
    } else {
        3
    };
    let chat_area = Rect {
        height: area.height.saturating_sub(input_h as u16),
        ..area
    };
    let input_area = Rect {
        y: area.y + chat_area.height,
        height: input_h as u16,
        ..area
    };
    let chat_width = (chat_area.width as usize).saturating_sub(1);

    let mut lines = state.chat_lines(chat_width);

    if state.is_streaming {
        lines.push(Line::from(vec![
            Span::styled("ai ▸ ", Style::default().fg(theme::CYAN).bg(theme::BG)),
            Span::styled(
                format!("{} working…", theme::GLYPH_FIRE),
                theme::warn(),
            ),
        ]));
    }
    if state.is_compacting {
        lines.push(Line::from(Span::styled(
            format!("  {} compacting context…", theme::GLYPH_ALEMBIC),
            theme::warn(),
        )));
    }

    let visible = chat_area.height as usize;
    let start = lines.len().saturating_sub(visible);
    let view: Vec<Line> = lines.into_iter().skip(start).collect();

    frame.render_widget(
        Paragraph::new(view).wrap(Wrap { trim: false }),
        chat_area,
    );

    // Autocomplete popup floats above the editor
    let cands = state.command_candidates();
    if !cands.is_empty() {
        let max_rows = (chat_area.height * 6 / 10).max(6);
        let popup_h = (cands.len() as u16 + 2).min(max_rows);
        let popup_area = Rect {
            y: input_area.y.saturating_sub(popup_h),
            height: popup_h,
            width: area.width.min(60),
            ..input_area
        };
        render_command_popup(frame, &cands, state.cand_sel, popup_area);
    }

    // Editor — rounded, PANEL bg; border reflects state (vitriol-tui panel anatomy)
    let border_style = if state.is_streaming {
        Style::new().fg(theme::COLD_BLUE)
    } else if !cands.is_empty() {
        Style::new().fg(theme::GOLD)
    } else {
        Style::new().fg(theme::BORDER_DIM)
    };
    let input_block = panel(
        Span::styled(" officina ", theme::banner()),
        border_style,
    );
    let inner = input_block.inner(input_area);
    frame.render_widget(input_block, input_area);

    let prompt = Span::styled("> ", Style::default().fg(theme::GREEN).bg(theme::PANEL));
    let (before, at, after) = split_at_char(&state.input, state.cursor);
    let line = Line::from(vec![
        prompt,
        Span::raw(before),
        Span::styled(
            at,
            Style::default()
                .bg(theme::PANEL)
                .add_modifier(Modifier::REVERSED),
        ),
        Span::raw(after),
    ]);
    frame.render_widget(Paragraph::new(line).style(Style::default().bg(theme::PANEL)), inner);
}

fn render_sidebar(frame: &mut Frame, state: &mut AppState, area: Rect) {
    // Rounded neutral panel, dim border, gold title — panel_neutral equivalent.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_DIM))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(" sidebar ", Style::default().fg(theme::GOLD)))
        .title_bottom(Span::styled(" officina ", theme::muted()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for widget in &state.widgets {
        if widget.lines.is_empty() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::from(""));
        }
        for l in &widget.lines {
            lines.push(ansi::parse_line(l));
        }
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("{} waiting for agent…", theme::GLYPH_VITRIOL),
            theme::muted(),
        )));
    }

    let view: Vec<Line> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(
        Paragraph::new(view).wrap(Wrap { trim: false }),
        inner,
    );
}

// ── Popups / overlays ─────────────────────────────────────────────────────

fn render_command_popup(
    frame: &mut Frame,
    cands: &[crate::rpc::protocol::SlashCommand],
    sel: usize,
    area: Rect,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::BG));

    let total = cands.len();
    let visible = (area.height as usize).saturating_sub(2);
    let title = if total > visible {
        format!(" commands {} · Tab completes ", total)
    } else {
        " commands · Tab completes ".to_string()
    };
    let block = block.title(Span::styled(title, theme::muted()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let skip = if sel + 1 > visible {
        sel + 1 - visible
    } else {
        0
    };
    let cands = &cands[skip.min(total)..];
    let mut lines: Vec<Line> = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        let absolute = skip + i;
        let desc = c.description.clone().unwrap_or_default();
        let selected = absolute == sel;
        let (name_style, desc_style, marker) = if selected {
            (
                Style::new().fg(theme::GOLD).bg(theme::PANEL).add_modifier(Modifier::BOLD),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
                "▸ ",
            )
        } else {
            (
                Style::new().fg(theme::GOLD),
                theme::muted(),
                "  ",
            )
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{:<14}", marker, c.name),
                name_style,
            ),
            Span::styled(
                trunc(&desc, (inner.width as usize).saturating_sub(18)),
                desc_style,
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_diag_overlay(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::crit())
        .style(Style::new().bg(theme::BG))
        .title(Span::styled(
            format!(" {} stderr diagnostics · F9 close ", theme::GLYPH_SULFUR),
            theme::crit(),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines: Vec<Line> = state
        .diag_view
        .iter()
        .map(|l| Line::from(Span::raw(trunc(l, (inner.width as usize).saturating_sub(1)))))
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Footer ────────────────────────────────────────────────────────────────

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut spans = vec![
        Span::styled("enter send", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("tab complete", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("esc dissolve", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("^c/^q quit", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("f9 stderr", theme::muted()),
    ];

    // Widgets present = telemetry path alive; else warn like vitriol-tui.
    if state.widgets.is_empty() {
        spans.push(Span::styled(
            "  ·  stack unreachable — no agent telemetry",
            theme::warn(),
        ));
    } else if let Some((msg, _)) = &state.notice {
        spans.push(Span::styled(
            format!("  ·  {}", trunc(msg, 40)),
            theme::gold_muted(),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_resume_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area.height.min((state.resume_entries.len() as u16 + 3).max(8));
    let w = area.width.min(70);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::BG))
        .title(Span::styled(" resume session ", theme::banner()))
        .title_bottom(Span::styled(
            " ↑↓ select · enter resume · esc cancel ",
            theme::muted(),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let visible = inner.height as usize;
    let sel = state.resume_sel;
    let skip = if sel + 1 > visible { sel + 1 - visible } else { 0 };
    let entries = &state.resume_entries[skip.min(state.resume_entries.len())..];

    let mut lines: Vec<Line> = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let absolute = skip + i;
        let when = e
            .modified
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| {
                let days = d.as_secs() / 86400;
                if days > 0 {
                    format!("{}d", days)
                } else {
                    format!("{}h", d.as_secs() / 3600)
                }
            })
            .unwrap_or_else(|_| "?".into());
        let selected = absolute == sel;
        if selected {
            lines.push(Line::from(Span::styled(
                trunc(
                    &format!("▸ {:>4}  {:>3} msg  {}", when, e.msg_count, e.title),
                    (inner.width as usize).saturating_sub(1),
                ),
                Style::new()
                    .fg(theme::BG)
                    .bg(theme::GOLD)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("{:>5} ", when), theme::live()),
                Span::styled(format!("{:>3} ", e.msg_count), theme::muted()),
                Span::styled(
                    format!(" {}", e.title),
                    Style::new().fg(theme::TEXT).bg(theme::BG),
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Panel helper (vitriol-tui anatomy) ────────────────────────────────────

fn panel<'a>(title: Span<'a>, border: Style) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .style(Style::new().bg(theme::PANEL))
        .title(title)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let cut: String = s.chars().take(w.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

fn split_at_char(s: &str, idx: usize) -> (String, String, String) {
    let mut before = String::new();
    let mut at = String::new();
    let mut after = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i < idx {
            before.push(ch);
        } else if i == idx {
            at.push(ch);
        } else {
            after.push(ch);
        }
    }
    if at.is_empty() {
        at.push(' ');
    }
    (before, at, after)
}
