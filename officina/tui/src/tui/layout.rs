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

/// Canonical mode → (alchemical symbol, color): the moon metal for PLAN,
/// the sun metal for BUILD (owner request 2026-09-02). Custom modes → None.
fn mode_chip(label: &str) -> Option<(&'static str, ratatui::style::Color)> {
    match label.to_ascii_uppercase().as_str() {
        "PLAN" => Some(("☽", theme::SILVER)),
        "BUILD" => Some(("☉", theme::GOLD)),
        _ => None,
    }
}

/// Chip spans: bold symbol+label in the mode color, greyed "· tab" hint.
/// `bg` sets the backdrop (BG for header use, PANEL for the composer title).
fn mode_spans(
    symbol: &str,
    label: &str,
    color: ratatui::style::Color,
    bg: ratatui::style::Color,
) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!("{} {}", symbol, label),
            Style::new().fg(color).bg(bg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · tab to switch mode", Style::new().fg(theme::MUTED).bg(bg)),
    ]
}

/// The composer title: the active agent mode (owner request 2026-09-02 —
/// the chip lives above the prompt box, not in the header). None until the
/// agent-mode widget reports in → titleless border.
fn composer_title(state: &AppState) -> Option<Vec<Span<'static>>> {
    let label = state.agent_mode_label.as_deref()?;
    if label.is_empty() {
        return None;
    }
    match mode_chip(label) {
        Some((symbol, color)) => Some(mode_spans(symbol, label, color, theme::PANEL)),
        // Custom mode: the widget's own badge glyph, gold chip.
        None => {
            let glyph = state
                .agent_mode_glyph
                .clone()
                .unwrap_or_else(|| theme::GLYPH_VITRIOL.to_string());
            Some(mode_spans(&glyph, label, theme::GOLD, theme::PANEL))
        }
    }
}

fn render_header(frame: &mut Frame, area: Rect, state: &AppState) {
    // Brand line (owner request 2026-09-02): 🜖VITRIOL·OFFICINA lives in the
    // top-left permanently; the agent mode moved to the composer title.
    let mut left = vec![Span::styled(
        "🜖 VITRIOL·OFFICINA",
        theme::banner(),
    )];
    if state.is_streaming {
        // Light yellow, the informational voice (owner request 2026-09-02):
        // same tint the engine TUI uses for "prompt-eval …" — the process
        // is moving. Orange stays reserved for genuine warnings.
        left.push(Span::styled(
            format!("  {} working", theme::GLYPH_FIRE),
            theme::info(),
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
        .unwrap_or_else(|| if state.session_id.is_empty() { String::new() } else { format!("SESSION ID: #{}", &state.session_id[..state.session_id.len().min(8)]) });

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

    // Fresh screen → the stone rises, lifted four rows clear of the editor,
    // centered in the space it occupies, glimmering (mode: /glimmer).
    if state.entries.is_empty() {
        crate::watermark::render(
            frame,
            Rect {
                height: area.height.saturating_sub(4),
                ..area
            },
            state.started.elapsed().as_millis(),
            state.glimmer,
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
            Span::styled(
                format!("{} ", theme::GLYPH_AI),
                Style::default().fg(theme::GOLD).bg(theme::BG),
            ),
            // Light yellow to match the header (owner request 2026-09-02 —
            // the informational voice, like the engine TUI's prompt-eval).
            Span::styled("working…", theme::info()),
        ]));
    }
    if state.is_compacting {
        // Compaction is a status, not a warning — informational voice.
        lines.push(Line::from(Span::styled(
            format!("  {} compacting context…", theme::GLYPH_ALEMBIC),
            theme::info(),
        )));
    }

    let visible = chat_area.height as usize;

    // Scrollback (owner request 2026-09-02): `scroll` counts rows back
    // from the live tail; the renderer owns the clamp because only it
    // knows the true line count. Streaming output keeps flowing beneath a
    // scrolled-back view (offset-from-bottom anchoring).
    let total = lines.len();
    let max_scroll = total.saturating_sub(visible) as u16;
    state.scroll_max = max_scroll;
    if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }
    let start = total.saturating_sub(visible + state.scroll as usize);
    let view: Vec<Line> = lines.into_iter().skip(start).collect();

    // Composer flames (owner request 2026-09-02): braille alchemical fire
    // rising from the editor's top edge; intensity = GPU load (low-passed
    // in the run loop). /fire toggles it. TEXT PRIORITY (owner request,
    // after live testing): the fire is drawn BEFORE the chat so the
    // transcript renders on top — flames read as a backdrop, visible only
    // in the gaps, never over a character.
    let fire_rows = if state.fire_on {
        crate::fire::rows_for(state.fire_level)
    } else {
        0
    };
    let fire_map = if fire_rows > 0 && fire_rows as u16 <= chat_area.height {
        crate::fire::render(
            frame,
            Rect {
                y: input_area.y.saturating_sub(fire_rows as u16),
                height: fire_rows as u16,
                ..input_area
            },
            state.started.elapsed().as_millis(),
            state.fire_level,
            state.fire_style,
        )
    } else {
        Vec::new()
    };

    frame.render_widget(
        Paragraph::new(view).wrap(Wrap { trim: false }),
        chat_area,
    );

    // Text-tint pass (owner request 2026-09-02: "user text discolors based
    // on the fire beneath it — do the same for AI text"). Unstyled user
    // spans already inherit the flame fg via ratatui's style-patch
    // semantics; markdown-styled AI spans don't. Walk the fire map and
    // force the flame color onto any text glyph inside the strip — flames
    // lend their color to everything standing in them.
    if !fire_map.is_empty() {
        let fire_y = input_area.y.saturating_sub(fire_rows as u16);
        let buffer = frame.buffer_mut();
        for (ry, map_row) in fire_map.iter().enumerate() {
            for (cx, c) in map_row.iter().enumerate() {
                if let Some(color) = c {
                    if let Some(cell) = buffer.cell_mut(ratatui::layout::Position {
                        x: input_area.x + cx as u16,
                        y: fire_y + ry as u16,
                    }) {
                        if cell.symbol() != " " {
                            cell.set_fg(*color);
                        }
                    }
                }
            }
        }
    }

    // Quicksilver gauge — single braille-dot column at the chat area's
    // right edge (owner request 2026-09-03). That column is already
    // reserved: chat_width = width − 1, so text never touches it.
    // Thumb position/size maps scroll onto the viewport; hidden when
    // the session fits in one screen or the watermark is up.
    {
        let total = (state.scroll_max as usize) + chat_area.height as usize;
        let visible = chat_area.height as usize;
        if total > visible && !state.entries.is_empty() && chat_area.width > 1 {
            let h = chat_area.height as usize;
            let thumb_h = ((visible as f64 / total as f64) * h as f64)
                .round()
                .max(1.0) as usize;
            let scroll_pct = if total <= visible {
                0.0
            } else {
                state.scroll as f64 / (total - visible) as f64
            };
            let thumb_top = scroll_pct * (h.saturating_sub(thumb_h)) as f64;
            let thumb_bot = thumb_top + thumb_h as f64;
            let gx = chat_area.x + chat_area.width - 1;
            let buffer = frame.buffer_mut();

            for row in 0..h {
                let fy = row as f64;
                // Above or below the thumb — empty.
                if fy + 1.0 < thumb_top || fy > thumb_bot {
                    continue;
                }
                // Braille left-column dots: dot1=0x01, dot2=0x02,
                // dot3=0x04, dot7=0x40. Full column = 0x47.
                // Fractional fill maps to dots from the top down.
                let bits = if fy >= thumb_top && fy + 1.0 <= thumb_bot {
                    0x47u32 // full cell
                } else if fy < thumb_top {
                    // top edge — partial
                    let frac = (fy + 1.0 - thumb_top).clamp(0.0, 1.0);
                    let dots = (frac * 4.0).round() as u32;
                    [0x00, 0x01, 0x03, 0x05, 0x47][dots.min(4) as usize]
                } else {
                    // bottom edge — partial
                    let frac = (thumb_bot - fy).clamp(0.0, 1.0);
                    let dots = (frac * 4.0).round() as u32;
                    [0x00, 0x40, 0x44, 0x46, 0x47][dots.min(4) as usize]
                };
                if bits == 0 {
                    continue;
                }
                let ch = char::from_u32(0x2800 + bits).unwrap_or('⡇');
                if let Some(cell) = buffer.cell_mut(ratatui::layout::Position {
                    x: gx,
                    y: chat_area.y + row as u16,
                }) {
                    cell.set_symbol(&ch.to_string());
                    cell.set_fg(theme::SILVER);
                    cell.set_bg(theme::BG);
                }
            }
        }
    }

    // Position badge while scrolled back — top-right of the chat column.
    // Drawn after the chat: the badge itself is text and keeps priority.
    if state.scroll > 0 {
        let badge = format!(" ↑ {}/{} ", state.scroll, max_scroll);
        let bw = badge.chars().count() as u16;
        if chat_area.width > bw {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(badge, theme::muted()))).alignment(Alignment::Right),
                Rect {
                    x: chat_area.x,
                    width: chat_area.width,
                    height: 1,
                    ..chat_area
                },
            );
        }
    }

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

    // Editor — rounded, PANEL bg; border reflects state (vitriol-tui panel
    // anatomy). Title carries the active agent mode (owner request
    // 2026-09-02); titleless until the widget reports in.
    let border_style = if state.is_streaming {
        Style::new().fg(theme::COLD_BLUE)
    } else if !cands.is_empty() {
        Style::new().fg(theme::GOLD)
    } else {
        Style::new().fg(theme::BORDER_DIM)
    };
    let mut input_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .style(Style::new().bg(theme::PANEL));
    if let Some(title) = composer_title(state) {
        input_block = input_block.title(Line::from(title));
    }
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
    // Rounded neutral panel, dim border — no title (owner request
    // 2026-09-02: brand lives in the header, mode in the composer title).
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_DIM))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    for widget in &state.widgets {
        if widget.lines.is_empty() {
            continue;
        }
        // agent-mode lives in the composer title now; engine-fire is the
        // flames' raw data feed — neither is sidebar content (owner request
        // 2026-09-02). Both widgets are still received and parsed.
        if widget.key == "agent-mode" || widget.key == "engine-fire" {
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

    // Widgets present = telemetry path alive. Absence is NOT a fault —
    // bare projects have no officina extensions — so the hint is muted
    // gray, not an orange "stack unreachable" false alarm (owner report
    // 2026-09-02, launched in Projects/ontic).
    if state.widgets.is_empty() {
        spans.push(Span::styled(
            "  ·  no telemetry",
            Style::new().fg(theme::MUTED).bg(theme::BG),
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
