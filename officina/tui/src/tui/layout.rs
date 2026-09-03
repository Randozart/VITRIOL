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
        Constraint::Length(1), // breathing room (owner request 2026-09-03)
        Constraint::Min(0),    // body
        Constraint::Length(1), // footer
    ])
    .split(area);

    render_header(frame, rows[0], state);
    render_body(frame, state, rows[2]);
    render_footer(frame, rows[3], state);
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

/// `/home/<user>/…` → `~/…` (the sidebar's old convention, moved to the
/// header with the session identity — 2026-09-03 declutter).
fn shorten_home(path: &std::path::Path) -> String {
    let s = path.to_string_lossy().to_string();
    match s.strip_prefix("/home/") {
        Some(rest) => match rest.find('/') {
            Some(i) => format!("~/{}", &rest[i + 1..]),
            None => "~".to_string(),
        },
        None => s,
    }
}

/// Header right label: session identity @ working folder.
/// Named → `"name" @ ~/path`; unnamed with id → `SESSION ID: #xxxxxxxx @ ~/path`;
/// neither → None.
fn session_label(state: &AppState) -> Option<String> {
    let home = shorten_home(&state.cwd);
    if let Some(name) = state.session_name.as_deref().filter(|n| !n.is_empty()) {
        return Some(format!("\"{}\" @ {}", name, home));
    }
    if !state.session_id.is_empty() {
        return Some(format!(
            "SESSION ID: #{} @ {}",
            &state.session_id[..state.session_id.len().min(8)],
            home
        ));
    }
    None
}

/// Fit a session label to `cap` chars: the identity prefix stays intact,
/// the path after " @ " elides from the LEFT (a path's tail identifies it),
/// snapping the cut to a `/` component boundary when one is close.
fn elide_path_tail(label: &str, cap: usize) -> String {
    let total = label.chars().count();
    if total <= cap {
        return label.to_string();
    }
    match label.find(" @ ") {
        Some(i) => {
            let path: Vec<char> = label[i + 3..].chars().collect();
            let mut cut = cap.saturating_sub(i + 4).min(path.len());
            if cut == 0 {
                return label.chars().take(cap).collect();
            }
            // Prefer cutting at a component boundary (/X) when it's close —
            // `…/VITRIOL` reads better than `…TRIOL`. `pos` is window-
            // relative: shifting the kept window start to pos+1 shrinks the
            // kept count by pos+1.
            if let Some(pos) = path[..cut].iter().rposition(|c| *c == '/') {
                if cut - pos <= 12 {
                    cut -= pos + 1;
                }
            }
            let tail: String = path[path.len() - cut..].iter().collect();
            format!("{} @ …{}", &label[..i], tail)
        }
        None => label.chars().take(cap).collect(),
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
    let right = session_label(state)
        .map(|l| elide_path_tail(&l, (area.width.saturating_sub(24) as usize).max(16)))
        .unwrap_or_default();

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
    if state.tools_modal_open {
        render_chat_and_editor(frame, state, area);
        render_tools_modal(frame, state, area);
        return;
    }
    if state.help_open {
        render_chat_and_editor(frame, state, area);
        render_help_modal(frame, state, area);
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
    // One breathing row between the transcript and the prompt box (owner
    // request 2026-09-03 — text flowed straight into the editor).
    let chat_area = Rect {
        height: area.height.saturating_sub(input_h as u16 + 1),
        ..area
    };
    let input_area = Rect {
        y: area.y + chat_area.height + 1,
        height: input_h as u16,
        ..area
    };
    let chat_width = (chat_area.width as usize).saturating_sub(1);
    // Selection mapping needs the viewport at ^c time (handle_key has no
    // Frame) — stash it every frame.
    state.last_chat_area = Some(chat_area);

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

    // Gap row: explicit BG paint, ALWAYS (owner request 2026-09-03 — the
    // terminal background must never peek through below the panels). The
    // carved motto rides on it when there's room and the screen isn't
    // fresh; fire dots rising through the gap keep their flame fg because
    // set_style patches bg only.
    {
        let gap = Rect {
            y: area.y + chat_area.height,
            height: 1,
            ..area
        };
        let motto = if state.entries.is_empty() {
            None
        } else {
            motto_for(area.width as usize)
        };
        let mut spans: Vec<Span> = Vec::new();
        if let Some(m) = motto {
            spans.push(carved(&m));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::BG)),
            gap,
        );
    }

    // NO paragraph wrap: chat_lines pre-wraps everything to budget. A
    // paragraph-level re-wrap would push rows off the bottom and SILENTLY
    // CLIP the transcript (owner bug 2026-09-03: "the console box hides
    // the bottom few rows" — raw code/table lines re-wrapped). Overlong
    // lines now lose one char horizontally instead of hiding history.
    //
    // EXPLICIT BG (owner request 2026-09-03): the style paints the whole
    // rect before the lines render — without it, cells no span touched
    // (blank rows, line tails) show the TERMINAL's own background, and the
    // console reads as two different darks. set_style patches bg only: the
    // fresh-screen stone keeps its glyphs and DIM.
    frame.render_widget(
        Paragraph::new(view).style(Style::new().bg(theme::BG)),
        chat_area,
    );

    // Text-tint pass (owner request 2026-09-02: "user text discolors based
    // on the fire beneath it — do the same for AI text"). Unstyled user
    // spans already inherit the flame fg via ratatui's style-patch
    // semantics; markdown-styled AI spans don't. Walk the fire map and
    // force the flame color onto any text glyph inside the strip — flames
    // lend their color to everything standing in them.
    //
    // 2026-09-03: `/fire tint off` keeps text its original color — EXCEPT
    // the carved motto (owner: "make the VISITA INTERIOREM text still
    // discolor as the one exception"). The motto lives in the gap row,
    // which is the fire strip's bottom row — so with tint off, only that
    // row is walked.
    if !fire_map.is_empty() {
        let tint_all = state.fire_tint;
        let fire_y = input_area.y.saturating_sub(fire_rows as u16);
        let buffer = frame.buffer_mut();
        for (ry, map_row) in fire_map.iter().enumerate() {
            if !tint_all && ry + 1 != fire_map.len() {
                continue; // the one exception: only the motto's row still tints
            }
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

    // Selection highlight (owner request 2026-09-03): left-drag across the
    // transcript, ^c copies. Full-width rows, gauge column excluded, drawn
    // after the fire tint so the highlight reads as selection, not flame.
    if let Some((y0, y1)) = state.selection_rows() {
        let (y0, y1) = (
            y0.max(chat_area.y),
            y1.min(chat_area.y + chat_area.height.saturating_sub(1)),
        );
        if y1 >= y0 {
            let buffer = frame.buffer_mut();
            let x_end = chat_area.x + chat_area.width.saturating_sub(1);
            for y in y0..=y1 {
                for x in chat_area.x..x_end {
                    if let Some(cell) = buffer.cell_mut(ratatui::layout::Position { x, y }) {
                        cell.set_bg(theme::COLD_BLUE);
                    }
                }
            }
        }
    }

    // Quicksilver gauge — single braille-dot column at the chat area's
    // right edge (owner request 2026-09-03). That column is already
    // reserved: chat_width = width − 1, so text never touches it.
    // FILL-UP depth gauge (owner revision, same day): not a moving thumb —
    // the column fills from the bottom as you scroll UP through history.
    // At the live tail it is EMPTY; fully scrolled back, it is FULL.
    // Hidden when the session fits in one screen or the watermark is up.
    {
        let total = (state.scroll_max as usize) + chat_area.height as usize;
        let visible = chat_area.height as usize;
        if total > visible && !state.entries.is_empty() && chat_area.width > 1 {
            let h = chat_area.height as usize;
            let max_s = (total - visible) as f64;
            let frac = if max_s > 0.0 {
                (state.scroll as f64 / max_s).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let fill = frac * h as f64;
            let gx = chat_area.x + chat_area.width - 1;
            let buffer = frame.buffer_mut();

            for row in 0..h {
                let bits = gauge_row_bits(row, h, fill);
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
    let line = if state.input.is_empty() {
        // Carved motto placeholder (owner request 2026-09-03) — the cursor
        // block sits ON the first letter, the rest fades into the panel.
        let budget = (inner.width as usize).saturating_sub(2); // "> " prompt
        let motto = motto_for(budget).unwrap_or_default();
        let mut chars = motto.chars();
        let first = chars.next().unwrap_or(' ');
        let rest: String = chars.collect();
        Line::from(vec![
            prompt,
            Span::styled(
                first.to_string(),
                Style::default()
                    .fg(theme::WATERMARK)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::REVERSED | Modifier::DIM),
            ),
            Span::styled(
                rest,
                Style::default()
                    .fg(theme::WATERMARK)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::DIM),
            ),
        ])
    } else {
        Line::from(vec![
            prompt,
            Span::raw(before),
            Span::styled(
                at,
                Style::default()
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::REVERSED),
            ),
            Span::raw(after),
        ])
    };
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
        .style(Style::new().bg(theme::PANEL));

    let total = cands.len();
    let visible = (area.height as usize).saturating_sub(2);
    let title = if total > visible {
        format!(" commands {} · Tab completes ", total)
    } else {
        " commands · Tab completes ".to_string()
    };
    let block = block.title(Span::styled(title, Style::new().fg(theme::MUTED)));
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
                Style::new().fg(theme::MUTED),
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
        .border_style(Style::new().fg(theme::RED))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} stderr diagnostics · F9 close ", theme::GLYPH_SULFUR),
            Style::new().fg(theme::RED),
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
        Span::styled("drag select · ^c copy", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("^esc quit", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("f9 stderr", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("^v tools", theme::muted()),
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
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            " resume session ",
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ select · enter resume · esc cancel ",
            Style::new().fg(theme::MUTED),
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
                Span::styled(format!("{:>5} ", when), Style::new().fg(theme::CYAN)),
                Span::styled(format!("{:>3} ", e.msg_count), Style::new().fg(theme::MUTED)),
                Span::styled(format!(" {}", e.title), Style::new().fg(theme::TEXT)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// /tools modal — tool verbosity config, resume-picker styling (owner
/// request 2026-09-03). Row 0 = global default; rows 1.. = known tools.
/// enter cycles mode · tab cycles strictness · bksp clears · esc closes.
fn render_tools_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    use crate::tui::state::{Strictness, ToolOverride};
    let row_count = crate::tui::state::KNOWN_TOOLS.len() + 1;
    let h = area.height.min((row_count as u16 + 3).max(9));
    let w = area.width.min(60);
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
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} tool verbosity ", theme::GLYPH_CRUCIBLE),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ select · enter mode · tab strict · ⌫ clear · esc ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let visible = inner.height as usize;
    let sel = state.tools_modal_sel;
    let skip = if sel + 1 > visible { sel + 1 - visible } else { 0 };

    let mut lines: Vec<Line> = Vec::new();
    for row in skip..row_count {
        let selected = row == sel;
        let (label, status, status_st): (String, String, Style) = if row == 0 {
            (
                "global".to_string(),
                state.tool_default.label().to_string(),
                Style::new().fg(theme::GOLD),
            )
        } else {
            let name = crate::tui::state::KNOWN_TOOLS[row - 1];
            let (status, st) = match state.tool_overrides.get(name) {
                Some(ToolOverride { mode, strictness }) => {
                    let word = match strictness {
                        Strictness::Pinned => "pinned",
                        Strictness::AtLeast => "at least",
                        Strictness::AtMost => "at most",
                    };
                    (
                        format!("{} ({})", mode.label(), word),
                        Style::new().fg(theme::CYAN),
                    )
                }
                None => (
                    format!("→ {}", state.tool_default.label()),
                    Style::new().fg(theme::MUTED),
                ),
            };
            (name.to_string(), status, st)
        };
        if selected {
            lines.push(Line::from(Span::styled(
                trunc(&format!("▸ {:<8} {}", label, status), (inner.width as usize).saturating_sub(1)),
                Style::new()
                    .fg(theme::BG)
                    .bg(theme::GOLD)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(vec![
                Span::styled(format!("  {:<8} ", label), Style::new().fg(theme::TEXT)),
                Span::styled(status, status_st),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The V.I.T.R.I.O.L. acrostic — the full classical motto, carved into
/// the UI's quiet corners (owner request 2026-09-03: the complete string,
/// all caps). Tactical shortening rule (owner): when width runs out, whole
/// words drop from the TAIL, word for word — never mid-word cuts.
const MOTTO_WORDS: &[&str] = &[
    "VISITA",
    "INTERIOREM",
    "TERRAE",
    "RECTIFICANDO",
    "INVENIES",
    "OCCULTUM",
    "LAPIDEM",
];

/// Longest whole-word prefix of the motto fitting `width`. None below the
/// first word — decoration never fights for space.
fn motto_for(width: usize) -> Option<String> {
    let mut cur = String::new();
    for w in MOTTO_WORDS {
        let candidate = if cur.is_empty() {
            (*w).to_string()
        } else {
            format!("{cur} {w}")
        };
        if candidate.chars().count() > width {
            break;
        }
        cur = candidate;
    }
    if cur.is_empty() {
        None
    } else {
        Some(cur)
    }
}

/// Carved ink — the same faint cut as the watermark stone.
fn carved(text: &str) -> Span<'static> {
    Span::styled(
        text.to_string(),
        Style::new()
            .fg(theme::WATERMARK)
            .add_modifier(Modifier::DIM),
    )
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Braille left-column bits for one quicksilver gauge row. The mercury
/// column is BOTTOM-anchored: it rises from the chat area's bottom edge as
/// you scroll up (owner correction 2026-09-03 — the first implementation
/// indexed rows from the top and filled downward). The partial edge cell
/// sits just above the filled block; its lower dots light first
/// (dot7=0x40, dot3=0x04, dot2=0x02, dot1=0x01; full column 0x47).
fn gauge_row_bits(row: usize, h: usize, fill: f64) -> u32 {
    let f = fill.clamp(0.0, h as f64);
    let full = f.floor() as usize;
    let edge = f - full as f64;
    if row >= h - full {
        0x47 // filled block — the bottom `full` rows
    } else if h - full - row == 1 && edge > 0.01 {
        // the cell just above the block: partial fill rises through it
        let dots = (edge * 4.0).round() as u32;
        [0x00, 0x40, 0x44, 0x46, 0x47][dots.min(4) as usize]
    } else {
        0
    }
}

/// /help modal — sidebar glossary, keys, commands (owner request
/// 2026-09-03). Commands render FROM AppState::LOCAL_COMMANDS so they
/// cannot drift. ↑↓/jk scrolls, esc/enter/q closes.
fn render_help_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    use crate::tui::state::{HelpRowKind, help_rows};
    let rows = help_rows();
    let h = area.height.min(24);
    let w = area.width.min(78);
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
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} help ", theme::GLYPH_ALEMBIC),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ scroll · esc close ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let visible = inner.height as usize;
    let sel = state.help_sel;
    let skip = if sel + 1 > visible { sel + 1 - visible } else { 0 };
    let label_w = 16usize;

    let mut lines: Vec<Line> = Vec::new();
    for row in rows.iter().skip(skip.min(rows.len())) {
        match row.kind {
            HelpRowKind::Header => {
                if !lines.is_empty() {
                    lines.push(Line::from(""));
                }
                lines.push(Line::from(Span::styled(
                    row.left.clone(),
                    Style::new()
                        .fg(theme::GOLD)
                        .bg(theme::PANEL)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            HelpRowKind::Item => {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("{:<width$} ", row.left, width = label_w),
                        Style::new().fg(theme::CYAN),
                    ),
                    Span::styled(row.right.clone(), Style::new().fg(theme::TEXT)),
                ]));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}


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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::state::AppState;

    #[test]
    fn shorten_home_tilde_prefix() {
        assert_eq!(shorten_home(std::path::Path::new("/home/randozart/Projects/VITRIOL")), "~/Projects/VITRIOL");
        assert_eq!(shorten_home(std::path::Path::new("/home/randozart")), "~");
        assert_eq!(shorten_home(std::path::Path::new("/opt/stuff")), "/opt/stuff");
    }

    #[test]
    fn session_label_named_unnamed_none() {
        let mut s = AppState::default();
        assert_eq!(session_label(&s), None);
        s.cwd = std::path::PathBuf::from("/home/randozart");
        s.session_id = "01a06380-1234".into();
        assert_eq!(session_label(&s).unwrap(), "SESSION ID: #01a06380 @ ~");
        s.cwd = std::path::PathBuf::from("/home/randozart/Projects/ontic");
        assert_eq!(session_label(&s).unwrap(), "SESSION ID: #01a06380 @ ~/Projects/ontic");
        s.session_name = Some("vitriol session".into());
        assert_eq!(session_label(&s).unwrap(), "\"vitriol session\" @ ~/Projects/ontic");
    }

    #[test]
    fn elide_keeps_prefix_and_path_tail() {
        let l = "SESSION ID: #01a06380 @ ~/Projects/VITRIOL";
        assert_eq!(elide_path_tail(l, 60), l); // fits — untouched
        let e = elide_path_tail(l, 34);
        assert!(e.starts_with("SESSION ID: #01a06380 @ …"), "got {}", e);
        assert!(e.chars().count() <= 34);
        assert!(e.ends_with("VITRIOL"), "component preserved: {}", e);
        // No " @ " separator → hard truncation to the cap.
        assert_eq!(elide_path_tail("plainlabel", 4), "plai");
    }
}

#[cfg(test)]
mod gauge_tests {
    use super::gauge_row_bits;

    #[test]
    fn gauge_rises_from_the_bottom() {
        // Empty — nothing lit anywhere.
        for row in 0..10 {
            assert_eq!(gauge_row_bits(row, 10, 0.0), 0);
        }
        // fill 3 of 10 — ONLY the bottom three rows, full cells.
        for row in 0..7 {
            assert_eq!(gauge_row_bits(row, 10, 3.0), 0, "row {row} must stay dark");
        }
        for row in 7..10 {
            assert_eq!(gauge_row_bits(row, 10, 3.0), 0x47);
        }
        // Fractional fill — the edge cell sits just ABOVE the block, lower
        // dots first: fill 3.5 → row 6 partial with two lowest dots (0x44).
        assert_eq!(gauge_row_bits(6, 10, 3.5), 0x44);
        assert_eq!(gauge_row_bits(5, 10, 3.5), 0);
        // Quarter fill — bottom dot only, in the bottom-most row.
        assert_eq!(gauge_row_bits(9, 10, 0.25), 0x40);
        assert_eq!(gauge_row_bits(8, 10, 0.25), 0);
        // Completely full — every row, no edge cell artifacts.
        for row in 0..10 {
            assert_eq!(gauge_row_bits(row, 10, 10.0), 0x47);
        }
    }
}

#[cfg(test)]
mod motto_tests {
    use super::motto_for;

    #[test]
    fn motto_shortens_word_for_word() {
        let full = "VISITA INTERIOREM TERRAE RECTIFICANDO INVENIES OCCULTUM LAPIDEM";
        assert_eq!(motto_for(64).as_deref(), Some(full));
        assert_eq!(motto_for(200).as_deref(), Some(full));
        // Tail words drop whole — never mid-word.
        assert_eq!(motto_for(56).as_deref(), Some("VISITA INTERIOREM TERRAE RECTIFICANDO INVENIES OCCULTUM"));
        assert_eq!(motto_for(38).as_deref(), Some("VISITA INTERIOREM TERRAE RECTIFICANDO"));
        assert_eq!(motto_for(24).as_deref(), Some("VISITA INTERIOREM TERRAE"));
        assert_eq!(motto_for(17).as_deref(), Some("VISITA INTERIOREM"));
        assert_eq!(motto_for(6).as_deref(), Some("VISITA"));
        assert_eq!(motto_for(5), None);
        assert_eq!(motto_for(0), None);
        // Every returned form fits its width.
        for w in 0..=70usize {
            if let Some(m) = motto_for(w) {
                assert!(m.chars().count() <= w, "width {w}: {:?} overflows", m);
            }
        }
    }
}
