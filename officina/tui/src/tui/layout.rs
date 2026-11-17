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
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::ansi;
use super::settings;
use super::state::{AppState, ChatEntry, DialogKind};
use crate::theme;

const SIDEBAR_W: u16 = 42;
/// Below this terminal width the sidebar is hidden automatically so the
/// chat column stays usable (phone/SSH sessions, owner request 2026-11-10).
const SIDEBAR_MIN_WIDTH: u16 = 100;

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let area = frame.area();

    // A notice gets its OWN row above the keybinding strip (2026-11-16: it used
    // to be appended to the hints line, where a long or multi-line notice
    // collided with the hints and made that row shimmer). The row is reserved
    // from the first notice onward — sticky, never released — so notices cannot
    // change the layout after the first one arrives.
    state.notice_row_sticky |= state.notice.is_some();
    let notice_h = u16::from(state.notice_row_sticky);
    let rows = Layout::vertical([
        Constraint::Length(1),        // header
        Constraint::Length(1),        // breathing room (owner request 2026-09-03)
        Constraint::Min(0),           // body
        Constraint::Length(notice_h), // notice strip (0 until the first notice)
        Constraint::Length(1),        // footer (hints only)
    ])
    .split(area);

    render_header(frame, rows[0], state);
    render_body(frame, state, rows[2]);
    if notice_h > 0 {
        render_notice(frame, rows[3], state);
    }
    render_footer(frame, rows[4], state);
}

// ── Header ────────────────────────────────────────────────────────────────

/// Canonical mode → (alchemical symbol, color): the moon metal for PLAN,
/// the sun metal for BUILD (owner request 2026-09-02). Custom modes → None.
fn mode_chip(label: &str) -> Option<(&'static str, ratatui::style::Color)> {
    match label.to_ascii_uppercase().as_str() {
        "PLAN" => Some(("☽", theme::SILVER)),
        "BUILD" => Some(("☉", theme::GOLD)),
        "INQUIRE" => Some(("?", theme::GREEN)),
        "STEER" => Some(("↝", theme::ORANGE)),
        "GRIND" => Some(("∞", theme::VIOLET)),
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
        Span::styled(
            " · tab to switch agent",
            Style::new().fg(theme::MUTED).bg(bg),
        ),
    ]
}

/// The composer title: the active agent mode (owner request 2026-09-02 —
/// the chip lives above the prompt box, not in the header). None until the
/// agent-mode widget reports in → titleless border.
fn composer_title(state: &AppState) -> Option<Vec<Span<'static>>> {
    let mut pending_chip: Option<Vec<Span<'static>>> = None;
    if !state.pending_images.is_empty() {
        pending_chip = Some(vec![Span::styled(
            format!(" ⎙{}", state.pending_images.len()),
            Style::new().fg(theme::GOLD).bg(theme::PANEL),
        )]);
    }
    let Some(label) = state.agent_mode_label.as_deref() else {
        // No agent chip yet — the image chip alone still deserves the title.
        return pending_chip;
    };
    if label.is_empty() {
        return pending_chip;
    }
    let mut spans = match mode_chip(label) {
        Some((symbol, color)) => mode_spans(symbol, label, color, theme::PANEL),
        // Custom mode: the widget's own badge glyph, gold chip.
        None => {
            let glyph = state
                .agent_mode_glyph
                .clone()
                .unwrap_or_else(|| theme::GLYPH_VITRIOL.to_string());
            mode_spans(&glyph, label, theme::GOLD, theme::PANEL)
        }
    };
    if let Some(mut chip) = pending_chip {
        spans.append(&mut chip);
    }
    Some(spans)
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
    let total = crate::tui::width::cols(&label);
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
    let mut left = vec![Span::styled("🜖 VITRIOL·OFFICINA", theme::banner())];
    if state.is_streaming {
        // Light yellow, the informational voice (owner request 2026-09-02):
        // same tint the engine TUI uses for "prompt-eval …". Animated with
        // a braille spinner on real activity (any lane — local or cloud);
        // when evidence goes quiet the label says so instead of lying.
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let f = (state.started.elapsed().as_millis() / 120) as usize % frames.len();
        if state.liveliness_active() {
            left.push(Span::styled(
                format!("  {} {} working {}", theme::GLYPH_FIRE, frames[f], {
                    let secs = state.turn_elapsed_secs();
                    if secs > 0 {
                        format!("{secs}s")
                    } else {
                        String::new()
                    }
                }),
                theme::info(),
            ));
        } else {
            left.push(Span::styled(
                format!(
                    "  {} working · stalled {}s",
                    theme::GLYPH_FIRE,
                    state.stalled_secs()
                ),
                Style::new().fg(theme::MUTED),
            ));
        }
    }

    // Formatted NAME + provider (2026-11-16): the header used to show the raw
    // model id, which told the operator nothing about who serves it.
    let model = state
        .model
        .as_ref()
        .map(AppState::model_label)
        .unwrap_or_default();
    let right = session_label(state)
        .map(|l| elide_path_tail(&l, (area.width.saturating_sub(24) as usize).max(16)))
        .unwrap_or_default();

    // Right-align the session label; model id sits before it.
    let right_w = (crate::tui::width::cols(&right) + 2) as u16;
    let mid_w = if right.is_empty() {
        area.width
    } else {
        area.width.saturating_sub(right_w)
    };
    let cols = Layout::horizontal([Constraint::Min(10), Constraint::Length(right_w)]).split(Rect {
        width: area.width,
        ..area
    });

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

    frame.render_widget(
        Paragraph::new(line),
        Rect {
            width: mid_w,
            ..cols[0]
        },
    );
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
    let show_sidebar = match state.sidebar_force {
        Some(v) => v,
        None => area.width >= SIDEBAR_MIN_WIDTH,
    };
    if !show_sidebar {
        // Clear the stale hit-box so wheel events over the (now full-width)
        // chat scroll the transcript, not a hidden sidebar.
        state.last_sidebar_area = None;
        render_main(frame, state, area);
        return;
    }

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
    // Extension dialogs are top priority: a pending trust/permission/plan
    // question must not be hidden behind another overlay (2026-11-10).
    if !state.dialogs.is_empty() {
        render_chat_and_editor(frame, state, area);
        render_dialog_modal(frame, state, area);
        return;
    }
    if state.perm_modal_open {
        render_chat_and_editor(frame, state, area);
        render_permissions_modal(frame, state, area);
        return;
    }
    if state.setting_menu.is_some() {
        render_chat_and_editor(frame, state, area);
        let menu_area = Rect {
            height: if area.height >= 10 {
                area.height - 4
            } else {
                area.height
            },
            ..area
        };
        settings::render(frame, state.setting_menu.as_ref().unwrap(), menu_area);
        return;
    }
    if state.show_diag {
        render_diag_overlay(frame, state, area);
        return;
    }
    if state.resume_open {
        render_chat_and_editor(frame, state, area);
        render_resume_modal(frame, state, area);
        return;
    }
    if state.tree_open {
        render_chat_and_editor(frame, state, area);
        render_tree_modal(frame, state, area);
        return;
    }
    if state.models_open {
        render_chat_and_editor(frame, state, area);
        render_models_modal(frame, state, area);
        return;
    }
    if state.cache_open {
        render_chat_and_editor(frame, state, area);
        render_cache_modal(frame, state, area);
        return;
    }
    if state.fx_open {
        render_chat_and_editor(frame, state, area);
        render_fx_modal(frame, state, area);
        return;
    }
    if state.answers_open {
        render_chat_and_editor(frame, state, area);
        render_answers_modal(frame, state, area);
        return;
    }
    if state.cost_open {
        render_chat_and_editor(frame, state, area);
        render_cost_modal(frame, state, area);
        return;
    }
    if state.routes_open {
        render_chat_and_editor(frame, state, area);
        render_routes_modal(frame, state, area);
        return;
    }
    if state.xsessions_open {
        render_chat_and_editor(frame, state, area);
        render_xsessions_modal(frame, state, area);
        return;
    }
    if state.review_open {
        render_chat_and_editor(frame, state, area);
        render_review_modal(frame, state, area);
        return;
    }
    if state.status_open {
        render_chat_and_editor(frame, state, area);
        render_status_modal(frame, state, area);
        return;
    }
    if state.caduceus.is_some() {
        render_caduceus_page(frame, state, area);
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

/// True when any of these band lines merely repeats the operator's draft, so
/// the strip would look like a second, read-only copy of the prompt (BAND-1,
/// 2026-11-17). A line counts as an echo when its trimmed text equals the draft
/// or is a prefix of it (a prediction is a continuation, so mid-typing it can
/// look identical for several keystrokes).
fn band_echoes_draft(lines: &[String], draft: &str) -> bool {
    let d = draft.trim();
    if d.is_empty() {
        return false;
    }
    lines.iter().any(|l| {
        let l = l.trim();
        !l.is_empty() && (l == d || d.starts_with(l) || l.starts_with(d))
    })
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

    // belowEditor indicators (plan/deep-research/phase-model/vitriol-decode)
    // get a compact strip between the transcript and the prompt. agent-mode
    // already owns the composer title; engine-fire is the flame data feed.
    let mut indicator_blocks: Vec<&crate::tui::state::WidgetBlock> = state
        .widgets
        .iter()
        .filter(|w| w.placement.as_deref() == Some("belowEditor"))
        .filter(|w| w.key != "agent-mode" && w.key != "engine-fire")
        // BAND-1 (2026-11-17 owner report): a belowEditor widget may carry a
        // prediction that repeats the operator's draft, and then the band reads
        // as if their text escaped the composer. No band row may echo what is
        // being typed — the composer owns that text.
        .filter(|w| !band_echoes_draft(&w.lines, &state.input))
        .collect();
    // Route state is the most important strip (full harness can have more
    // belowEditor widgets than rows). Then plan/research/phase, decode last.
    indicator_blocks.sort_by_key(|w| match w.key.as_str() {
        // Route + live decode/progress are the operator's primary scan
        // targets; phase/plan/research indicators are secondary. Keeping
        // decode in the first three rows guarantees tk/s and log progress are
        // never pushed out by a transient indicator.
        "overflow-route" => 0,
        "vitriol-decode" => 1,
        "plan-mode" => 2,
        "deep-research-mode" => 3,
        "phase-model" => 4,
        _ => 9,
    });
    let indicators: Vec<String> = indicator_blocks
        .iter()
        .flat_map(|w| w.lines.iter().cloned())
        .filter(|l| !l.trim().is_empty())
        .take(3)
        .collect();

    let input_h = composer_box_height(&state.input, area.width);
    // Status band = belowEditor indicator rows (decode/route/phase tags) +
    // the context-fill row. Both parts derive from live data that can appear
    // for a single frame, so EACH PART is settled separately (2026-11-16):
    // settling only the total let a transient indicator line take the ctx
    // row's slot for a frame — "the ctx field flickers to look like a
    // different field". With parts settled, a transient line simply does not
    // render, and the ctx row never moves.
    let ctx_present = state.ctx_bar_label().is_some();
    let (ind_h, ctx_h) =
        state.stabilize_band_parts(indicators.len() as u16, u16::from(ctx_present));
    let band_h = ind_h + ctx_h;
    let chat_area = Rect {
        height: area.height.saturating_sub(input_h + 1 + band_h),
        ..area
    };
    let band_area = Rect {
        y: area.y + chat_area.height + 1,
        height: band_h,
        ..area
    };
    let indicator_area = Rect {
        height: ind_h,
        ..band_area
    };
    // The ctx row is BOTTOM-anchored: its position is a function of the
    // settled heights only, so indicator churn can never shift it.
    let ctx_area = Rect {
        y: band_area.y + band_h - ctx_h,
        height: ctx_h,
        ..area
    };
    debug_assert!(ind_h + ctx_h == band_h);
    let input_area = Rect {
        y: band_area.y + band_h,
        height: input_h,
        ..area
    };
    let chat_width = (chat_area.width as usize).saturating_sub(1);
    // Selection mapping needs the viewport at ^c time (handle_key has no
    // Frame) — stash it every frame.
    state.last_chat_area = Some(chat_area);

    let mut lines = state.chat_lines(chat_width);

    if state.is_streaming {
        // Animated spinner + elapsed seconds (2026-11-13): works on ANY
        // lane — the spinner runs on stream evidence, so cloud turns
        // animate exactly like local ones. Stalled → muted, says so.
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let f = (state.started.elapsed().as_millis() / 120) as usize % frames.len();
        let (style, suffix) = if state.liveliness_active() {
            let secs = state.turn_elapsed_secs();
            (
                theme::info(),
                if secs > 0 {
                    format!(" {secs}s")
                } else {
                    String::new()
                },
            )
        } else {
            (
                Style::new().fg(theme::MUTED),
                format!(" · stalled {}s", state.stalled_secs()),
            )
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} {} ", theme::GLYPH_AI, frames[f]),
                Style::default().fg(theme::GOLD).bg(theme::BG),
            ),
            // Light yellow to match the header (owner request 2026-09-02 —
            // the informational voice, like the engine TUI's prompt-eval).
            Span::styled(format!("working…{suffix}"), style),
        ]));
    }
    if state.is_compacting {
        // Compaction is a status, not a warning — informational voice.
        // Spinner + elapsed so it never reads as a freeze (2026-11-13).
        let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
        let f = (state.started.elapsed().as_millis() / 120) as usize % frames.len();
        let secs = state
            .compaction_started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        lines.push(Line::from(Span::styled(
            format!(
                "  {} {} compacting context… {secs}s",
                theme::GLYPH_ALEMBIC,
                frames[f]
            ),
            theme::info(),
        )));
    }
    // Trailing status lines have no entry — keep the map aligned (hitboxes).
    state.line_entries.resize(lines.len(), usize::MAX);

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
    // Scroll lock (2026-11-13, operator request): while scrolled back the
    // top line is pinned by absolute index — streaming below never drags
    // the text being read. Reaching the tail re-engages follow mode.
    let max_start = total.saturating_sub(visible);
    let start = if state.scroll == 0 {
        state.scroll_lock = None;
        max_start
    } else {
        let anchor = state.scroll_lock.unwrap_or(state.last_start);
        let locked = anchor.min(max_start);
        if locked >= max_start {
            state.scroll_follow();
            max_start
        } else {
            state.scroll_lock = Some(locked);
            locked
        }
    };
    state.last_start = start;
    state.last_chat_total = total;
    // Only the visible window is materialised (2026-11-14 perf: long
    // transcripts used to clone thousands of out-of-view lines per frame).
    let view: Vec<Line> = lines
        .into_iter()
        .skip(start)
        .take(visible)
        .map(|l| elide_line(l, chat_width))
        .collect();

    // Scrolled-back hint + click target (2026-11-13): the breathing row
    // between transcript and prompt says the view is locked and offers a
    // one-click jump back to the live tail.
    state.tail_hint_area = None;
    if chat_area.height > 0 {
        let hint = Rect {
            y: chat_area.y + chat_area.height,
            height: 1,
            ..chat_area
        };
        if let Some(held) = state.held_label() {
            // Held messages (2026-11-13, operator request): what was typed
            // while the agent is busy, until the agent reaches it.
            frame.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    format!("  ⏳ held — {held} · inserted when the agent reaches it"),
                    Style::new().fg(theme::GOLD).bg(theme::BG),
                ))),
                hint,
            );
        } else if state.scroll > 0 {
            frame.render_widget(
                ratatui::widgets::Paragraph::new(Line::from(Span::styled(
                    format!(
                        "  {}▲ scrolled back {} rows — press END to go back to bottom",
                        theme::GLYPH_ALEMBIC,
                        state
                            .last_chat_total
                            .saturating_sub(state.last_start + visible as usize)
                    ),
                    Style::new().fg(theme::MUTED).bg(theme::BG),
                ))),
                hint,
            );
            state.tail_hint_area = Some(hint);
        }
    }

    // Thinking-block hitboxes (2026-11-13): a left click on any row of a
    // block toggles collapse. Rows are screen coords of the visible
    // window; rebuilt every frame from the line→entry map.
    state.thinking_hitboxes.clear();
    state.tool_hitboxes.clear();
    if visible > 0 {
        let last_visible = (start + visible - 1).min(state.line_entries.len().saturating_sub(1));
        let mut i = 0usize;
        while i < state.line_entries.len() {
            let idx = state.line_entries[i];
            let mut j = i;
            while j + 1 < state.line_entries.len() && state.line_entries[j + 1] == idx {
                j += 1;
            }
            let kind = state.entries.get(idx).map(|e| match e {
                ChatEntry::Thinking { .. } => 1u8,
                ChatEntry::Tool { .. } => 2,
                _ => 0,
            });
            if let Some(kind) = kind.filter(|k| *k != 0) {
                let lo = i.max(start);
                let hi = j.min(last_visible);
                if lo <= hi {
                    let range = (
                        chat_area.y + (lo - start) as u16,
                        chat_area.y + (hi - start) as u16,
                        idx,
                    );
                    if kind == 1 {
                        state.thinking_hitboxes.push(range);
                    } else {
                        state.tool_hitboxes.push(range);
                    }
                }
            }
            i = j + 1;
        }
    }

    // Composer flames (owner request 2026-09-02): braille alchemical fire
    // rising from the editor's top edge; intensity = GPU load (low-passed
    // in the run loop). /fire toggles it. TEXT PRIORITY (owner request,
    // after live testing): the fire is drawn BEFORE the chat so the
    // transcript renders on top — flames read as a backdrop, visible only
    // in the gaps, never over a character.
    //
    // 2026-11-16 (owner): the fire IGNITES ON THE DIVIDER ROW. The braille
    // field sits strictly above it; the divider lights per column (⡀) where
    // the flame has reach, so a dead fire is a plain underscore rule and a
    // live one grows out of it. Height comes through `rows_for_stable` so a
    // breathing level cannot flap the strip (that flapping re-coloured a
    // whole row of text).
    let fire_rows = if state.fire_on {
        crate::fire::rows_for_stable(state.fire_level, state.fire_rows_prev)
    } else {
        0
    };
    state.fire_rows_prev = fire_rows;
    // The divider row is the base; the glyph field gets the remaining rows
    // (it used to spend one on a base row the divider then erased).
    let ign_y = area.y + chat_area.height;
    let flame_rows = if fire_rows > 0 {
        fire_rows.saturating_sub(1).max(1)
    } else {
        0
    };
    let fire_map = if flame_rows > 0 && flame_rows as u16 <= ign_y {
        crate::fire::render(
            frame,
            Rect {
                y: ign_y.saturating_sub(flame_rows as u16),
                height: flame_rows as u16,
                ..input_area
            },
            state.started.elapsed().as_millis(),
            state.fire_level,
            state.fire_style,
            state.fire_taper,
        )
    } else {
        Vec::new()
    };
    let fire_base_y = ign_y.saturating_sub(fire_map.len() as u16);

    // Question panel: drawn LAST (see the tail of this function) so the
    // transcript can never paint over its upper rows.

    // Gap row: explicit BG paint, ALWAYS (owner request 2026-09-03 — the
    // terminal background must never peek through below the panels). It now
    // carries the DIVIDER (owner request 2026-11-11): a 1px dim rule in the
    // sidebar/ctx-bar convention (BORDER_DIM) cordonning the bottom status
    // band (slots/engine/ctx rows) off from the transcript above — the
    // user-space band vs the log the agent writes into. Fire dots rising
    // through the gap keep their flame fg because set_style patches bg only.
    // 2026-11-16: underscore (not box-drawing) so the line hugs the status
    // band below it, reading as an underline rather than a floating rule —
    // and it is the fire's ignition line: with flames up, the columns that
    // have reach are replaced by a thin braille dot in the flame's base
    // colour. The selection is hashed once a second, so the line drifts
    // slowly and never flickers.
    {
        let gap = Rect {
            y: area.y + chat_area.height,
            height: 1,
            ..area
        };
        let w = gap.width as usize;
        let phase_ms = state.started.elapsed().as_millis();
        // The rule IS the fire's bottom row (2026-11-16): a column the flame
        // claims shows the flame's own base glyph, everything else keeps the
        // underscore. `flame_rows` is the field height above this row, so the
        // two agree on where the bottom row sits.
        let lit = crate::fire::line_cells(w, flame_rows.max(1), phase_ms, state.fire_level);
        let flame = crate::fire::ignition_color(phase_ms, state.fire_level, state.fire_style);
        let line = Line::from(ignition_spans(w, &lit, flame));
        frame.render_widget(Paragraph::new(line).style(Style::new().bg(theme::BG)), gap);
    }

    // Band surface first: with the height debounced, a row can be reserved
    // one frame longer than its content — it must read as the panel's own
    // tone, never as raw terminal background.
    if band_area.height > 0 {
        frame.render_widget(
            Paragraph::new(" ").style(Style::new().bg(theme::PANEL)),
            band_area,
        );
    }

    if ind_h > 0 {
        let lines: Vec<Line> = indicators.iter().map(|l| ansi::parse_line(l)).collect();
        // Dedicated status-band surface (owner request 2026-11-10): the
        // three below-editor lines sit on the panel tone, and the flames
        // start at this band's top edge instead of under it.
        frame.render_widget(
            Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
            indicator_area,
        );
    }

    // Context fill row (owner request 2026-11-11): a one-row strip in the
    // status band with the sidebar's divider style — a dim horizontal rule
    // on the panel tone separating it from the breathing row.
    if let Some(label) = state.ctx_bar_label() {
        let w = ctx_area.width as usize;
        let text = format!(" {} ", label);
        let rule_len = w
            .saturating_sub(crate::tui::width::cols(&text).saturating_add(2))
            .max(2);
        let mut spans = vec![
            Span::styled(format!("{:^w$}", "", w = 1), Style::default()),
            Span::styled(label, Style::new().fg(theme::MUTED)),
            Span::styled(" ", Style::default()),
        ];
        // Dim horizontal rule filling the rest of the row (sidebar divider
        // convention: BORDER_DIM on PANEL).
        spans.push(Span::styled(
            "─".repeat(rule_len),
            Style::new().fg(theme::BORDER_DIM),
        ));
        let line = Line::from(spans);
        frame.render_widget(
            Paragraph::new(line).style(Style::new().bg(theme::PANEL)),
            ctx_area,
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

    // User-turn band (owner request 2026-11-16): paint the rows that belong to
    // a USER entry, full width minus the gauge column. Drawn AFTER the chat
    // paragraph (so it replaces BG) and BEFORE the fire tint (which writes fg
    // only) and the selection (which must win on selected rows). Blank
    // separator rows are skipped: `chat_lines` tags the gap row with the current
    // entry's index, and a tinted gap under every turn would read as stripes.
    if state.user_turn_bg && !state.entries.is_empty() {
        let start = state.last_start;
        let visible = chat_area.height as usize;
        let x_end = chat_area.x + chat_area.width.saturating_sub(1);
        let buffer = frame.buffer_mut();
        for row in 0..visible {
            let idx = state
                .line_entries
                .get(start + row)
                .copied()
                .unwrap_or(usize::MAX);
            if !matches!(
                state.entries.get(idx),
                Some(crate::tui::state::ChatEntry::User(_))
            ) {
                continue;
            }
            let y = chat_area.y + row as u16;
            let blank = (chat_area.x..x_end).all(|x| buffer[(x, y)].symbol().trim().is_empty());
            if blank {
                continue;
            }
            for x in chat_area.x..x_end {
                if let Some(cell) = buffer.cell_mut(ratatui::layout::Position { x, y }) {
                    cell.set_bg(theme::USER_BG);
                }
            }
        }
    }

    // Text-tint pass (owner request 2026-09-02: "user text discolors based
    // on the fire beneath it — do the same for AI text"). Unstyled user
    // spans already inherit the flame fg via ratatui's style-patch
    // semantics; markdown-styled AI spans don't. Walk the fire map and
    // force the flame color onto any text glyph inside the strip — flames
    // lend their color to everything standing in them.
    //
    // 2026-09-03: `/fire tint off` keeps text its original color — EXCEPT
    // the carved motto (owner: "make the VISITA INTERIOREM text still
    // discolor as the one exception"). The motto lived in the gap row,
    // which is the fire strip's bottom row — so with tint off, only that
    // row was walked.
    // 2026-09-04: the carved motto is gone (redundant with the input
    // placeholder); with no glyphs in the gap row, tint-off now leaves
    // every fire row alone.
    if !fire_map.is_empty() {
        let tint_all = state.fire_tint;
        // The mask's row 0 is the TOP of the braille field (one row above
        // the divider since 2026-11-16).
        let fire_y = fire_base_y;
        let buffer = frame.buffer_mut();
        for (ry, map_row) in fire_map.iter().enumerate() {
            if !tint_all {
                continue; // no motto exception anymore — tint off = text untouched
            }
            for (cx, c) in map_row.iter().enumerate() {
                if let Some(color) = c {
                    if let Some(cell) = buffer.cell_mut(ratatui::layout::Position {
                        x: input_area.x + cx as u16,
                        y: fire_y + ry as u16,
                    }) {
                        // PANEL-surfaced cells (tool blocks) are opaque
                        // plates standing IN the fire, not of it — the
                        // flames never discolor them.
                        if cell.symbol() != " " && cell.bg != theme::PANEL {
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
        let bw = crate::tui::width::cols(&badge) as u16;
        if chat_area.width > bw {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(badge, theme::muted())))
                    .alignment(Alignment::Right),
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
        // +2 rows for the selected-command detail line (owner request
        // 2026-09-03: menu rows truncate — the detail carries the rest).
        let popup_h = (cands.len() as u16 + 2).min(max_rows) + 2;
        let popup_area = Rect {
            y: input_area.y.saturating_sub(popup_h),
            height: popup_h,
            width: area.width.min(72),
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
    let line = if let Some(ghost) = state
        .prompt_ghost
        .as_ref()
        .filter(|_| state.input.is_empty())
    {
        // Voice-suggestion ghost (stage 2): dimmed gold, cursor block on the
        // first glyph — same shape as the motto, but a suggestion you can
        // accept with →. Render-only; nothing is ever pre-sent (I1).
        let budget = (inner.width as usize).saturating_sub(2);
        let text: String = ghost.text.chars().take(budget).collect();
        let mut chars = text.chars();
        let first = chars.next().unwrap_or(' ');
        let rest: String = chars.collect();
        Line::from(vec![
            prompt,
            Span::styled(
                first.to_string(),
                Style::default()
                    .fg(theme::GOLD)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::REVERSED | Modifier::DIM),
            ),
            Span::styled(
                rest,
                Style::default()
                    .fg(theme::GOLD)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::DIM),
            ),
        ])
    } else if state.input.is_empty() {
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
        composer_line(&state.input, state.cursor)
    };
    // The composer WRAPS: a long line grows the box AND flows onto further
    // rows. Rendering a bare Line clipped the tail while the box grew
    // (bug 2026-11-11).
    frame.render_widget(
        Paragraph::new(Text::from(vec![line]))
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme::PANEL)),
        inner,
    );

    // Question panel last of all: it is an overlay on top of everything the
    // transcript, band and composer paint (2026-11-16).
    render_question_panel(frame, state, area, input_area);
}

/// Divider-row spans: `_` for unlit columns, the thin braille dot for lit
/// ones, run-length grouped so a wide row costs a handful of spans instead of
/// one per column (2026-11-16).
fn ignition_spans(
    width: usize,
    lit: &[Option<char>],
    flame: ratatui::style::Color,
) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_lit = false;
    for x in 0..width {
        let glyph = lit.get(x).copied().flatten();
        let is_lit = glyph.is_some();
        if !run.is_empty() && is_lit != run_lit {
            out.push(ignition_span(std::mem::take(&mut run), run_lit, flame));
        }
        run_lit = is_lit;
        run.push(glyph.unwrap_or('_'));
    }
    if !run.is_empty() {
        out.push(ignition_span(run, run_lit, flame));
    }
    out
}

/// One coloured run of the divider row.
fn ignition_span(text: String, lit: bool, flame: ratatui::style::Color) -> Span<'static> {
    let fg = if lit { flame } else { theme::BORDER_DIM };
    Span::styled(text, Style::new().fg(fg).bg(theme::BG))
}

/// Question panel (2026-11-13) — the ask extension stages question.json and
/// the operator's menu is drawn where their eyes already are: just above the
/// prompt. Rendered LAST of the chat screen (2026-11-16 operator report: the
/// panel used to be painted before the transcript and composer, so their
/// rows drew over the menu's upper border — it seemed to open BEHIND the
/// text). Enter answers, esc cancels, digits quick-pick; it disappears the
/// moment the answer is written.
fn render_question_panel(frame: &mut Frame, state: &AppState, area: Rect, input_area: Rect) {
    let Some(q) = state.question.as_ref() else {
        return;
    };
    let panel_w = area.width.min(72);
    let opt_h = q.options.len() as u16;
    let panel_h = (opt_h + 5).min(area.height.saturating_sub(4));
    let panel = Rect {
        x: area.x + area.width.saturating_sub(panel_w) / 2,
        y: input_area.y.saturating_sub(panel_h + 1),
        width: panel_w,
        height: panel_h,
    };
    frame.render_widget(ratatui::widgets::Clear, panel);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GREEN).bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} question ", theme::GLYPH_AI),
            Style::new()
                .fg(theme::GREEN)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(panel);
    frame.render_widget(block, panel);
    if inner.height < 1 || inner.width < 20 {
        return;
    }
    let mut qlines: Vec<Line> = Vec::new();
    let qw = inner.width.saturating_sub(2) as usize;
    for seg in crate::tui::state::wrap_text(&q.question, qw) {
        qlines.push(Line::from(Span::styled(
            format!(" {seg}"),
            Style::new().fg(theme::TEXT).bg(theme::PANEL),
        )));
    }
    for (i, opt) in q.options.iter().enumerate() {
        let sel = i == state.question_sel;
        let marker = if sel { "▸" } else { " " };
        let st = if sel {
            Style::new().fg(theme::BG).bg(theme::GREEN)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        qlines.push(Line::from(Span::styled(
            format!(" {marker}{} {opt}", i + 1),
            st,
        )));
    }
    qlines.truncate(inner.height as usize);
    frame.render_widget(
        Paragraph::new(qlines).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

/// /permissions modal: ask / auto / yolo with the protected-roots note.
fn render_permissions_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = (AppState::PERM_MODES.len() as u16 + 5).min(area.height.saturating_sub(2));
    let w = area.width.clamp(30, 78);
    let modal = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            " permissions ",
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ move · enter set · esc ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 3 {
        return;
    }
    let width = inner.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (idx, (mode, desc)) in AppState::PERM_MODES.iter().enumerate() {
        let selected = idx == state.perm_modal_sel;
        let current = *mode == state.perm_mode;
        let base = if selected {
            Style::new()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        let muted = if selected {
            base
        } else {
            Style::new().fg(theme::MUTED).bg(theme::PANEL)
        };
        let marker = if selected { "▸ " } else { "  " };
        let check = if current { "● " } else { "  " };
        let name = format!("{:<6}", mode);
        let text = trunc(
            &format!("{marker}{check}{name} {desc}"),
            width.saturating_sub(1),
        );
        let mut spans = vec![Span::styled(text, base)];
        let used = crate::tui::width::cols(&spans[0].content);
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), base));
        }
        let _ = muted;
        lines.push(Line::from(spans));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        Rect {
            height: inner.height.saturating_sub(2),
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trunc("sensitive roots/secrets always prompt, even in YOLO", width),
            Style::new()
                .fg(theme::MUTED)
                .bg(theme::PANEL)
                .add_modifier(Modifier::ITALIC),
        ))),
        Rect {
            y: inner.y + inner.height - 1,
            height: 1,
            ..inner
        },
    );
}

fn render_dialog_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let Some(dialog) = state.dialogs.front() else {
        return;
    };
    let kind = dialog.kind;
    let option_rows = if kind == DialogKind::Select {
        dialog.options.len().min(12) as u16
    } else {
        0
    };
    let wanted = match kind {
        DialogKind::Confirm => 7,
        DialogKind::Select => (option_rows + 5).max(7),
        DialogKind::Input => 7,
        DialogKind::Editor => 12,
    };
    let h = wanted.min(area.height.saturating_sub(2)).max(5);
    let w = area.width.clamp(30, 82);
    let modal = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} ", trunc(&dialog.title, w.saturating_sub(6) as usize)),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 3 {
        return;
    }

    let hint = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    let body = Rect {
        height: inner.height - 1,
        ..inner
    };
    let width = body.width as usize;

    let hint_text = match kind {
        DialogKind::Confirm => " y/n or ←→ then enter · esc cancel ",
        DialogKind::Select => " ↑↓ move · enter choose · esc cancel ",
        DialogKind::Input => " enter submit · esc cancel ",
        DialogKind::Editor => " alt+enter submit · enter newline · esc cancel ",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint_text,
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        ))),
        hint,
    );

    let base = Style::new().fg(theme::TEXT).bg(theme::PANEL);
    let muted = Style::new().fg(theme::MUTED).bg(theme::PANEL);
    let accent = Style::new()
        .fg(theme::GOLD)
        .bg(theme::PANEL)
        .add_modifier(Modifier::BOLD);
    let selected = Style::new()
        .fg(theme::BG)
        .bg(theme::GOLD)
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line> = Vec::new();
    match kind {
        DialogKind::Select => {
            let visible = body.height as usize;
            let sel = dialog.selected;
            let skip = (sel + 1).saturating_sub(visible);
            for (i, option) in dialog.options.iter().enumerate().skip(skip) {
                if lines.len() >= visible {
                    break;
                }
                let is_sel = i == sel;
                let marker = if is_sel { "▸ " } else { "  " };
                let text = format!("{marker}{}", trunc(option, width.saturating_sub(3)));
                let style = if is_sel { selected } else { base };
                let mut spans = vec![Span::styled(text, style)];
                let used = crate::tui::width::cols(&spans[0].content) + 1;
                if used < width {
                    spans.push(Span::styled(" ".repeat(width - used + 1), style));
                }
                lines.push(Line::from(spans));
            }
            if skip > 0 || skip + visible < dialog.options.len() {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(" {} of {} ", sel + 1, dialog.options.len()),
                        muted,
                    ))),
                    hint,
                );
            }
        }
        DialogKind::Confirm => {
            if let Some(message) = &dialog.message {
                let mut msg: Vec<Line> = message
                    .lines()
                    .map(|l| Line::from(Span::styled(trunc(l, width), base)))
                    .collect();
                msg.truncate(body.height.saturating_sub(2) as usize);
                frame.render_widget(
                    Paragraph::new(msg).wrap(Wrap { trim: false }),
                    Rect {
                        height: body.height.saturating_sub(2),
                        ..body
                    },
                );
            }
            let yes = dialog.selected == 0;
            let mut spans = vec![
                Span::styled(
                    if yes { " ▸ Yes " } else { "   Yes " },
                    if yes { selected } else { base },
                ),
                Span::styled("   ", base),
                Span::styled(
                    if yes { "   No " } else { " ▸ No " },
                    if yes { base } else { selected },
                ),
            ];
            let used: usize = spans
                .iter()
                .map(|s| crate::tui::width::cols(&s.content))
                .sum();
            if used < width {
                spans.push(Span::styled(" ".repeat(width - used), base));
            }
            frame.render_widget(
                Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::PANEL)),
                Rect {
                    y: body.y + body.height.saturating_sub(1),
                    height: 1,
                    ..body
                },
            );
            return;
        }
        DialogKind::Input | DialogKind::Editor => {
            let cursor = dialog.cursor;
            let before: String = dialog.input.chars().take(cursor).collect();
            let after: String = dialog.input.chars().skip(cursor).collect();
            if kind == DialogKind::Input {
                let text = format!("{before}▏{after}");
                lines.push(Line::from(vec![Span::styled(trunc(&text, width), base)]));
            } else {
                let cur_line = before.split('\n').count();
                let mut rendered = 0usize;
                for (idx, raw) in dialog.input.split('\n').enumerate() {
                    if rendered >= body.height as usize {
                        break;
                    }
                    let text = if idx + 1 == cur_line {
                        let last = before.rsplit('\n').next().unwrap_or("");
                        let after_line = after.split('\n').next().unwrap_or("");
                        format!("{last}▏{after_line}")
                    } else {
                        raw.to_string()
                    };
                    lines.push(Line::from(Span::styled(trunc(&text, width), base)));
                    rendered += 1;
                }
                if rendered == 0 {
                    lines.push(Line::from(Span::styled("▏", base)));
                }
                let _ = cur_line;
            }
        }
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(Style::new().bg(theme::PANEL))
            .wrap(Wrap { trim: false }),
        body,
    );
    let _ = accent;
}

fn render_sidebar(frame: &mut Frame, state: &mut AppState, area: Rect) {
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
        // belowEditor widgets are composer-adjacent indicators, not sidebar
        // content (full extension load makes this distinction critical).
        if widget.placement.as_deref() == Some("belowEditor") {
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

    let visible = area.height.saturating_sub(2) as usize;
    let max = lines.len().saturating_sub(visible);
    state.sidebar_scroll_max = max.min(u16::MAX as usize) as u16;
    if state.sidebar_scroll as usize > max {
        state.sidebar_scroll = max.min(u16::MAX as usize) as u16;
    }
    let off = state.sidebar_scroll as usize;
    let more_below = off < max;

    // Rounded neutral panel, dim border. A small scroll position marker
    // appears only while the sidebar is scrolled (owner request 2026-11-10:
    // the panel can be taller than the screen).
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_DIM))
        .style(Style::new().bg(theme::PANEL));
    if off > 0 || more_below {
        let marker = format!(
            " {} ",
            if off > 0 {
                format!("↑{off}")
            } else {
                format!("↓{}", max - off)
            }
        );
        block = block.title_bottom(
            Line::from(Span::styled(marker, Style::new().fg(theme::MUTED)))
                .alignment(Alignment::Right),
        );
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    state.last_sidebar_area = Some(area);

    let view: Vec<Line> = lines
        .into_iter()
        .skip(off)
        .take(inner.height as usize)
        .collect();
    frame.render_widget(Paragraph::new(view).wrap(Wrap { trim: false }), inner);
}

// ── Popups / overlays ─────────────────────────────────────────────────────

fn render_command_popup(
    frame: &mut Frame,
    cands: &[crate::rpc::protocol::SlashCommand],
    sel: usize,
    area: Rect,
) {
    // The menu floats over the transcript. Clear first, then paint every row
    // on the panel tone — otherwise chat text shows through the gaps where a
    // row has no span (owner report 2026-11-10).
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL));

    let total = cands.len();
    let visible = (area.height as usize).saturating_sub(2);
    let title = if total > visible {
        format!(" commands {} · → completes ", total)
    } else {
        " commands · → completes ".to_string()
    };
    let block = block.title(Span::styled(title, Style::new().fg(theme::MUTED)));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Detail line: the selected command's FULL description, wrapped into
    // the bottom two rows. Local commands read their long form from
    // LOCAL_COMMANDS; RPC-backed commands use their own description.
    let detail_rect = Rect {
        height: 2.min(inner.height),
        y: inner.y + inner.height.saturating_sub(2),
        ..inner
    };
    let rows_rect = Rect {
        y: inner.y,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let detail = cands
        .get(sel)
        .map(|c| {
            AppState::LOCAL_COMMANDS
                .iter()
                .find(|(n, _, _)| *n == c.name)
                .map(|(_, _, long)| long.to_string())
                .or_else(|| c.description.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            detail,
            Style::new().fg(theme::CYAN).bg(theme::PANEL),
        )))
        .wrap(Wrap { trim: false })
        .style(Style::new().bg(theme::PANEL)),
        detail_rect,
    );

    let visible = rows_rect.height as usize;
    let skip = (sel + 1).saturating_sub(visible);
    let cands = &cands[skip.min(total)..];
    let mut lines: Vec<Line> = Vec::new();
    for (i, c) in cands.iter().enumerate() {
        let absolute = skip + i;
        let desc = c.description.clone().unwrap_or_default();
        let selected = absolute == sel;
        let (name_style, desc_style, marker) = if selected {
            (
                Style::new()
                    .fg(theme::GOLD)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::BOLD),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
                "▸ ",
            )
        } else {
            (
                Style::new().fg(theme::GOLD).bg(theme::PANEL),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
                "  ",
            )
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<14}", marker, c.name), name_style),
            Span::styled(
                trunc(&desc, (inner.width as usize).saturating_sub(18)),
                desc_style,
            ),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        rows_rect,
    );
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
        .map(|l| {
            Line::from(Span::raw(trunc(
                l,
                (inner.width as usize).saturating_sub(1),
            )))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

// ── Footer ────────────────────────────────────────────────────────────────

/// Notice strip (2026-11-16): one row, above the keybinding hints, showing the
/// notice's FIRST line with a `(+N lines)` tail when it has more. Width-aware
/// truncation — the old fixed-40 cut a /nimbus table down to noise.
fn render_notice(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some((head, kind)) = state.notice_head() else {
        frame.render_widget(Paragraph::new("").style(Style::new().bg(theme::BG)), area);
        return;
    };
    let color = match kind.as_str() {
        "error" => theme::RED,
        "warning" => theme::ORANGE,
        _ => theme::GOLD,
    };
    let prefix = "  · ";
    let avail = (area.width as usize).saturating_sub(crate::tui::width::cols(prefix) + 1);
    let text = if crate::tui::width::cols(&head) > avail {
        let mut cut: String = head.chars().take(avail.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        head
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prefix, Style::new().fg(theme::BORDER_DIM).bg(theme::BG)),
            Span::styled(text, Style::new().fg(color).bg(theme::BG)),
        ]))
        .style(Style::new().bg(theme::BG)),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut spans = vec![
        Span::styled("enter send", theme::muted()),
        Span::styled(" · ", Style::new().fg(theme::BORDER_DIM)),
        Span::styled("→ complete", theme::muted()),
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
    // The hints row is a pure function of the keybindings + telemetry presence:
    // NO notice text here (2026-11-16). Everything state-dependent lives on the
    // notice strip above, so this row cannot shimmer.
    if state.widgets.is_empty() {
        spans.push(Span::styled(
            "  ·  no telemetry",
            Style::new().fg(theme::MUTED).bg(theme::BG),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_resume_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area
        .height
        .min((state.resume_entries.len() as u16 + 3).max(8));
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
            if state.resume_filter.is_empty() {
                " ↑↓ select · enter resume · type to filter · esc cancel ".to_string()
            } else {
                format!(" /{} _ · ↑↓ · enter · esc clears ", state.resume_filter)
            },
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let visible = inner.height as usize;
    let filtered = state.filtered_resume_entries();
    let sel = state.resume_sel;
    let skip = (sel + 1).saturating_sub(visible);

    let mut lines: Vec<Line> = Vec::new();
    if filtered.is_empty() {
        lines.push(Line::from(Span::styled(
            if state.resume_filter.is_empty() {
                "  no sessions".to_string()
            } else {
                format!("  no sessions match '{}'", state.resume_filter)
            },
            Style::new().fg(theme::MUTED),
        )));
    }
    let window: Vec<_> = filtered
        .into_iter()
        .skip(skip.min(state.resume_entries.len()))
        .collect();
    for (i, e) in window.iter().enumerate() {
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
                Span::styled(
                    format!("{:>3} ", e.msg_count),
                    Style::new().fg(theme::MUTED),
                ),
                Span::styled(format!(" {}", e.title), Style::new().fg(theme::TEXT)),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// /model picker — parity with the JS TUI's selector (owner request
/// 2026-11-10). Lists `get_available_models`, marks the active model, and
/// switches on Enter via `set_model`.
fn render_models_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    // Display rows are group headers + model rows. Selection indexes the
    // flattened MODEL list, never the headers.
    let groups = state.filtered_model_groups();
    let mut flat: Vec<&crate::rpc::protocol::Model> = Vec::new();
    let mut rows: Vec<(Option<String>, Option<usize>)> = Vec::new();
    for group in &groups {
        rows.push((
            Some(format!("{} · {}", group.title, group.models.len())),
            None,
        ));
        for model in &group.models {
            flat.push(model);
            rows.push((None, Some(flat.len() - 1)));
        }
    }
    // + borders(2) + search(1) + bottom gap(1)
    let h = area.height.min((rows.len() as u16 + 4).max(9));
    let w = area.width.min(86);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(
                " {} select model · {} of {} ",
                theme::GLYPH_CRUCIBLE,
                flat.len(),
                state.models.len()
            ),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " type to filter · ↑↓ move · enter switch · esc ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 {
        return;
    }

    // Filter line — always visible, doubles as the typed query echo.
    let search = Rect { height: 1, ..inner };
    let filter_spans = if state.models_filter.is_empty() {
        vec![
            Span::styled("⌕ ", Style::new().fg(theme::GOLD).bg(theme::PANEL)),
            Span::styled(
                "type to filter…",
                Style::new()
                    .fg(theme::MUTED)
                    .bg(theme::PANEL)
                    .add_modifier(Modifier::ITALIC),
            ),
            Span::styled("▏", Style::new().fg(theme::GOLD).bg(theme::PANEL)),
        ]
    } else {
        vec![
            Span::styled("⌕ ", Style::new().fg(theme::GOLD).bg(theme::PANEL)),
            Span::styled(
                state.models_filter.clone(),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
            Span::styled("▏", Style::new().fg(theme::GOLD).bg(theme::PANEL)),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(filter_spans)), search);

    let list = Rect {
        y: inner.y + 1,
        height: inner.height.saturating_sub(2),
        ..inner
    };
    let width = list.width as usize;
    let sel = state.models_sel;
    let visible = list.height as usize;
    let sel_row = rows
        .iter()
        .position(|(_, model)| *model == Some(sel))
        .unwrap_or(0);
    let start = if rows.len() <= visible {
        0
    } else {
        let mut start = sel_row.saturating_sub(visible / 2);
        if sel_row >= start + visible {
            start = sel_row + 1 - visible;
        }
        start.min(rows.len() - visible)
    };

    let name_w = (width * 34 / 100).clamp(12, 32);
    let prov_w = (width * 16 / 100).clamp(8, 16);
    let ctx_w = 7usize;
    let fixed = 4 /*markers*/ + name_w + 1 + prov_w + 1 + ctx_w;
    let id_w = width.saturating_sub(fixed + 1);

    let mut lines: Vec<Line> = Vec::new();
    for (header, model_idx) in rows.iter().skip(start).take(visible) {
        if let Some(header) = header {
            let rule = width.saturating_sub(crate::tui::width::cols(header) + 2);
            lines.push(Line::from(vec![
                Span::styled(
                    format!(" {header} "),
                    Style::new()
                        .fg(theme::GOLD)
                        .bg(theme::PANEL)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "─".repeat(rule),
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                ),
            ]));
            continue;
        }
        let Some(index) = model_idx else { continue };
        let m = flat[*index];
        let i = *index;
        let selected = i == sel;
        let current = state
            .model
            .as_ref()
            .is_some_and(|c| c.provider == m.provider && c.id == m.id);
        let name = m.name.clone().unwrap_or_else(|| m.id.clone());
        let ctx = m
            .context_window
            .map(|n| format!("{}k", n / 1024))
            .unwrap_or_else(|| "-".to_string());
        let reasoning = if m.reasoning.unwrap_or(false) {
            "✦"
        } else {
            " "
        };

        let base = if selected {
            Style::new()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        let muted = if selected {
            base
        } else {
            Style::new().fg(theme::MUTED).bg(theme::PANEL)
        };
        let accent = if selected {
            base
        } else {
            Style::new().fg(theme::GOLD).bg(theme::PANEL)
        };
        let live = if selected {
            base
        } else {
            Style::new().fg(theme::GREEN).bg(theme::PANEL)
        };

        let sel_marker = format!("{} ", if selected { "▸" } else { " " });
        let cur_marker = format!("{} ", if current { "●" } else { " " });
        let name_cell = format!("{:<width$}", trunc(&name, name_w), width = name_w);
        let prov_cell = format!("{:<width$}", trunc(&m.provider, prov_w), width = prov_w);
        let ctx_cell = format!("{:>width$}", ctx, width = ctx_w);
        let id_cell = if id_w > 2 {
            format!(" {}", trunc(&m.id, id_w.saturating_sub(1)))
        } else {
            String::new()
        };
        let used = crate::tui::width::cols(&sel_marker)
            + crate::tui::width::cols(&cur_marker)
            + crate::tui::width::cols(&name_cell)
            + 1
            + crate::tui::width::cols(&prov_cell)
            + 1
            + crate::tui::width::cols(&ctx_cell)
            + crate::tui::width::cols(&reasoning)
            + crate::tui::width::cols(&id_cell);

        let mut spans = vec![
            Span::styled(sel_marker, accent),
            Span::styled(cur_marker, live),
            Span::styled(name_cell, base),
            Span::styled(" ", base),
            Span::styled(prov_cell, muted),
            Span::styled(format!(" {}", ctx_cell), accent),
            Span::styled(reasoning, accent),
            Span::styled(id_cell, muted),
        ];
        // Fill to the full inner width so the selected row is a solid bar.
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), base));
        }
        lines.push(Line::from(spans));
    }
    if flat.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching models",
            Style::new()
                .fg(theme::MUTED)
                .bg(theme::PANEL)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        list,
    );
}

/// /tree — the session entry tree, one numbered line per entry so
/// `/fork <n>` can address an entry by its listing position. Scroll-only:
/// selection moves the view, /fork still happens from the composer.
fn render_tree_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area.height.min((state.tree_lines.len() as u16 + 4).max(8));
    let w = area.width.min(92);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(
                " {} session tree · {} entries ",
                theme::GLYPH_QUINTESSENCE,
                state.tree_lines.len()
            ),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ scroll · /fork <n> · esc ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 1 || inner.width < 4 {
        return;
    }

    let visible = inner.height as usize;
    let sel = state.tree_sel;
    let skip = (sel + 1).saturating_sub(visible);
    let width = inner.width as usize;

    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, _id)) in state.tree_lines.iter().enumerate().skip(skip).take(visible) {
        let style = if i == sel {
            Style::new()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        lines.push(Line::from(Span::styled(trunc(label, width), style)));
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "empty session tree",
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

/// /fx (2026-11-13) — fire + glimmer controls with a LIVE preview. The
/// preview obeys the liveliness contract: when the session is stalled the
/// fire preview disappears and says so (no decorative lie).
fn render_fx_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let w = area.width.min(76);
    let h = 16u16.min(area.height);
    let modal = Rect {
        x: area.x + area.width.saturating_sub(w) / 2,
        y: area.y + area.height.saturating_sub(h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let active = state.liveliness_active();
    let chip = if active {
        format!(" ACTIVE · streaming {}s ", state.stalled_secs())
    } else {
        format!(" STALLED · {}s ", state.stalled_secs())
    };
    let chip_style = if active {
        Style::new().fg(theme::GREEN).bg(theme::PANEL)
    } else {
        Style::new().fg(theme::ORANGE).bg(theme::PANEL)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} fx ", theme::GLYPH_SULFUR),
            Style::new()
                .fg(theme::GOLD)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(chip, chip_style))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 8 || inner.width < 40 {
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(5), // rows
        Constraint::Length(2), // preview
        Constraint::Length(2), // status/help
    ])
    .split(inner);

    let rows = [
        ("fire", if state.fire_on { "on" } else { "off" }.to_string()),
        ("voice", state.fire_style.label().to_string()),
        (
            "tint",
            if state.fire_tint { "on" } else { "off" }.to_string(),
        ),
        ("glimmer", state.glimmer.label().to_string()),
    ];
    let mut lines: Vec<Line> = Vec::new();
    for (i, (label, value)) in rows.iter().enumerate() {
        let sel = i == state.fx_sel;
        let marker = if sel { "▸" } else { " " };
        let text = format!(
            " {marker} {label:<10} {value}",
            label = label,
            value = value
        );
        let style = if sel {
            Style::new().fg(theme::BG).bg(theme::GOLD)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        chunks[0],
    );

    // Live preview: animated fire strip (only when armed AND active) plus a
    // shimmer sample for the selected glimmer mode.
    let phase = state.started.elapsed().as_millis();
    let preview = chunks[1];
    if state.fire_on && active && preview.height >= 1 {
        let level = 0.55 + 0.25 * ((phase as f64 / 900.0).sin().abs());
        crate::fire::render(
            frame,
            Rect {
                x: preview.x + 1,
                y: preview.y,
                width: preview.width.saturating_sub(2),
                height: preview.height.min(2),
            },
            phase,
            level,
            state.fire_style,
            state.fire_taper,
        );
    } else {
        let msg = if state.fire_on {
            format!(
                " fire preview hidden — stalled {}s (no stream, no tool CPU)",
                state.stalled_secs()
            )
        } else {
            " fire off".to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ))),
            Rect {
                height: 1,
                ..preview
            },
        );
    }
    // Glimmer sample: phase-driven shimmer glyphs (decorative by design).
    if state.glimmer != crate::watermark::GlimmerMode::Off {
        let t = (phase as f64 / 500.0).sin() * 0.5 + 0.5;
        let pulse = Style::new()
            .fg(theme::GOLD)
            .bg(theme::PANEL)
            .add_modifier(if t > 0.6 {
                Modifier::BOLD
            } else {
                Modifier::DIM
            });
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" ⌁ ", Style::new().fg(theme::MUTED).bg(theme::PANEL)),
                Span::styled(format!("{} ⣀⣤⣶⣿⣶⣤⣀", state.glimmer.label()), pulse),
            ])),
            Rect {
                height: 1,
                y: preview.y + preview.height.saturating_sub(1),
                ..preview
            },
        );
    }

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                " ↑↓ row · space/←→ cycle · esc close",
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            )),
            Line::from(Span::styled(
                " fire follows the liveliness contract: no evidence → no flames",
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            )),
        ])
        .style(Style::new().bg(theme::PANEL)),
        chunks[2],
    );
}

/// /answers (2026-11-13) — the last INQUIRE-mode question + answer, wrapped
/// and scrollable. Read-only facts-file view.
fn render_answers_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area.height.min(area.height.saturating_sub(2)).max(8);
    let w = area.width.min(96);
    let modal = Rect {
        y: area.y,
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} inquire · answer ", theme::GLYPH_HERM),
            Style::new()
                .fg(theme::GOLD)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " ↑↓ scroll · r reload · esc close ",
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        ))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 20 {
        return;
    }
    let Some(doc) = state.answers.as_ref() else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "  no inquiry answer yet — switch to INQUIRE (TAB) and ask a question",
                    Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    "  the answer lands here without interrupting the build flow",
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                )),
            ]),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  Q  ", Style::new().fg(theme::GOLD).bg(theme::PANEL)),
        Span::styled(
            trunc(&doc.question, inner.width.saturating_sub(6) as usize),
            Style::new().fg(theme::TEXT).bg(theme::PANEL),
        ),
    ]));
    lines.push(Line::from(""));
    let body_w = inner.width.saturating_sub(6) as usize;
    for para_line in doc.answer.lines() {
        for seg in crate::tui::state::wrap_text(para_line, body_w) {
            lines.push(Line::from(vec![
                Span::styled("  ", Style::new().bg(theme::PANEL)),
                Span::styled(seg, Style::new().fg(theme::TEXT).bg(theme::PANEL)),
            ]));
        }
    }
    let visible = inner.height as usize;
    let start = (state.answers_scroll as usize).min(lines.len().saturating_sub(1));
    let window: Vec<Line> = lines.into_iter().skip(start).take(visible).collect();
    frame.render_widget(
        Paragraph::new(window).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

/// /cost (exec-infra stage 5) — today's route ledger + budget bars.
/// Euro appears only when the ledger itself carried costEstimate (I5).
fn render_routes_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area.height.min(30);
    let w = area.width.min(116);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let day = state
        .routes
        .as_ref()
        .map(|r| r.day.clone())
        .unwrap_or_else(|| "—".to_string());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(
            format!(" 🜍 caduceus decisions · {day} (newest first) "),
            Style::new()
                .fg(theme::GOLD)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.width < 60 || inner.height < 4 {
        return;
    }
    let now_ms = crate::tui::caduceus::now_ms();
    let Some(report) = state.routes.as_ref() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no route ledger for today",
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ))),
            inner,
        );
        return;
    };
    if report.entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no decisions recorded today",
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ))),
            inner,
        );
        return;
    }
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<9}{:<7}{:<6}{:<40}{:<10}{:<7}{:>7} {:>9}  {:<12}",
            "time", "tier", "state", "model", "reason", "local", "latency", "tokens", "probe"
        ),
        Style::new().fg(theme::MUTED).bg(theme::PANEL),
    )));
    let visible = (inner.height as usize).saturating_sub(2);
    let total = report.entries.len();
    let max_sel = total.saturating_sub(visible);
    if state.routes_sel > max_sel {
        state.routes_sel = max_sel;
    }
    let sel = state.routes_sel;
    for (i, e) in report.entries.iter().enumerate().skip(sel).take(visible) {
        let latest = i == 0;
        let marker = if latest { "▸" } else { " " };
        let state_style = if e.outcome == "ok" {
            Style::new().fg(theme::GREEN).bg(theme::PANEL)
        } else {
            Style::new().fg(theme::ORANGE).bg(theme::PANEL)
        };
        let tier_style = if e.tier == "local" {
            Style::new().fg(theme::GREEN).bg(theme::PANEL)
        } else {
            Style::new().fg(theme::GOLD).bg(theme::PANEL)
        };
        let model = format!("{}/{}", e.provider, e.model);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} {:<9}", marker, crate::tui::routes::short_time(&e.ts)),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ),
            Span::styled(format!("{:<7}", e.tier), tier_style),
            Span::styled(format!("{:<6}", e.outcome), state_style),
            Span::styled(
                format!("{:<40}", trunc(&model, 40)),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
            Span::styled(
                format!("{:<10}", trunc(&e.reason, 10)),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ),
            Span::styled(
                format!("{:<7}", trunc(&e.local_state, 7)),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ),
            Span::styled(
                format!("{:>6.1}s", e.latency_ms as f64 / 1000.0),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
            Span::styled(
                format!("  {:>4}→{:<4}", e.tokens_in, e.tokens_out),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ),
            Span::styled(
                format!(
                    "  {:<12}",
                    trunc(
                        &crate::tui::routes::probe_label(report.probes.get(&e.model), now_ms),
                        12
                    )
                ),
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!("  {} decisions · ↑↓ scroll · esc closes", total),
        Style::new().fg(theme::MUTED).bg(theme::PANEL),
    )));
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

fn render_cost_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = area.height.min(30);
    let w = area.width.min(96);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let day = state
        .cost
        .as_ref()
        .map(|r| r.day.clone())
        .unwrap_or_else(|| "—".to_string());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} cost · route ledger {day} ", theme::GLYPH_HERM),
            Style::new()
                .fg(theme::GOLD)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.width < 50 || inner.height < 4 {
        return;
    }
    let Some(report) = state.cost.as_ref() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  no ledger data",
                Style::new().fg(theme::MUTED).bg(theme::PANEL),
            ))),
            inner,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    if report.budget_present {
        let bar = |frac: f32| {
            let filled = (frac * 20.0).round() as usize;
            let mut s = String::from("[");
            for i in 0..20 {
                s.push(if i < filled { '#' } else { '.' });
            }
            s.push(']');
            s
        };
        let style = |frac: f32| {
            if frac >= 1.0 {
                Style::new().fg(theme::RED).bg(theme::PANEL)
            } else if frac >= 0.8 {
                Style::new().fg(theme::ORANGE).bg(theme::PANEL)
            } else {
                Style::new().fg(theme::GREEN).bg(theme::PANEL)
            }
        };
        let df = report.daily_fraction();
        let mf = report.monthly_fraction();
        lines.push(Line::from(vec![
            Span::styled("  day    ", Style::new().fg(theme::MUTED).bg(theme::PANEL)),
            Span::styled(bar(df), style(df)),
            Span::styled(
                format!(
                    " {} / {} ({})",
                    crate::tui::caduceus::fmt_count(report.budget_daily_tokens),
                    crate::tui::caduceus::fmt_count(report.budget_daily_cap),
                    if report.budget_allowed {
                        "open"
                    } else {
                        "EXHAUSTED"
                    }
                ),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  month  ", Style::new().fg(theme::MUTED).bg(theme::PANEL)),
            Span::styled(bar(mf), style(mf)),
            Span::styled(
                format!(
                    " {} / {}",
                    crate::tui::caduceus::fmt_count(report.budget_monthly_tokens),
                    crate::tui::caduceus::fmt_count(report.budget_monthly_cap)
                ),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
    }
    lines.push(Line::from(Span::styled(
        format!(
            "  {:<44} {:>8} {:>6} {:>10} {:>10}",
            "provider/model", "reqs", "fails", "tok in", "tok out"
        ),
        Style::new().fg(theme::MUTED).bg(theme::PANEL),
    )));
    let budget_h: usize = if report.budget_present { 3 } else { 1 };
    let visible = (inner.height - (budget_h as u16 + 1)) as usize;
    for row in report.rows.iter().take(visible) {
        let fail_style = if row.failures > 0 {
            Style::new().fg(theme::ORANGE).bg(theme::PANEL)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {:<44} {:>8}", trunc(&row.key, 44), row.requests),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
            Span::styled(format!(" {:>6}", row.failures), fail_style),
            Span::styled(
                format!(
                    " {:>10} {:>10}",
                    crate::tui::caduceus::fmt_count(row.tokens_in),
                    crate::tui::caduceus::fmt_count(row.tokens_out)
                ),
                Style::new().fg(theme::TEXT).bg(theme::PANEL),
            ),
        ]));
    }
    if report.rows.len() > visible {
        lines.push(Line::from(Span::styled(
            format!("  … {} more models", report.rows.len() - visible),
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        )));
    }
    lines.push(Line::from(vec![
        Span::styled(
            format!(
                "  total: {} in · {} out",
                crate::tui::caduceus::fmt_count(report.total_in),
                crate::tui::caduceus::fmt_count(report.total_out)
            ),
            Style::new().fg(theme::GOLD).bg(theme::PANEL),
        ),
        Span::styled(
            if report.total_cost > 0.0 {
                format!(" · €{:.4}", report.total_cost)
            } else {
                String::new()
            },
            Style::new().fg(theme::GOLD).bg(theme::PANEL),
        ),
    ]));
    lines.truncate(inner.height as usize);
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

/// /sessions (exec-infra stage 4) — every project store, newest first.
/// Foreign projects render READ-ONLY: enter prints the launch hint (I4).
fn render_xsessions_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let view = state.xsessions_view();
    let h = area.height.min((view.len() as u16 + 3).clamp(8, 26));
    let w = area.width.min(104);
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
            " sessions · all projects ",
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            if state.xsessions_filter.is_empty() {
                " ↑↓ select · enter resume/local · type to filter · esc cancel ".to_string()
            } else {
                format!(" /{} _ · ↑↓ · enter · esc clears ", state.xsessions_filter)
            },
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let visible = inner.height as usize;
    let sel = state.xsessions_sel;
    let skip = (sel + 1).saturating_sub(visible);
    let mut lines: Vec<Line> = Vec::new();
    if view.is_empty() {
        lines.push(Line::from(Span::styled(
            if state.xsessions_filter.is_empty() {
                "  no sessions in any store".to_string()
            } else {
                format!("  nothing matches '{}'", state.xsessions_filter)
            },
            Style::new().fg(theme::MUTED),
        )));
    }
    let cwd_str = state.cwd.to_string_lossy().to_string();
    let window: Vec<_> = view.into_iter().skip(skip).collect();
    for (i, a) in window.iter().enumerate() {
        let selected = skip + i == sel;
        let marker = if selected { "▸" } else { " " };
        let project = if a.project == cwd_str {
            "this project".to_string()
        } else {
            shorten_home(std::path::Path::new(&a.project))
        };
        let age = caduceus_age(
            std::time::SystemTime::now()
                .duration_since(a.entry.modified)
                .unwrap_or_default(),
        );
        let foreign = if a.project == cwd_str { "" } else { " · ro" };
        let text = format!(
            " {} {:<28} {:<44} {:>5}msg {:>7}{foreign}",
            marker,
            trunc(&project, 28),
            trunc(&a.entry.title, 44),
            a.entry.msg_count,
            age
        );
        let style = if selected {
            Style::new().fg(theme::BG).bg(theme::GOLD)
        } else if a.project == cwd_str {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        } else {
            Style::new().fg(theme::MUTED).bg(theme::PANEL)
        };
        lines.push(Line::from(Span::styled(text, style)));
    }
    let body: Vec<Line> = lines.into_iter().take(visible).collect();
    frame.render_widget(
        Paragraph::new(body).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

fn caduceus_age(d: std::time::Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s}s")
    } else if s < 3_600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3_600)
    } else {
        format!("{}d", s / 86_400)
    }
}

/// /review-findings (codex-qol stage 7) — severity-colored findings from
/// .officina/review.json. Read-only; honest hint when nothing was reviewed.
fn render_review_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    use crate::tui::review::Finding;
    let rows = state.review_rows();
    let detail_h = if rows.is_empty() { 0 } else { 5 };
    let h = area
        .height
        .min((rows.len() as u16 + 7 + detail_h).clamp(8, 30));
    let w = area.width.min(92);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let (title_st, title) = match state.review.as_ref() {
        Some(f) => (
            Style::new().fg(theme::GOLD).bg(theme::PANEL),
            format!(
                " {} review findings · {} · {} high / {} med / {} low ",
                theme::GLYPH_HERM,
                if f.preset.is_empty() { "?" } else { &f.preset },
                f.count_by("high"),
                f.count_by("medium"),
                f.count_by("low")
            ),
        ),
        None => (
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
            " review findings ".to_string(),
        ),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(title, title_st.add_modifier(Modifier::BOLD)))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 40 {
        return;
    }
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(detail_h),
        Constraint::Length(1),
    ])
    .split(inner);

    let Some(f) = state.review.as_ref() else {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "  no review findings yet — run /review (uncommitted | commit | base | custom)",
                    Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    "  findings land in .officina/review.json after the review turn",
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                )),
            ]),
            chunks[1],
        );
        return;
    };

    // summary line
    let summary = if f.summary.is_empty() {
        "(no summary)".to_string()
    } else {
        f.summary.clone()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(
                "  {}",
                trunc(&summary, inner.width.saturating_sub(4) as usize)
            ),
            Style::new().fg(theme::TEXT).bg(theme::PANEL),
        ))),
        chunks[0],
    );

    let visible = chunks[1].height as usize;
    let start = if state.review_sel < visible {
        0
    } else {
        (state.review_sel + 1).saturating_sub(visible)
    };
    let mut lines: Vec<Line> = Vec::new();
    let sev_style = |sev: &str| match sev {
        "high" => Style::new().fg(theme::RED).bg(theme::PANEL),
        "medium" => Style::new().fg(theme::ORANGE).bg(theme::PANEL),
        "low" => Style::new().fg(theme::GOLD).bg(theme::PANEL),
        _ => Style::new().fg(theme::MUTED).bg(theme::PANEL),
    };
    for (idx, finding) in rows.iter().enumerate() {
        let selected = idx == state.review_sel;
        let marker = if selected { "▸" } else { " " };
        let badge = format!("{:<6}", finding.severity);
        let loc = trunc(&finding.locator(), 34);
        let text = format!(
            " {} {:<6} {:<34} {}",
            marker,
            badge,
            loc,
            trunc(&finding.title, inner.width.saturating_sub(48) as usize)
        );
        let style = if selected {
            Style::new().fg(theme::BG).bg(theme::GOLD)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        lines.push(Line::from(vec![
            Span::styled(text, style),
            Span::styled("", sev_style(&finding.severity)),
        ]));
    }
    let body: Vec<Line> = lines.into_iter().skip(start).take(visible).collect();
    frame.render_widget(
        Paragraph::new(body).style(Style::new().bg(theme::PANEL)),
        chunks[1],
    );

    // detail pane
    if detail_h > 0 {
        if let Some(finding) = rows.get(state.review_sel) {
            let d: &Finding = finding;
            let detail_lines = vec![
                Line::from(Span::styled(
                    format!(
                        "  {} · {}",
                        d.title,
                        trunc(&d.locator(), inner.width.saturating_sub(6) as usize)
                    ),
                    Style::new().fg(theme::GOLD).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    format!(
                        "  {}",
                        trunc(&d.detail, inner.width.saturating_sub(4) as usize)
                    ),
                    Style::new().fg(theme::TEXT).bg(theme::PANEL),
                )),
                Line::from(Span::styled(
                    "  esc close · ↑↓ navigate",
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                )),
            ];
            frame.render_widget(
                Paragraph::new(detail_lines).style(Style::new().bg(theme::PANEL)),
                chunks[2],
            );
        }
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " ↑↓ select · esc close",
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        ))),
        chunks[3],
    );
}

/// /status (codex-qol stage 2) — read-only session snapshot, help-modal
/// geometry. Rows were snapshotted at open (no per-frame file reads).
fn render_status_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let h = (state.status_lines.len() as u16 + 4)
        .min(area.height.saturating_sub(2))
        .max(6);
    let w = area.width.min(80);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .title(Span::styled(
            format!(" {} status ", theme::GLYPH_HERM),
            Style::new()
                .fg(theme::GOLD)
                .bg(theme::PANEL)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    let lines: Vec<Line> = state
        .status_lines
        .iter()
        .take(inner.height as usize)
        .map(|(label, value)| {
            Line::from(vec![
                Span::styled(
                    format!("  {label:<12}"),
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                ),
                Span::styled(value.clone(), Style::new().fg(theme::TEXT).bg(theme::PANEL)),
            ])
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        inner,
    );
}

/// /caduceus page (2026-11-12) — full-bleed, four tabs. Facts come from the
/// extension's caduceus-state.json; policy from the TUI-owned
/// caduceus-prefs.json. I4: no HTTP from the TUI, no secrets in it either.
fn render_caduceus_page(frame: &mut Frame, state: &mut AppState, area: Rect) {
    use crate::tui::caduceus::{self, ModelState, PolicyRow, Tab};

    let Some(page) = state.caduceus.as_ref() else {
        return;
    };
    let now = caduceus::now_ms();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD).bg(theme::PANEL))
        .style(Style::new().bg(theme::PANEL));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 6 || inner.width < 40 {
        return;
    }

    let detail_h = if page.detail {
        match page.tab {
            Tab::Providers => 6,
            Tab::Models => 4,
            _ => 0,
        }
    } else {
        0
    };
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(detail_h),
        Constraint::Length(2),
    ])
    .split(inner);

    // ── tab bar ──
    let mut spans: Vec<Span> = vec![Span::styled(
        format!(" {} caduceus  ", theme::GLYPH_HERM),
        Style::new()
            .fg(theme::GOLD)
            .bg(theme::PANEL)
            .add_modifier(Modifier::BOLD),
    )];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let style = if *tab == page.tab {
            Style::new()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme::MUTED).bg(theme::PANEL)
        };
        spans.push(Span::styled(format!(" {} {} ", i + 1, tab.label()), style));
        spans.push(Span::raw(" "));
    }
    let stamp = match page.state.as_ref() {
        Some(s) => {
            let stale = if s.is_stale(now) { " · STALE" } else { "" };
            format!(
                "{} free lanes · state {}{stale}",
                s.free_candidate_count(),
                caduceus::fmt_age_ms(s.age_ms(now))
            )
        }
        None => "no state file yet".to_string(),
    };
    spans.push(Span::styled(
        format!("  {stamp}"),
        Style::new().fg(theme::MUTED).bg(theme::PANEL),
    ));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::new().bg(theme::PANEL)),
        chunks[0],
    );

    // ── header row ──
    let header = match page.tab {
        Tab::Providers => format!(
            "{:<22} {:<4} {:<5} {:<4} {:<8} {:>5} {:<8}",
            "provider", "use", "rank", "key", "gateway", "free", "method"
        ),
        Tab::Models => format!(
            "{:<44} {:<14} {:>8} {:<5} {:<7}",
            "model", "owner", "ctx", "free", "state"
        ),
        Tab::Health => "gateway · catalog · budget · connections".to_string(),
        Tab::Policy => "policy — space toggles · +/- adjusts".to_string(),
        Tab::Vitriol => "engine · vitriol-server.service".to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("  {header}"),
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        ))),
        chunks[1],
    );

    // ── body ──
    let mut lines: Vec<Line> = Vec::new();
    match page.tab {
        Tab::Providers => {
            for (idx, p) in page.provider_view().iter().enumerate() {
                let selected = idx == page.sel;
                let use_ = if page.prefs.routing(&p.id) == "off" {
                    "off"
                } else {
                    "on"
                };
                let rank = page
                    .prefs
                    .rank(&p.id)
                    .map(|r| r.to_string())
                    .unwrap_or_else(|| "—".to_string());
                let key = if p.key_present { "✓" } else { "—" };
                let method = if p.method.is_empty() {
                    p.category.as_str()
                } else {
                    p.method.as_str()
                };
                let text = format!(
                    "  {:<22} {:<4} {:<5} {:<4} {:<8} {:>5} {:<8}",
                    trunc(&p.id, 22),
                    use_,
                    rank,
                    key,
                    p.gateway,
                    p.free_models,
                    method
                );
                lines.push(Line::from(Span::styled(text, caduceus_row_style(selected))));
            }
        }
        Tab::Models => {
            for (idx, c) in page.candidate_view().iter().enumerate() {
                let selected = idx == page.sel;
                let state_label = if page.show_hidden && page.prefs.is_hidden(&c.id) {
                    "hidden"
                } else {
                    page.prefs.model_state(&c.id).label()
                };
                // `f` marks the reroute fallback (2026-11-16): shown so the
                // operator can see which model catches a spent lane.
                let state_label = if page.is_fallback(&c.id) {
                    format!("{state_label} ⤵").trim().to_string()
                } else {
                    state_label.to_string()
                };
                let text = format!(
                    "  {:<44} {:<14} {:>8} {:<5} {:<7}",
                    trunc(&c.id, 44),
                    trunc(&c.owner, 14),
                    caduceus::fmt_count(c.context_window),
                    if c.free { "✓" } else { "paid" },
                    state_label
                );
                let style = if selected {
                    caduceus_row_style(true)
                } else if page.prefs.is_hidden(&c.id) {
                    Style::new().fg(theme::MUTED).bg(theme::PANEL)
                } else {
                    caduceus_row_style(false)
                };
                lines.push(Line::from(Span::styled(text, style)));
            }
        }
        Tab::Health => match page.state.as_ref() {
            Some(s) => {
                let b = &s.budget;
                let pct = |used: u64, cap: u64| {
                    if cap == 0 {
                        "—".to_string()
                    } else {
                        format!("{:.0}%", used as f64 * 100.0 / cap as f64)
                    }
                };
                let rows = [
                    format!("gateway      {}", if s.gateway_up { "Up" } else { "DOWN" }),
                    format!("base url     {}", trunc(&s.base_url, 64)),
                    format!(
                        "catalog      {} models · fetched {}",
                        s.catalog_count,
                        if s.catalog_fetched_at.is_empty() {
                            "—".to_string()
                        } else {
                            trunc(&s.catalog_fetched_at, 24)
                        }
                    ),
                    format!(
                        "budget       day {} / {} ({}) · month {} / {} ({})",
                        caduceus::fmt_count(b.daily_tokens),
                        caduceus::fmt_count(b.daily_cap),
                        pct(b.daily_tokens, b.daily_cap),
                        caduceus::fmt_count(b.monthly_tokens),
                        caduceus::fmt_count(b.monthly_cap),
                        pct(b.monthly_tokens, b.monthly_cap)
                    ),
                    format!(
                        "cap state    {} · paid {}",
                        if b.allowed {
                            "within cap"
                        } else {
                            "CAP REACHED"
                        },
                        if b.paid_allowed { "allowed" } else { "never" }
                    ),
                    format!("cooling      {} models", s.cooling),
                    format!(
                        "connections  {} active · {} backoff · {} hard-down",
                        s.connections.active.len(),
                        s.connections.backoff.len(),
                        s.connections.hard_down.len()
                    ),
                    format!(
                        "active       {}",
                        trunc(&s.connections.active.join(", "), 92)
                    ),
                    format!(
                        "backoff      {}",
                        trunc(&s.connections.backoff.join(", "), 92)
                    ),
                    format!(
                        "hard-down    {}",
                        trunc(&s.connections.hard_down.join(", "), 92)
                    ),
                    format!(
                        "keys         {} readable rows · inference {}",
                        s.keys.len(),
                        if s.inference_key.is_empty() {
                            "MISSING"
                        } else {
                            "set"
                        }
                    ),
                ];
                for row in rows {
                    lines.push(Line::from(Span::styled(
                        format!("  {row}"),
                        Style::new().fg(theme::TEXT).bg(theme::PANEL),
                    )));
                }
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "  no ~/.vitriol/officina/caduceus-state.json yet — the caduceus extension writes it at session start + every 60s",
                    Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                )));
                lines.push(Line::from(Span::styled(
                    "  run `vitriol caduceus setup` if the gateway is not installed",
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                )));
            }
        },
        Tab::Vitriol => match page.engine.as_ref() {
            Some(e) => {
                let state_st = match e.state.as_str() {
                    "active" => Style::new().fg(theme::GREEN).bg(theme::PANEL),
                    "inactive" => Style::new().fg(theme::MUTED).bg(theme::PANEL),
                    _ => Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                };
                lines.push(Line::from(vec![
                    Span::styled(
                        "  unit       ",
                        Style::new().fg(theme::MUTED).bg(theme::PANEL),
                    ),
                    Span::styled(
                        "vitriol-server.service",
                        Style::new().fg(theme::TEXT).bg(theme::PANEL),
                    ),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(
                        "  state      ",
                        Style::new().fg(theme::MUTED).bg(theme::PANEL),
                    ),
                    Span::styled(
                        format!(
                            "{}{}{}",
                            e.state,
                            if e.pid.is_empty() {
                                String::new()
                            } else {
                                format!(" (pid {})", e.pid)
                            },
                            if e.since.is_empty() {
                                String::new()
                            } else {
                                format!(" · since {}", e.since)
                            }
                        ),
                        state_st,
                    ),
                ]));
                lines.push(Line::from(Span::styled(
                    format!(
                        "  last log   {}",
                        trunc(&e.last_log, inner.width.saturating_sub(14) as usize)
                    ),
                    Style::new().fg(theme::TEXT).bg(theme::PANEL),
                )));
                lines.push(Line::from(Span::styled(
                        "  controls   S start · X stop · R restart (press twice to confirm) · r re-sample",
                        Style::new().fg(theme::GOLD).bg(theme::PANEL),
                    )));
                lines.push(Line::from(Span::styled(
                        "  config     your live daily driver (unit as-is; `vitriol config load <profile>` outside the TUI)",
                        Style::new().fg(theme::MUTED).bg(theme::PANEL),
                    )));
                lines.push(Line::from(Span::styled(
                    "  ── config view (read-only · j/k scroll · docs/engine-settings.md) ──",
                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                )));
                match page.config.as_ref() {
                    Some(rows) => {
                        let start = page.config_scroll.min(rows.len());
                        for r in rows.iter().skip(start) {
                            let val_st = if r.risk {
                                Style::new().fg(theme::ORANGE).bg(theme::PANEL)
                            } else {
                                Style::new().fg(theme::TEXT).bg(theme::PANEL)
                            };
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!(
                                        "  {:<24} {:<14}",
                                        trunc(&r.label(), 24),
                                        trunc(&r.value, 14)
                                    ),
                                    val_st,
                                ),
                                Span::styled(
                                    trunc(r.explain, inner.width.saturating_sub(44) as usize),
                                    Style::new().fg(theme::MUTED).bg(theme::PANEL),
                                ),
                            ]));
                        }
                    }
                    None => lines.push(Line::from(Span::styled(
                        "  config not readable (~/.vitriol/config)",
                        Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                    ))),
                }
            }
            None => {
                lines.push(Line::from(Span::styled(
                    "  engine facts not sampled yet — press r",
                    Style::new().fg(theme::ORANGE).bg(theme::PANEL),
                )));
            }
        },
        Tab::Policy => {
            for (idx, row) in page.policy_rows().iter().enumerate() {
                let selected = idx == page.sel;
                let text = match row {
                    PolicyRow::FreeOnly => format!(
                        "  {:<24} {}",
                        "free only",
                        if page.prefs.free_only { "on" } else { "off" }
                    ),
                    PolicyRow::PerOwner => {
                        format!("  {:<24} {} (1–32)", "per-owner cap", page.prefs.per_owner)
                    }
                    PolicyRow::MaxCandidates => format!(
                        "  {:<24} {} (1–256)",
                        "max candidates", page.prefs.max_candidates
                    ),
                    PolicyRow::AllowPaid { provider, allow } => format!(
                        "  {:<24} {}",
                        format!("allow paid · {provider}"),
                        if *allow { "yes" } else { "no" }
                    ),
                };
                lines.push(Line::from(Span::styled(text, caduceus_row_style(selected))));
            }
        }
    }
    let visible = chunks[2].height as usize;
    let start = if visible == 0 || page.sel < visible {
        0
    } else {
        (page.sel + 1).saturating_sub(visible)
    };
    let body_lines: Vec<Line> = lines.into_iter().skip(start).take(visible).collect();
    frame.render_widget(
        Paragraph::new(body_lines).style(Style::new().bg(theme::PANEL)),
        chunks[2],
    );

    // ── detail pane ──
    if page.detail && detail_h > 0 {
        let detail = match page.tab {
            Tab::Providers => page.provider_view().get(page.sel).map(|p| {
                vec![
                    format!("{} — {}", p.id, p.name),
                    format!(
                        "website   {}",
                        if p.website.is_empty() {
                            "—"
                        } else {
                            p.website.as_str()
                        }
                    ),
                    format!(
                        "env       {} · key {}",
                        if p.credential_env.is_empty() {
                            "—"
                        } else {
                            p.credential_env.as_str()
                        },
                        if p.key_present { "present" } else { "missing" }
                    ),
                    format!(
                        "free      {} · {} free models",
                        if p.has_free { "yes" } else { "no" },
                        p.free_models
                    ),
                    format!(
                        "signup    {} · status {}",
                        if p.method.is_empty() {
                            "—"
                        } else {
                            p.method.as_str()
                        },
                        if p.signup_status.is_empty() {
                            "pending"
                        } else {
                            p.signup_status.as_str()
                        }
                    ),
                    format!(
                        "notes     {}{}",
                        trunc(&p.free_note, 48),
                        if p.auth_hint.is_empty() {
                            String::new()
                        } else {
                            format!(" · {}", trunc(&p.auth_hint, 40))
                        }
                    ),
                ]
            }),
            Tab::Models => page.candidate_view().get(page.sel).map(|c| {
                vec![
                    c.id.clone(),
                    format!(
                        "owner     {} · ctx {} · max {} · {}",
                        c.owner,
                        caduceus::fmt_count(c.context_window),
                        caduceus::fmt_count(c.max_tokens),
                        if c.free { "free" } else { "paid" }
                    ),
                    format!(
                        "state     {}{}",
                        match page.prefs.model_state(&c.id) {
                            ModelState::Neutral => "neutral (space: pin → deny → neutral)",
                            other => other.label(),
                        },
                        if page.prefs.is_hidden(&c.id) {
                            " · hidden (h unhides, H reveals)"
                        } else {
                            " · visible (h hides)"
                        }
                    ),
                ]
            }),
            _ => None,
        };
        if let Some(rows) = detail {
            let detail_lines: Vec<Line> = rows
                .into_iter()
                .take(chunks[3].height as usize)
                .map(|r| {
                    Line::from(Span::styled(
                        format!("  {r}"),
                        Style::new().fg(theme::TEXT).bg(theme::PANEL),
                    ))
                })
                .collect();
            frame.render_widget(
                Paragraph::new(detail_lines).style(Style::new().bg(theme::PANEL)),
                chunks[3],
            );
        }
    }

    // ── footer ──
    let mut footer = vec![Line::from(Span::styled(
        "  tab switch · space toggle · +/- rank/cap · h hide · H show hidden · a add key · A wizard · o oauth/site · / filter · r reload · esc close",
        Style::new().fg(theme::MUTED).bg(theme::PANEL),
    ))];
    if page.filtering {
        footer.push(Line::from(Span::styled(
            format!("  /{}_", page.filter),
            Style::new().fg(theme::GOLD).bg(theme::PANEL),
        )));
    } else if let Some(arm) = page.engine_arm.as_ref() {
        footer.push(Line::from(Span::styled(
            format!(
                "  ARMED: press {} again to confirm engine {} (any other key cancels)",
                arm.chars().next().unwrap_or('?'),
                arm
            ),
            Style::new().fg(theme::ORANGE).bg(theme::PANEL),
        )));
    } else if let Some(note) = page.note.as_ref() {
        let style = if note.contains("ignored") || note.contains("failed") {
            theme::ORANGE
        } else {
            theme::GREEN
        };
        footer.push(Line::from(Span::styled(
            format!("  {note} · {}", page.policy_summary()),
            Style::new().fg(style).bg(theme::PANEL),
        )));
    } else {
        footer.push(Line::from(Span::styled(
            format!("  {}", page.policy_summary()),
            Style::new().fg(theme::MUTED).bg(theme::PANEL),
        )));
    }
    footer.truncate(chunks[4].height as usize);
    frame.render_widget(
        Paragraph::new(footer).style(Style::new().bg(theme::PANEL)),
        chunks[4],
    );
}

fn caduceus_row_style(selected: bool) -> Style {
    if selected {
        Style::new().fg(theme::BG).bg(theme::GOLD)
    } else {
        Style::new().fg(theme::TEXT).bg(theme::PANEL)
    }
}

fn render_cache_modal(frame: &mut Frame, state: &mut AppState, area: Rect) {
    use crate::tui::cache as c;
    let h = area
        .height
        .min((state.cache_entries.len() as u16 + 5).max(10));
    let w = area.width.min(92);
    let modal = Rect {
        y: area.y + area.height.saturating_sub(h + 1),
        x: area.x,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::GOLD))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(
            format!(
                " {} kv cache · {} entries · {} ",
                theme::GLYPH_CRUCIBLE,
                state.cache_entries.len(),
                c::budget_line(state.cache_total)
            ),
            Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(Span::styled(
            " d delete · p prune 7d · W wipe all · r rescan · esc ",
            Style::new().fg(theme::MUTED),
        ));
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 3 {
        return;
    }

    let dir = Rect { height: 1, ..inner };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            trunc(&state.cache_dir, inner.width as usize),
            Style::new()
                .fg(theme::MUTED)
                .bg(theme::PANEL)
                .add_modifier(Modifier::ITALIC),
        ))),
        dir,
    );

    let list = Rect {
        y: inner.y + 2,
        height: inner.height.saturating_sub(3),
        ..inner
    };
    let width = list.width as usize;
    let visible = list.height as usize;
    let sel = state.cache_sel;
    let skip = (sel + 1).saturating_sub(visible);
    let age_w = 5usize;
    let size_w = 10usize;
    let name_w = width.saturating_sub(2 + age_w + 1 + size_w);

    let mut lines: Vec<Line> = Vec::new();
    for (i, e) in state.cache_entries.iter().enumerate().skip(skip) {
        let selected = i == sel;
        let protected = c::is_protected(&e.name);
        let base = if selected {
            Style::new()
                .fg(theme::BG)
                .bg(theme::GOLD)
                .add_modifier(Modifier::BOLD)
        } else if protected {
            Style::new().fg(theme::GREEN).bg(theme::PANEL)
        } else {
            Style::new().fg(theme::TEXT).bg(theme::PANEL)
        };
        let muted = if selected {
            base
        } else {
            Style::new().fg(theme::MUTED).bg(theme::PANEL)
        };
        let accent = if selected {
            base
        } else {
            Style::new().fg(theme::GOLD).bg(theme::PANEL)
        };

        let marker = format!("{} ", if selected { "▸" } else { " " });
        let label = if protected {
            format!("{} (protected)", e.name)
        } else {
            e.name.clone()
        };
        let name_cell = format!("{:<width$}", trunc(&label, name_w), width = name_w);
        let age_cell = format!("{:>width$}", c::fmt_age(e.age()), width = age_w);
        let size_cell = format!("{:>width$}", c::fmt_bytes(e.size), width = size_w);
        let used = crate::tui::width::cols(&marker)
            + crate::tui::width::cols(&name_cell)
            + 1
            + crate::tui::width::cols(&age_cell)
            + 1
            + crate::tui::width::cols(&size_cell);
        let mut spans = vec![
            Span::styled(marker, accent),
            Span::styled(name_cell, base),
            Span::styled(" ", base),
            Span::styled(age_cell, muted),
            Span::styled(" ", base),
            Span::styled(size_cell, accent),
        ];
        if used < width {
            spans.push(Span::styled(" ".repeat(width - used), base));
        }
        lines.push(Line::from(spans));
    }
    if state.cache_entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  cache is empty",
            Style::new()
                .fg(theme::MUTED)
                .bg(theme::PANEL)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::new().bg(theme::PANEL)),
        list,
    );
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
    let skip = (sel + 1).saturating_sub(visible);

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
            (crate::tui::state::tool_display_name(name), status, st)
        };
        if selected {
            lines.push(Line::from(Span::styled(
                trunc(
                    &format!("▸ {:<16} {}", label, status),
                    (inner.width as usize).saturating_sub(1),
                ),
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
        if crate::tui::width::cols(&candidate) > width {
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
    use crate::tui::state::{help_rows, HelpRowKind};
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
    let skip = (sel + 1).saturating_sub(visible);
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

/// Elide one rendered line to exactly `width` columns, keeping its styles.
///
/// WRAP-1 (2026-11-17): the transcript is painted WITHOUT `.wrap()`, so a line
/// wider than the pane was silently cut at the edge with no marker — long
/// tokens, URLs and wide-char runs vanished mid-word. Eliding here keeps the
/// "one logical line = one visual row" invariant that the scroll maths below
/// depends on (SCROLL-1): do NOT enable `.wrap()` on this paragraph without
/// switching `max_scroll`/`start` to visual rows.
fn elide_line(line: ratatui::text::Line<'static>, width: usize) -> ratatui::text::Line<'static> {
    use ratatui::text::Span;
    if width == 0 {
        return ratatui::text::Line::from("");
    }
    let used: usize = line
        .spans
        .iter()
        .map(|sp| crate::tui::width::cols(&sp.content))
        .sum();
    if used <= width {
        return line;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut budget = width;
    for span in line.spans {
        if budget == 0 {
            break;
        }
        let w = crate::tui::width::cols(&span.content);
        if w < budget {
            budget -= w;
            out.push(span);
            continue;
        }
        // This span is the one that overflows: cut it and mark the elision.
        out.push(Span::styled(
            crate::tui::width::trunc_cols(&span.content, budget),
            span.style,
        ));
        budget = 0;
    }
    ratatui::text::Line::from(out)
}

fn trunc(s: &str, w: usize) -> String {
    // Cells, not chars: a CJK glyph or emoji is two columns wide, so
    // `chars().count()` overflowed every fixed-width cell (2026-11-17).
    crate::tui::width::trunc_cols(s, w)
}

/// Maximum composer content rows — beyond this the transcript would be
/// squeezed out, so the composer scrolls instead of growing forever.
const COMPOSER_MAX_ROWS: u16 = 8;

/// Visual rows the composer content needs at `width` columns.
///
/// COMPOSER-1 (2026-11-17): this used to compute hard (character) wrapping
/// while the widget renders with `Wrap { trim: false }` (word wrapping), so a
/// prose prompt needing MORE rows than the estimate had its last line clipped —
/// precisely while the operator was writing it. Ask the widget itself instead:
/// `Paragraph::line_count(width)` is the renderer's own answer, so the estimate
/// cannot drift from the drawing again.
fn composer_rows(input: &str, width: u16) -> u16 {
    let width = width.max(1);
    let line = composer_line(input, input.chars().count());
    let rows = Paragraph::new(Text::from(vec![line]))
        .wrap(Wrap { trim: false })
        .line_count(width);
    (rows.max(1).min(COMPOSER_MAX_ROWS as usize)) as u16
}

/// Total composer box height: wrapped content rows plus the rounded border.
fn composer_box_height(input: &str, area_width: u16) -> u16 {
    // Left and right border consume one column each.
    let content_width = area_width.saturating_sub(2);
    composer_rows(input, content_width) + 2
}

/// Styled composer line: the `"> "` prompt, text before the cursor, the
/// cursor cell (reversed), then the remainder. Pure, so the wrap behaviour
/// is testable without a Frame.
fn composer_line(input: &str, cursor: usize) -> Line<'static> {
    let prompt = Span::styled("> ", Style::default().fg(theme::GREEN).bg(theme::PANEL));
    let (before, at, after) = split_at_char(input, cursor);
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
        assert_eq!(
            shorten_home(std::path::Path::new("/home/randozart/Projects/VITRIOL")),
            "~/Projects/VITRIOL"
        );
        assert_eq!(shorten_home(std::path::Path::new("/home/randozart")), "~");
        assert_eq!(
            shorten_home(std::path::Path::new("/opt/stuff")),
            "/opt/stuff"
        );
    }

    #[test]
    fn session_label_named_unnamed_none() {
        let mut s = AppState::default();
        assert_eq!(session_label(&s), None);
        s.cwd = std::path::PathBuf::from("/home/randozart");
        s.session_id = "01a06380-1234".into();
        assert_eq!(session_label(&s).unwrap(), "SESSION ID: #01a06380 @ ~");
        s.cwd = std::path::PathBuf::from("/home/randozart/Projects/ontic");
        assert_eq!(
            session_label(&s).unwrap(),
            "SESSION ID: #01a06380 @ ~/Projects/ontic"
        );
        s.session_name = Some("vitriol session".into());
        assert_eq!(
            session_label(&s).unwrap(),
            "\"vitriol session\" @ ~/Projects/ontic"
        );
    }

    #[test]
    fn elide_keeps_prefix_and_path_tail() {
        let l = "SESSION ID: #01a06380 @ ~/Projects/VITRIOL";
        assert_eq!(elide_path_tail(l, 60), l); // fits — untouched
        let e = elide_path_tail(l, 34);
        assert!(e.starts_with("SESSION ID: #01a06380 @ …"), "got {}", e);
        assert!(crate::tui::width::cols(&e) <= 34);
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
        assert_eq!(
            motto_for(56).as_deref(),
            Some("VISITA INTERIOREM TERRAE RECTIFICANDO INVENIES OCCULTUM")
        );
        assert_eq!(
            motto_for(38).as_deref(),
            Some("VISITA INTERIOREM TERRAE RECTIFICANDO")
        );
        assert_eq!(motto_for(24).as_deref(), Some("VISITA INTERIOREM TERRAE"));
        assert_eq!(motto_for(17).as_deref(), Some("VISITA INTERIOREM"));
        assert_eq!(motto_for(6).as_deref(), Some("VISITA"));
        assert_eq!(motto_for(5), None);
        assert_eq!(motto_for(0), None);
        // Every returned form fits its width.
        for w in 0..=70usize {
            if let Some(m) = motto_for(w) {
                assert!(
                    crate::tui::width::cols(&m) <= w,
                    "width {w}: {:?} overflows",
                    m
                );
            }
        }
    }
}

// ── Composer wrapping (bug 2026-11-11: box grew, text clipped) ────────────
//
// The composer used to grow its box from 3 to 5 rows on raw character count
// while rendering the content as a single Line — the box got taller and the
// text still ran off the right edge. These tests pin the fix: row count is
// derived from WRAPPED rows, and the rendered buffer proves the text
// actually lands on a second physical row.
#[cfg(test)]
mod composer_wrap_tests {
    use super::*;
    use ratatui::{buffer::Buffer, widgets::Widget};

    /// The renderer's own row count for the composer's line, which is what
    /// `composer_rows` must return for anything under COMPOSER_MAX_ROWS.
    fn width_rows(input: &str, width: u16) -> usize {
        let line = composer_line(input, input.chars().count());
        Paragraph::new(Text::from(vec![line]))
            .wrap(Wrap { trim: false })
            .line_count(width.max(1))
    }

    #[test]
    fn short_input_is_one_row() {
        assert_eq!(composer_rows("hello", 60), 1);
        assert_eq!(composer_rows("", 60), 1);
    }

    /// Relation, not a magic number (2026-11-17): the composer asks the
    /// RENDERER for its row count, so pinning exact counts here would encode
    /// one ratatui version instead of the property we care about — an unbroken
    /// token needs several rows, and those rows are what the box reserves.
    #[test]
    fn unbroken_token_counts_wrapped_rows() {
        let input = "x".repeat(200);
        let rows = composer_rows(&input, 60);
        assert!(
            (4..=6).contains(&rows),
            "unbroken token at 60 cols gave {rows} rows"
        );
        assert_eq!(width_rows(&input, 60), rows as usize);
    }

    #[test]
    fn rows_follow_available_width_not_a_fixed_threshold() {
        let input = "y".repeat(100);
        let narrow = composer_rows(&input, 40);
        let wide = composer_rows(&input, 200);
        assert!(
            narrow > wide,
            "narrow box needs more rows: {narrow} vs {wide}"
        );
        assert_eq!(wide, 1, "100 chars fit in 200 columns");
        assert_eq!(width_rows(&input, 40), narrow as usize);
    }

    #[test]
    fn rows_are_clamped_so_the_transcript_keeps_its_space() {
        let huge = "z".repeat(10_000);
        assert_eq!(composer_rows(&huge, 20), COMPOSER_MAX_ROWS);
    }

    #[test]
    fn word_wrapped_text_is_never_underestimated() {
        let input = "word ".repeat(40); // 200 chars, wraps on spaces
        let rows = composer_rows(&input, 40);
        assert!(
            rows >= 5,
            "under-reported rows for word-wrapped text: {rows}"
        );
    }

    #[test]
    fn long_token_lands_on_more_than_one_physical_row() {
        // The actual regression: render at width 20 and assert the tail is
        // VISIBLE on later rows instead of being clipped off the right edge.
        // (Ratatui's word wrap moves an over-long token to a fresh row, so
        // row 0 may legitimately hold only the "> " prompt.)
        let input = "x".repeat(40);
        let line = composer_line(&input, crate::tui::width::cols(&input));
        let area = Rect::new(0, 0, 20, 3);
        let mut buf = Buffer::empty(area);
        Paragraph::new(Text::from(vec![line]))
            .wrap(Wrap { trim: false })
            .render(area, &mut buf);
        let rows: Vec<String> = (0..3)
            .map(|y| (0..20).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect();
        assert!(
            rows[0].starts_with('>'),
            "prompt on first row: {:?}",
            rows[0]
        );
        assert!(
            rows[1].contains('x'),
            "second physical row must carry the wrapped tail: {:?}",
            rows
        );
        // Nothing is lost: every character is still on screen across rows.
        let rendered: String = rows.concat();
        assert_eq!(
            rendered.matches('x').count(),
            40,
            "all 40 chars must remain visible after wrapping: {:?}",
            rows
        );
    }
}

#[cfg(test)]
mod render_smoke {
    //! Every surface at several terminal sizes (2026-11-13 crash hunt):
    //! a panic here reproduces an "Officina crashed" render path.

    use super::*;

    fn draw_at(state: &mut AppState, w: u16, h: u16) {
        let backend = ratatui::backend::TestBackend::new(w, h);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| render(f, state))
            .expect("draw must not panic");
    }

    fn base_state() -> AppState {
        let mut s = AppState::default();
        s.mark_activity();
        s.cwd = std::path::PathBuf::from("/tmp/proj");
        s.tool_start(
            Some("c1"),
            "bash",
            &serde_json::json!({"command": "brievc build c4probe.bv --verify-bodies", "timeout": 240}),
        );
        s.tool_update(Some("c1"), "bash", "compiling…\n");
        s.apply_notify(
            "hang-watchdog: \"brievc build\" — no output for 61s",
            "warning",
        );
        s.apply_notify(
            "note with a longer body that will wrap across the modal width",
            "info",
        );
        s.entries.push(crate::tui::state::ChatEntry::Notice(
            "multi-line table\nrow one\nrow two".into(),
        ));
        s
    }

    #[test]
    fn all_modals_render_at_common_sizes() {
        for (w, h) in [(40u16, 12u16), (80, 24), (120, 40)] {
            // fx modal
            let mut s = base_state();
            s.fx_open = true;
            for sel in 0..4 {
                s.fx_sel = sel;
                draw_at(&mut s, w, h);
            }
            // answers modal (with a doc)
            let mut s = base_state();
            s.answers_open = true;
            s.answers = Some(super::super::state::AnswerDoc {
                version: 1,
                ts: 1,
                question: "how does the cache layer work?".into(),
                answer: "It stores prompt KV per slot.\n".repeat(30),
            });
            draw_at(&mut s, w, h);
            // cost modal
            let mut s = base_state();
            s.cost_open = true;
            s.cost = Some(crate::tui::cost::CostReport::default());
            draw_at(&mut s, w, h);
            // routes modal (decision trail, incl. an empty ledger)
            let mut s = base_state();
            s.routes_open = true;
            s.routes = Some(crate::tui::routes::RouteReport::default());
            draw_at(&mut s, w, h);
            s.routes = Some(crate::tui::routes::RouteReport {
                day: "20261114".into(),
                probes: Default::default(),
                entries: vec![crate::tui::routes::RouteEntry {
                    ts: "2026-11-14T09:28:51.223Z".into(),
                    reason: "user".into(),
                    tier: "cloud".into(),
                    provider: "caduceus".into(),
                    model: "opencode-go/deepseek-v4.1-flash".into(),
                    profile: "auto".into(),
                    local_state: "down".into(),
                    latency_ms: 17_908,
                    tokens_in: 3536,
                    tokens_out: 1319,
                    cost_estimate: 0.0,
                    outcome: "ok".into(),
                    candidates_skipped: vec!["dahl/x: cooling 12m".into()],
                }],
            });
            draw_at(&mut s, w, h);
            // review findings
            let mut s = base_state();
            s.review_open = true;
            s.review = crate::tui::review::parse(
                r#"{"version":1,"preset":"uncommitted","summary":"s",
                    "findings":[{"severity":"high","file":"a.rs","line":"3","title":"t","detail":"d"}]}"#,
            );
            draw_at(&mut s, w, h);
            // cross-project sessions
            let mut s = base_state();
            s.xsessions_open = true;
            s.xsessions_open_with(Vec::new());
            draw_at(&mut s, w, h);
            // model picker (grouped: OFFICINA → OpenCode Go → providers)
            let mut s = base_state();
            s.models_open = true;
            s.models = serde_json::from_value(serde_json::json!([
                {"id":"lapis","provider":"officina","name":"Lapis Occultus"},
                {"id":"nimbus","provider":"officina","name":"Nimbus"},
                {"id":"glm-5.2","provider":"opencode-go","name":"GLM-5.2","contextWindow":1000000},
                {"id":"glm-5.2","provider":"caduceus","name":"GLM-5.2 gateway","contextWindow":1000000},
                {"id":"glm-4.6","provider":"zai","name":"GLM 4.6","contextWindow":200000}
            ]))
            .unwrap();
            s.models_sel = 2;
            draw_at(&mut s, w, h);
            // status
            let mut s = base_state();
            s.status_open_modal();
            draw_at(&mut s, w, h);
            // caduceus page, every tab (config rows injected)
            let mut s = base_state();
            if let Some(page) = s.caduceus.as_mut() {
                page.tab = crate::tui::caduceus::Tab::Vitriol;
                page.engine = Some(crate::tui::caduceus::probe_engine());
                page.config = Some(crate::tui::engine_config::parse_rows(
                    "[gpu]\ntensor_split = 27,21\n[model]\nmmproj =\n",
                ));
            } else {
                s.caduceus = Some(crate::tui::caduceus::Page::default());
                let page = s.caduceus.as_mut().unwrap();
                page.tab = crate::tui::caduceus::Tab::Vitriol;
                page.config = Some(crate::tui::engine_config::parse_rows(
                    "[gpu]\ntensor_split = 27,21\n[model]\nmmproj =\n",
                ));
            }
            for tab in crate::tui::caduceus::Tab::ALL {
                if let Some(page) = s.caduceus.as_mut() {
                    page.set_tab(tab);
                }
                draw_at(&mut s, w, h);
            }
        }
    }

    #[test]
    fn watchdog_card_and_fire_render_with_facts() {
        let mut s = base_state();
        let now = crate::tui::caduceus::now_ms();
        s.watchdog = Some(crate::tui::state::WatchdogDoc {
            version: 1,
            ts: now,
            kind: "verdict".into(),
            command: "brievc build c4probe.bv".into(),
            elapsed_s: 252,
            stalled_s: 189,
            verdict: "HUNG".into(),
            reason: "waiting on stdin".into(),
            next_check_at: now + 21_000,
            phase: "rejudge".into(),
            ..Default::default()
        });
        s.fire_on = true;
        s.fire_target = 0.8;
        s.fire_level = 0.8;
        for (w, h) in [(40u16, 12u16), (100, 30)] {
            draw_at(&mut s, w, h);
        }
    }
}

#[cfg(test)]
mod grind_chip_tests {
    use super::*;

    #[test]
    fn grind_mode_gets_its_own_alchemical_chip() {
        assert_eq!(mode_chip("GRIND"), Some(("∞", theme::VIOLET)));
        assert_eq!(mode_chip("grind"), Some(("∞", theme::VIOLET)));
        let mut s = AppState::default();
        s.agent_mode_label = Some("GRIND".into());
        s.agent_mode_glyph = Some("∞".into());
        let title = composer_title(&s).expect("chip present");
        let text: String = title.iter().map(|sp| sp.content.to_string()).collect();
        assert!(text.contains("GRIND"), "{text}");
        assert!(text.contains('∞'), "{text}");
    }
}

#[cfg(test)]
mod settings_chat_render_tests {
    use super::*;

    fn screen(state: &mut AppState, w: u16, h: u16) -> String {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, state)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn settings_choices_fit_small_screens() {
        for command in [
            "/fire",
            "/chat-bg",
            "/shimmer",
            "/sidebar",
            "/thinking",
            "/thinking-view",
            "/settings",
            "/diag",
        ] {
            check_menu_sizes(command);
        }
    }

    fn check_menu_sizes(command: &str) {
        for (w, h) in [(20, 8), (40, 12), (80, 24), (120, 40)] {
            let mut state = AppState::default();
            settings::command(&mut state, command);
            let last = state.setting_menu.as_ref().unwrap().choices.len() - 1;
            state.setting_menu.as_mut().unwrap().selected = last;
            let text = screen(&mut state, w, h);
            assert!(
                text.contains('›'),
                "selected choice absent at {w}x{h}: {command}\n{text}"
            );
        }
    }

    #[test]
    fn compaction_is_in_chat_once_and_respects_scrollback() {
        let mut state = AppState::default();
        state.is_compacting = true;
        state.compaction_started_at = Some(std::time::Instant::now());
        state
            .entries
            .push(ChatEntry::Notice("transcript message".into()));
        let text = screen(&mut state, 80, 24);
        assert_eq!(text.matches("compacting context").count(), 1, "{text}");
        assert!(!text.contains("summarizing the session"));
        for i in 0..70 {
            state
                .entries
                .push(ChatEntry::Notice(format!("older message {i}")));
        }
        state.scroll = u16::MAX;
        let text = screen(&mut state, 80, 24);
        assert!(
            !text.contains("compacting context"),
            "compaction must scroll with chat: {text}"
        );
    }

    /// Background colours of every rendered cell (2026-11-16: the user-turn
    /// band is a bg-only effect, so symbols alone cannot assert it).
    fn screen_bgs(state: &mut AppState, w: u16, h: u16) -> Vec<Vec<ratatui::style::Color>> {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render(f, state)).unwrap();
        let buf = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].bg).collect::<Vec<_>>())
            .collect()
    }

    /// Rows whose background is the user-turn band.
    fn band_rows(bgs: &[Vec<ratatui::style::Color>]) -> Vec<usize> {
        bgs.iter()
            .enumerate()
            .filter(|(_, row)| row.iter().any(|c| *c == theme::USER_BG))
            .map(|(y, _)| y)
            .collect()
    }

    #[test]
    fn user_turn_gets_a_background_band() {
        let mut state = AppState::default();
        state
            .entries
            .push(ChatEntry::User("add a band here".into()));
        state.entries.push(ChatEntry::Assistant("done".into()));
        let bgs = screen_bgs(&mut state, 60, 18);
        let rows = band_rows(&bgs);
        assert_eq!(rows.len(), 1, "exactly the user row is banded: {rows:?}");
        // The band spans the chat width minus the gauge column.
        let y = rows[0];
        assert_eq!(bgs[y][0], theme::USER_BG);
        // The gauge column is the LAST column of the chat area (chat_width is
        // width - 1), not the one before it.
        assert_ne!(bgs[y][59], theme::USER_BG, "gauge column stays clean");
    }

    #[test]
    fn band_off_restores_the_plain_transcript() {
        let mut state = AppState::default();
        state.user_turn_bg = false;
        state
            .entries
            .push(ChatEntry::User("add a band here".into()));
        state.entries.push(ChatEntry::Assistant("done".into()));
        let bgs = screen_bgs(&mut state, 60, 18);
        assert!(
            band_rows(&bgs).is_empty(),
            "/chat-bg off must leave no banded cell"
        );
    }

    #[test]
    fn band_covers_a_wrapped_turn_and_skips_the_gap() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::User(
            "a long user message that wraps onto more than one row when the column is narrow"
                .into(),
        ));
        state.entries.push(ChatEntry::Assistant("ok".into()));
        state.entries.push(ChatEntry::User("second".into()));
        let bgs = screen_bgs(&mut state, 40, 20);
        let rows = band_rows(&bgs);
        assert!(rows.len() >= 3, "wrapped turn + second turn: {rows:?}");
        // No banded row is blank: the inter-entry gap stays BG.
        let text = screen(&mut state, 40, 20);
        let lines: Vec<&str> = text.lines().collect();
        for y in &rows {
            assert!(
                !lines[*y].trim().is_empty(),
                "gap row {y} was banded:\n{text}"
            );
        }
    }

    /// 2026-11-16: the hints row carries keybindings ONLY. A notice used to be
    /// appended here, where a long or multi-line notice collided with the hints
    /// and made the row shimmer whenever it changed.
    #[test]
    fn hints_row_never_carries_a_notice() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::User("hi".into()));
        state.notice = Some(("nimbus · ctx 12k · next: caduceus/x".into(), "info".into()));
        // The strong form: the hints row must be BYTE-IDENTICAL with and
        // without a notice. That cannot be satisfied by "the notice merely
        // landed somewhere else".
        let plain = screen(&mut state, 200, 20);
        let plain_hints = plain
            .lines()
            .find(|l| l.contains("enter send"))
            .expect("hints row")
            .to_string();
        state.notice = Some(("nimbus · ctx 12k · next: caduceus/x".into(), "info".into()));
        let text = screen(&mut state, 200, 20);
        let hints = text
            .lines()
            .find(|l| l.contains("enter send"))
            .expect("hints row")
            .to_string();
        assert_eq!(plain_hints, hints, "a notice changed the hints row");
        assert!(
            !hints.contains("next:"),
            "notice leaked into hints: {hints}"
        );
        // ...and it is on its own row above.
        let notice_row = text
            .lines()
            .find(|l| l.contains("next:"))
            .expect("notice strip");
        assert!(notice_row.trim_start().starts_with('·'), "{notice_row}");
    }

    /// Multi-line notices (the /nimbus table) render as `head … (+N lines)`
    /// instead of being cut to 40 characters.
    #[test]
    fn multi_line_notice_shows_a_head_and_a_count() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::User("hi".into()));
        state.notice = Some((
            "nimbus · 3 lane(s)\n  lane a\n  lane b".to_string(),
            "info".to_string(),
        ));
        let text = screen(&mut state, 120, 20);
        assert!(text.contains("nimbus · 3 lane(s)"), "{text}");
        assert!(text.contains("(+2 lines)"), "{text}");
    }

    /// A long notice is truncated to the row, not to a hard-coded 40 columns.
    #[test]
    fn long_notice_is_truncated_to_the_row() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::User("hi".into()));
        state.notice = Some(("x".repeat(300), "warning".to_string()));
        let text = screen(&mut state, 80, 20);
        let notice_row = text
            .lines()
            .find(|l| l.contains('x'))
            .expect("notice strip");
        assert!(
            crate::tui::width::cols(&notice_row) <= 80,
            "notice overflows the row: {} cols",
            crate::tui::width::cols(&notice_row)
        );
    }

    /// The notice row is reserved from the first notice onward, so notices can
    /// never move the transcript afterwards.
    #[test]
    fn notice_row_is_reserved_once_seen() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::User("hi".into()));
        state.notice = Some(("first".to_string(), "info".to_string()));
        let with_notice = screen(&mut state, 80, 20);
        assert!(state.notice_row_sticky);
        // The notice clears; the row stays reserved (the hints row's index must
        // not move), which is what keeps the layout still.
        state.notice = None;
        let after = screen(&mut state, 80, 20);
        let row_of = |t: &str| t.lines().position(|l| l.contains("enter send"));
        assert_eq!(row_of(&with_notice), row_of(&after));
    }

    /// F-C1 (owner 2026-11-16, again 2026-11-17): the flames must TAPER — each
    /// row above the base is narrower than the one below it, INCLUDING the row
    /// that sits on the rule.
    ///
    /// The previous version asserted that the bottom body row was unchanged by
    /// the taper and then looked one row higher to find any taper at all. That
    /// encoded the defect (`rn == 0` exempted that row) instead of catching it:
    /// the owner saw two identical rows while this test was green.
    ///
    /// Differential on purpose: the same state renders with the taper off and
    /// with the shipped value, so jitter cannot satisfy the assertion.
    #[test]
    fn flames_taper_with_height() {
        // Lit width per body row, base row LAST (the rule row is last of all).
        fn widths(taper: f64, level: f64) -> Vec<usize> {
            let mut state = AppState::default();
            state.entries.push(ChatEntry::Notice("hello".into()));
            state.fire_on = true;
            state.fire_level = level;
            state.fire_target = level;
            state.fire_taper = taper;
            let text = screen(&mut state, 120, 24);
            let lines: Vec<&str> = text.lines().collect();
            let rule = lines
                .iter()
                .position(|l| l.chars().filter(|c| *c == '_').count() > 3)
                .expect("divider row");
            let lit = |y: usize| {
                lines[y]
                    .chars()
                    .filter(|c| !c.is_whitespace() && !matches!(c, '\u{2500}'))
                    .count()
            };
            let mut out: Vec<usize> = (rule.saturating_sub(6)..rule).map(lit).collect();
            out.push(lit(rule));
            out
        }

        // Levels that produce 2, 3, 4, 5 and 6 flame rows (rows_for(level)).
        for level in [0.05, 0.3, 0.55, 0.8, 1.0] {
            let flat = widths(0.0, level);
            let tapered = widths(0.55, level);
            let last = tapered.len() - 1;

            // The rule row is never tapered (the base stays unbroken)…
            assert_eq!(
                flat[last], tapered[last],
                "level {level}: the taper must not narrow the rule: flat={flat:?} tapered={tapered:?}"
            );
            // …but the row ABOVE the rule is narrower than the rule. This is
            // the assertion the old test skipped.
            assert!(
                tapered[last - 1] < tapered[last],
                "level {level}: the row above the rule is not narrower: tapered={tapered:?}"
            );
            // Negative control: the SAME row is wider with the taper off, so
            // the narrowing above is attributable to the taper and not to the
            // flame's own shape. (The rule is always at least as wide as a body
            // row — it claims every lit column — so comparing rule vs row
            // alone would pass even with the feature removed.)
            assert!(
                tapered[last - 1] < flat[last - 1],
                "level {level}: the taper did not narrow the first body row: \
                 flat={flat:?} tapered={tapered:?}"
            );
            // Monotone: widths never increase going UP from the base.
            let base = tapered[last];
            let mut seen: Vec<usize> = vec![base];
            seen.extend(tapered[..last].iter().rev().copied());
            for pair in seen.windows(2) {
                assert!(
                    pair[1] <= pair[0],
                    "level {level}: widths increase going up from the base: {seen:?} (base-first)"
                );
            }
        }
    }

    /// The card's two horizontal rules must be the same length (owner report
    /// 2026-11-16: the top edge stopped at 60 columns while the bottom spanned
    /// the card).
    #[test]
    fn watchdog_card_borders_match_width() {
        let mut s = AppState::default();
        s.entries.push(ChatEntry::Watchdog {
            text: "hang-watchdog: watch".into(),
            doc: None,
        });
        let text = screen(&mut s, 120, 24);
        let lines: Vec<&str> = text.lines().collect();
        // The card is the block whose title row carries the watch label; its
        // closing rule is the next row with a bottom-left corner.
        let top = lines
            .iter()
            .position(|l| l.contains(" watch "))
            .expect("card top rule");
        let bottom = lines
            .iter()
            .enumerate()
            .skip(top + 1)
            .find(|(_, l)| l.contains('\u{2570}'))
            .map(|(i, _)| i)
            .expect("card bottom rule");
        // Compare the CARD's own span, not the whole screen row: the sidebar
        // sits beside it and has rules of its own. Char indices throughout —
        // braille/box glyphs are multi-byte.
        let span = |line: &str, open: char, close: char| -> usize {
            let chars: Vec<char> = line.chars().collect();
            let from = chars.iter().position(|c| *c == open).unwrap_or(0);
            let to = chars[from..]
                .iter()
                .position(|c| *c == close)
                .map(|i| from + i)
                .unwrap_or_else(|| chars.len().saturating_sub(1));
            to.saturating_sub(from) + 1
        };
        assert_eq!(
            span(lines[top], '\u{256d}', '\u{256e}'),
            span(lines[bottom], '\u{2570}', '\u{256f}'),
            "top and bottom rules differ:\n{text}"
        );
    }

    /// Row with the most underscores — the divider line.
    fn divider_row(text: &str) -> String {
        text.lines()
            .max_by_key(|l| l.chars().filter(|c| *c == '_').count())
            .unwrap_or("")
            .to_string()
    }

    #[test]
    fn divider_is_all_underscores_without_fire() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Notice("hello".into()));
        state.fire_on = false;
        let text = screen(&mut state, 80, 24);
        let row = divider_row(&text);
        assert!(row.contains('_'), "divider missing:\n{text}");
        assert!(
            row.chars().all(|c| c == '_' || c == ' '),
            "no fire ⇒ plain underscore rule, got {row:?}"
        );
    }

    /// 2026-11-16 (owner): the fire ignites ON the divider — a dead fire is a
    /// plain underscore rule, a live one lights it selectively with the
    /// thinnest braille dot.
    #[test]
    fn divider_lights_ignition_dots_when_the_fire_burns() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Notice("hello".into()));
        state.fire_on = true;
        state.fire_level = 1.0;
        state.fire_target = 1.0;
        let text = screen(&mut state, 80, 24);
        let row = divider_row(&text);
        assert!(
            row.chars().any(|c| c != '_' && c != ' '),
            "the burning flame must claim part of the rule: {row:?}"
        );
        // The rule now carries the fire's own base glyphs (2026-11-16): any
        // braille ramp glyph is legitimate, but nothing else.
        assert!(
            row.chars()
                .all(|c| c == '_' || c == ' ' || (0x2800..=0x28FF).contains(&(c as u32))),
            "the divider carries only underscores and braille, got {row:?}"
        );
        assert!(
            row.chars().filter(|c| *c == '_').count() > 0,
            "the rule is never consumed wholesale: {row:?}"
        );
    }

    /// Index of the status-band row that carries the context fill.
    fn ctx_row_index(text: &str) -> Option<usize> {
        text.lines().position(|l| l.contains(" ctx "))
    }

    fn band_state() -> AppState {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Notice("hello".into()));
        state.ctx_usage = Some((12_345, 262_144));
        // Pretend the parts have already settled (the dwell is wall-clock).
        state.band_ind.force(1);
        state.band_ctx.force(1);
        state.set_widget_placed(
            "vitriol-decode",
            vec!["◈ VITRIOL  12.3 tok/s".into()],
            Some("belowEditor".into()),
        );
        state
    }

    /// Operator report (2026-11-16): "the ctx field occasionally flickers to
    /// look like a different field for one frame". A transient indicator line
    /// must never take the ctx row's slot.
    #[test]
    fn ctx_row_keeps_its_slot_when_an_indicator_appears() {
        let mut state = band_state();
        let before = ctx_row_index(&screen(&mut state, 80, 24))
            .expect("ctx row must be present in the steady state");

        // A second indicator line for one frame (decode strips do this).
        state.set_widget_placed(
            "phase-model",
            vec!["◇ plan: some-model".into()],
            Some("belowEditor".into()),
        );
        let during = screen(&mut state, 80, 24);
        assert_eq!(
            ctx_row_index(&during),
            Some(before),
            "transient indicator moved the ctx row:\n{during}"
        );
        assert!(
            during.contains(" ctx "),
            "the ctx field must still be rendered:\n{during}"
        );

        // And back to one line: still the same row.
        state.set_widget_placed("phase-model", vec![], Some("belowEditor".into()));
        let after = screen(&mut state, 80, 24);
        assert_eq!(ctx_row_index(&after), Some(before));
    }

    /// The ctx row is bottom-anchored in the band: its position depends only
    /// on settled heights, so indicator churn can never shift it.
    #[test]
    fn ctx_row_is_bottom_anchored() {
        for ind in 0..=2u16 {
            let mut state = band_state();
            state.band_ind.force(ind);
            state.band_ctx.force(1);
            state.widgets.clear();
            for line in 0..ind {
                state.set_widget_placed(
                    &format!("w{line}"),
                    vec![format!("indicator {line}")],
                    Some("belowEditor".into()),
                );
            }
            let text = screen(&mut state, 80, 24);
            let ctx = ctx_row_index(&text).expect("ctx row missing");
            // The divider is the row above the band; ctx is the band's last
            // row, i.e. one above the composer's top border.
            let composer_top = text
                .lines()
                .position(|l| l.trim_start().starts_with('╭'))
                .expect("composer border");
            assert_eq!(
                ctx + 1,
                composer_top,
                "ctx row must sit on the last band row (ind={ind}):\n{text}"
            );
        }
    }

    /// I27 (third revision, 2026-11-16): THE FIRE SPAWNS ON THE RULE. The rule
    /// cell carries the flame's own base density — it is the hottest row of the
    /// column, strictly denser than the cell above it — and at low load the
    /// scaled value drops below the smoulder threshold so underscores survive
    /// *between* the flamelets.
    #[test]
    fn fire_spawns_on_the_rule() {
        const RAMP: [char; 8] = ['⠀', '⡀', '⡄', '⡆', '⡇', '⣇', '⣧', '⣷'];
        for level in [0.3_f64, 0.6, 1.0] {
            let mut state = AppState::default();
            state.entries.push(ChatEntry::Notice("hello".into()));
            state.fire_on = true;
            state.fire_level = level;
            state.fire_target = level;
            let text = screen(&mut state, 90, 24);
            let lines: Vec<&str> = text.lines().collect();
            let idx = lines
                .iter()
                .position(|l| l.chars().filter(|c| *c == '_').count() > 5)
                .expect("divider row");
            assert!(idx > 0);
            let rule: Vec<char> = lines[idx].chars().collect();
            let above: Vec<char> = lines[idx - 1].chars().collect();
            let mut claimed = 0;
            let mut strictly_denser = 0;
            for (x, c) in rule.iter().enumerate() {
                if *c == '_' || *c == ' ' {
                    continue;
                }
                claimed += 1;
                let below_rank = RAMP.iter().position(|d| d == c);
                let above_rank = above.get(x).and_then(|u| RAMP.iter().position(|d| d == u));
                // The rule is the hottest row: never lighter than the cell above
                // it. Rounding can tie two adjacent ramp steps, so the strict
                // form is checked in aggregate below.
                if let (Some(b), Some(a)) = (below_rank, above_rank) {
                    assert!(
                        b >= a,
                        "level {level}: rule cell {x} ({c:?}) is lighter than the row above ({:?})",
                        above[x]
                    );
                    if b > a {
                        strictly_denser += 1;
                    }
                } else {
                    assert!(
                        below_rank.is_some(),
                        "rule glyph is not a ramp glyph: {c:?}"
                    );
                }
            }
            assert!(claimed > 0, "no claimed columns at level {level}:\n{text}");
            if level >= 0.6 {
                assert!(
                    strictly_denser * 2 >= claimed,
                    "level {level}: only {strictly_denser}/{claimed} rule cells are denser than the row above"
                );
            }
            assert_ne!(
                lines[idx],
                lines[idx - 1],
                "level {level}: rule duplicates the row above"
            );
        }
    }

    /// I24: the flame always *starts* on the rule — wherever the rule is
    /// claimed, the flame's bottom row above it must carry a glyph.
    /// BAND-1 (owner 2026-11-17: typed text appeared "between ctx and route").
    /// A belowEditor widget carrying the draft — or a prediction that prefixes
    /// it — must not reach the band; the composer owns that text.
    ///
    /// The render-level assertion compares the WHOLE screen with and without
    /// the echoing widget: they must be identical, because the band is the only
    /// thing that widget could have changed. The negative control keeps an
    /// unrelated band row, which must change the screen — so the test cannot be
    /// satisfied by dropping every band row.
    /// COMPOSER-1 (2026-11-17): the box height was computed with hard character
    /// wrapping while the widget renders word-wrapped, so prose prompts lost
    /// their last line exactly while being written. The box must now be at least
    /// as tall as the renderer says the text is.
    /// WRAP-1 (2026-11-17): the transcript does not wrap, so an overlong line
    /// must be elided with a visible marker instead of vanishing at the edge.
    /// SCROLL-1 lives in the same invariant: elision keeps 1 line = 1 row, which
    /// is what the scroll maths assumes.
    #[test]
    fn overlong_lines_are_elided_not_clipped() {
        use ratatui::text::{Line, Span};
        // Pure: a wide line loses its tail but keeps its styles and gains a
        // marker, and never exceeds the budget.
        let long = Line::from(vec![
            Span::raw("see "),
            Span::styled("x".repeat(200), Style::new().fg(theme::GOLD)),
        ]);
        let cut = elide_line(long, 40);
        let out: String = cut.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(out.ends_with('…'), "no elision marker: {out:?}");
        assert!(
            crate::tui::width::cols(&out) <= 40,
            "elision overran: {out:?}"
        );
        assert!(
            cut.spans.iter().any(|s| s.style.fg == Some(theme::GOLD)),
            "elision dropped the styling"
        );
        // A line that fits is returned untouched (no marker).
        let fits = Line::from(Span::raw("short"));
        assert_eq!(elide_line(fits.clone(), 40).spans.len(), fits.spans.len());
        // Screen level: with an overlong line in the transcript, no drawn row
        // exceeds the frame. (Elision is the mechanism that guarantees it —
        // markdown pre-wraps most content, so this uses a raw entry.)
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Warn(format!(
            "https://example.com/{}",
            "y".repeat(300)
        )));
        let text = screen(&mut state, 60, 20);
        for line in text.lines() {
            assert!(
                crate::tui::width::cols(line) <= 60,
                "a drawn row exceeds the frame: {:?}",
                line
            );
        }
    }

    #[test]
    fn composer_box_fits_a_wrapped_prompt() {
        let prose = "please refactor the render audit findings one at a time and \
                     keep every existing test green while doing it, then summarise";
        for width in [40u16, 60, 80, 120] {
            let content_w = width.saturating_sub(2);
            let line = composer_line(prose, prose.chars().count());
            let needed = Paragraph::new(Text::from(vec![line]))
                .wrap(Wrap { trim: false })
                .line_count(content_w);
            let boxed = composer_box_height(prose, width);
            assert!(
                boxed as usize >= needed + 2,
                "width {width}: box {boxed} < needed {}",
                needed + 2
            );
        }
        // And a short prompt still gets the minimal two-row box.
        assert_eq!(composer_box_height("hi", 80), 3);
    }

    #[test]
    fn band_never_echoes_the_draft() {
        fn base() -> AppState {
            let mut state = AppState::default();
            state.entries.push(ChatEntry::Notice("hi".into()));
            state.input = "run the render audit".into();
            state
        }
        fn with_widget(key: &str, line: &str) -> String {
            let mut state = base();
            state.widgets.push(crate::tui::state::WidgetBlock {
                key: key.into(),
                lines: vec![line.into()],
                placement: Some("belowEditor".into()),
            });
            screen(&mut state, 120, 24)
        }

        // Pure predicate first: an echo is an equal line or a prefix either way.
        assert!(band_echoes_draft(
            &["run the render audit now".to_string()],
            "run the render audit"
        ));
        assert!(band_echoes_draft(
            &["run the render".to_string()],
            "run the render audit"
        ));
        assert!(!band_echoes_draft(
            &["plan · 3 steps".to_string()],
            "run the render audit"
        ));
        assert!(!band_echoes_draft(&["anything".to_string()], "   "));

        // Render level: both cases carry the SAME neutral band row (widget
        // presence itself changes the footer hint), so the only difference is
        // the prediction. Filtered ⇒ identical; unrelated ⇒ different.
        fn render(extra: &str) -> String {
            let mut state = base();
            state.widgets.push(crate::tui::state::WidgetBlock {
                key: "plan-mode".into(),
                lines: vec!["plan · 3 steps".into()],
                placement: Some("belowEditor".into()),
            });
            if !extra.is_empty() {
                state.widgets.push(crate::tui::state::WidgetBlock {
                    key: "composer-prediction".into(),
                    lines: vec![extra.into()],
                    placement: Some("belowEditor".into()),
                });
            }
            // The band only renders once its height has SETTLED (BAND_SETTLE);
            // force it so this test measures the filter, not the dwell.
            state.band_ind.force(2);
            screen(&mut state, 120, 24)
        }
        assert_eq!(
            render("run the render audit now"),
            render(""),
            "an echoing band widget changed the screen"
        );
        assert_ne!(
            render("verify failed — run make test"),
            render(""),
            "the control did not render, so the filter is over-broad"
        );
    }

    #[test]
    fn flame_meets_its_line() {
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Notice("hello".into()));
        state.fire_on = true;
        state.fire_level = 1.0;
        state.fire_target = 1.0;
        let text = screen(&mut state, 80, 24);
        let lines: Vec<&str> = text.lines().collect();
        let idx = lines
            .iter()
            .position(|l| l.chars().filter(|c| *c == '_').count() > 5)
            .expect("divider row");
        assert!(idx > 0);
        let rule: Vec<char> = lines[idx].chars().collect();
        let above: Vec<char> = lines[idx - 1].chars().collect();
        // Direction matters (2026-11-17): the RULE is the flame's full-width
        // base and the taper starts immediately above it, so the rule may be
        // wider than the body — that is the pedestal the fire stands on. What
        // must never happen is the reverse: a burning body cell floating where
        // the rule has no flame, which is the "detached band" the owner
        // reported on 2026-11-16.
        let mut checked = 0;
        for (x, c) in above.iter().enumerate() {
            if *c == ' ' {
                continue;
            }
            let down = rule.get(x).copied().unwrap_or(' ');
            assert_ne!(
                down, ' ',
                "column {x} burns above the rule but the rule is empty there"
            );
            checked += 1;
        }
        assert!(checked > 0, "no body columns at full load:\n{text}");
        // And the rule itself is continuous at full load: no gaps.
        let lit_rule = rule.iter().filter(|c| **c != ' ' && **c != '_').count();
        assert!(
            lit_rule > rule.iter().filter(|c| **c == '_').count(),
            "the rule is not continuous at full load:\n{text}"
        );
    }

    #[test]
    fn question_panel_draws_over_the_transcript() {
        // 2026-11-16 operator report: the question menu opened BEHIND the
        // text because the transcript painted after it. The panel is an
        // overlay and must win every cell it covers.
        let mut state = AppState::default();
        for i in 0..60 {
            state
                .entries
                .push(ChatEntry::Notice(format!("older message {i}")));
        }
        state.question = Some(crate::tui::question::QuestionDoc {
            version: 1,
            id: "q1".into(),
            question: "Which plan?".into(),
            options: vec!["alpha".into(), "beta".into()],
            ts: 0,
        });
        let text = screen(&mut state, 80, 24);
        assert!(text.contains("Which plan?"), "question hidden:\n{text}");
        assert!(text.contains("alpha") && text.contains("beta"), "{text}");
        assert!(
            text.contains("question"),
            "panel title (*question*) hidden behind the transcript:\n{text}"
        );
    }
}
