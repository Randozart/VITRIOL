//! Ratatui rendering: the dashboard layout in the Vitriolum theme.
//!
//! V1 ships the DASHBOARD tab only. Layout: a VITRIOL banner + active tab, a
//! row of three service cards (GEN / HERMETIS / EMBED), a GPU card with
//! btop-style gauges, and a decode-t/s sparkline card. All rendering is
//! snapshot-driven and never panics on a down service.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Gauge, Paragraph, Sparkline, Wrap};
use ratatui::Frame;

use crate::app::{App, LogSource, Tab};
use crate::control::Action;
use crate::model::Snapshot;
use crate::{subsystems, theme};

/// Draw the whole UI for the current app state.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(frame, rows[0], app);
    match app.tab {
        Tab::Dashboard => render_dashboard(frame, rows[1], app),
        Tab::Gpu => render_gpu_tab(frame, rows[1], app),
        Tab::Logs => render_logs_tab(frame, rows[1], app),
        Tab::Controls => render_controls_tab(frame, rows[1], app),
        Tab::Hermetis => render_hermetis_tab(frame, rows[1], app),
        Tab::Subsystems => render_subsystems_tab(frame, rows[1], app),
        Tab::Profiles => render_profiles_tab(frame, rows[1], app),
        Tab::Guide => render_guide_tab(frame, rows[1], app),
    }
    render_footer(frame, rows[2], app);
}

/// Top banner: gold VITRIOL title + tab bar + project id.
fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![Span::styled(" VITRIOL ", theme::banner())];
    for tab in Tab::ALL {
        let style = if tab == app.tab {
            theme::title().add_modifier(Modifier::UNDERLINED)
        } else {
            theme::muted()
        };
        spans.push(Span::styled(format!(" {} ", tab.label()), style));
    }
    spans.push(Span::styled(
        format!("   {}", app.cfg.project_id),
        theme::muted(),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Bottom keybinding bar.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" [q] quit ", theme::muted()),
        Span::styled("[Tab] tab  ", theme::muted()),
        Span::styled("[r] refresh", theme::muted()),
    ];
    if app.tab == Tab::Logs {
        spans.push(Span::styled("  [1/2/3] log", theme::muted()));
    }
    if app.tab == Tab::Controls {
        spans.push(Span::styled(
            "  [↑/↓] move  [enter] run  [x] abort",
            theme::muted(),
        ));
    }
    spans.push(Span::styled("  ·  vitriol-tui v0.1.0", theme::muted()));
    if app.snapshot.is_empty() {
        spans.push(Span::styled(
            "  ·  stack unreachable — nothing on :8279/:7980/:4779",
            Style::default().fg(theme::ORANGE),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Dashboard body: service cards row + GPU/decode row.
fn render_dashboard(frame: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::vertical([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)]).split(area);

    let services = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(rows[0]);

    render_gen_card(frame, services[0], &app.snapshot);
    render_hermetis_card(frame, services[1], &app.snapshot);
    render_embed_card(frame, services[2], &app.snapshot);

    let bottom =
        Layout::horizontal([Constraint::Ratio(3, 5), Constraint::Ratio(2, 5)]).split(rows[1]);

    render_gpu_card(frame, bottom[0], &app.snapshot);
    render_decode_card(frame, bottom[1], app);
}

/// Build a bordered panel block with a themed title.
fn panel(title: &str, up: bool) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::panel_border(up))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(title, theme::title()))
        .title_bottom(Span::styled(" vitriol-tui ", theme::muted()))
}

/// Build a neutral (non-service) panel: dim border, gold title.
fn panel_neutral(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_DIM))
        .style(Style::new().bg(theme::PANEL))
        .title(Span::styled(title, Style::default().fg(theme::GOLD)))
}

/// GEN card: status, model, ctx/parallel, live decode t/s.
fn render_gen_card(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let g = &snap.gen;
    let title = format!(" {} GEN ", theme::GLYPH_GEN);
    let block = panel(&title, g.up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        status_line(g.up, "llama-server"),
        kv_line("model", g.model.as_deref().unwrap_or("—")),
        kv_line(
            "ctx",
            &g.n_ctx.map(|n| n.to_string()).unwrap_or_else(|| "—".into()),
        ),
        kv_line(
            "parallel",
            &g.n_parallel
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into()),
        ),
        Line::from(vec![
            Span::styled("decode  ", theme::muted()),
            Span::styled(
                format!("{:.1} t/s", g.decode_t_s),
                if g.decode_t_s > 0.0 {
                    theme::live()
                } else {
                    theme::muted()
                },
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// HERMETIS card: status + episode/node/session counts.
fn render_hermetis_card(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let h = &snap.hermetis;
    let title = format!(" {} HERMETIS ", theme::GLYPH_HERM);
    let block = panel(&title, h.up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        status_line(h.up, "memory server"),
        count_line("episodes", h.episodes),
        count_line("nodes", h.nodes),
        count_line("sessions", h.sessions),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// EMBED card: status.
fn render_embed_card(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let e = &snap.embed;
    let title = format!(" {} EMBED ", theme::GLYPH_EMBED);
    let block = panel(&title, e.up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        status_line(e.up, "bge embedder"),
        kv_line("mode", "cpu"),
        Line::from(vec![Span::styled("bge ctx 512, ngl 0", theme::muted())]),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

/// GPU card: btop-style gauges for VRAM + utilisation, temp/power, processes.
fn render_gpu_card(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let up = snap.gpu.as_ref().map(|g| g.present).unwrap_or(false);
    let title = format!(" {} GPU ", theme::GLYPH_GPU);
    let block = panel(&title, up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(gpu) = &snap.gpu else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nvidia-smi unavailable",
                theme::muted(),
            ))),
            inner,
        );
        return;
    };

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(&gpu.name, theme::text()),
            Span::styled(
                format!("  {:.0}W  {}°C", gpu.power_w, gpu.temp_c),
                theme::muted(),
            ),
        ])),
        rows[0],
    );

    let vram_ratio = if gpu.vram_total_mib > 0 {
        gpu.vram_used_mib as f64 / gpu.vram_total_mib as f64
    } else {
        0.0
    };
    render_gauge_row(
        frame,
        rows[1],
        &format!(
            "VRAM  {:.2}/{:.2} GiB  {:.0}%",
            gpu.vram_used_mib as f64 / 1024.0,
            gpu.vram_total_mib as f64 / 1024.0,
            vram_ratio * 100.0
        ),
        vram_ratio,
    );
    render_gauge_row(
        frame,
        rows[2],
        &format!("UTIL  {}%", gpu.util_pct),
        gpu.util_pct as f64 / 100.0,
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("PROCESSES", theme::muted()))),
        rows[3],
    );

    let proc_lines: Vec<Line> = gpu
        .processes
        .iter()
        .take(3)
        .map(|p| {
            Line::from(vec![
                Span::styled(format!("  {:<6} ", p.pid), theme::muted()),
                Span::styled(format!("{:<28}", short_name(&p.name)), theme::text()),
                Span::styled(
                    format!("{:.1} GiB", p.vram_mib as f64 / 1024.0),
                    theme::live(),
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(proc_lines), rows[4]);
}

/// One btop-style gauge row: a muted label line above a fill bar.
fn render_gauge_row(frame: &mut Frame, area: Rect, label: &str, ratio: f64) {
    let cols =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label, theme::muted()))),
        cols[0],
    );
    let fill_style = if ratio > 0.8 {
        theme::gauge_fill_warn()
    } else {
        theme::gauge_fill()
    };
    frame.render_widget(
        Gauge::default()
            .ratio(ratio.clamp(0.0, 1.0))
            .label("")
            .gauge_style(fill_style),
        cols[1],
    );
}

/// DECODE card: sparkline of recent t/s plus the latest value.
fn render_decode_card(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_neutral(" DECODE ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);

    let last = app.snapshot.gen.decode_t_s;
    let max = app.decode_history.iter().copied().fold(0.0f64, f64::max);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("{last:.1} t/s   peak {max:.1}"),
            if last > 0.0 {
                theme::live()
            } else {
                theme::muted()
            },
        )])),
        rows[0],
    );

    let data: Vec<u64> = app
        .decode_history
        .iter()
        .map(|v| (*v).max(0.0) as u64)
        .collect();
    frame.render_widget(
        Sparkline::default()
            .data(&data)
            .style(theme::sparkline())
            .max(data.iter().copied().max().unwrap_or(1)),
        rows[1],
    );
}

/// Full btop-style GPU panel: metric gauges on top, process table below.
fn render_gpu_tab(frame: &mut Frame, area: Rect, app: &App) {
    let Some(gpu) = &app.snapshot.gpu else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nvidia-smi unavailable",
                theme::muted(),
            ))),
            area,
        );
        return;
    };

    let rows = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);

    let g_title = format!(" {} GAUGES ", theme::GLYPH_GPU);
    let gauge_panel = panel_neutral(&g_title);
    let g_inner = gauge_panel.inner(rows[0]);
    frame.render_widget(gauge_panel, rows[0]);

    let g_rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(g_inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(&gpu.name, theme::text()),
            Span::styled(
                format!(
                    "    {:.0}W   {}°C   SM {} MHz   MEM {} MHz",
                    gpu.power_w, gpu.temp_c, gpu.sm_clock_mhz, gpu.mem_clock_mhz
                ),
                theme::muted(),
            ),
        ])),
        g_rows[0],
    );

    let vram_ratio = ratio(gpu.vram_used_mib as f64, gpu.vram_total_mib as f64);
    render_metric_row(
        frame,
        g_rows[1],
        "VRAM",
        vram_ratio,
        &format!(
            "{:.2}/{:.2} GiB {:.0}%",
            gpu.vram_used_mib as f64 / 1024.0,
            gpu.vram_total_mib as f64 / 1024.0,
            vram_ratio * 100.0
        ),
    );
    render_metric_row(
        frame,
        g_rows[2],
        "UTIL",
        gpu.util_pct as f64 / 100.0,
        &format!("{}%", gpu.util_pct),
    );
    render_metric_row(
        frame,
        g_rows[3],
        "TEMP",
        gpu.temp_c as f64 / 100.0,
        &format!("{}°C", gpu.temp_c),
    );
    render_metric_row(
        frame,
        g_rows[4],
        "SM CLK",
        gpu.sm_clock_mhz as f64 / 2000.0,
        &format!("{} MHz", gpu.sm_clock_mhz),
    );
    render_metric_row(
        frame,
        g_rows[5],
        "MEM CLK",
        gpu.mem_clock_mhz as f64 / 8000.0,
        &format!("{} MHz", gpu.mem_clock_mhz),
    );
    let power_ratio = ratio(gpu.power_w, gpu.power_limit_w);
    render_metric_row(
        frame,
        g_rows[6],
        "POWER",
        power_ratio,
        &format!("{:.0}W / {:.0}W", gpu.power_w, gpu.power_limit_w),
    );

    let proc_panel = panel_neutral(" PROCESSES ");
    let p_inner = proc_panel.inner(rows[1]);
    frame.render_widget(proc_panel, rows[1]);

    let mut lines = vec![Line::from(vec![
        Span::styled("  PID      ", theme::muted()),
        Span::styled(format!("{:<24}", "NAME"), theme::muted()),
        Span::styled("VRAM", theme::muted()),
    ])];
    lines.extend(gpu.processes.iter().map(|p| {
        Line::from(vec![
            Span::styled(format!("  {:<8} ", p.pid), theme::text()),
            Span::styled(format!("{:<24}", short_name(&p.name)), theme::text()),
            Span::styled(
                format!("{:.1} GiB", p.vram_mib as f64 / 1024.0),
                theme::live(),
            ),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), p_inner);
}

/// A GPU metric row: label | gauge | value.
fn render_metric_row(frame: &mut Frame, area: Rect, label: &str, r: f64, value: &str) {
    let cols = Layout::horizontal([
        Constraint::Length(8),
        Constraint::Min(0),
        Constraint::Length(18),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label, theme::muted()))),
        cols[0],
    );
    let fill_style = if r > 0.8 {
        theme::gauge_fill_warn()
    } else {
        theme::gauge_fill()
    };
    frame.render_widget(
        Gauge::default()
            .ratio(r.clamp(0.0, 1.0))
            .label("")
            .gauge_style(fill_style),
        cols[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(value, theme::live()))),
        cols[2],
    );
}

/// Live log tail for the selected service.
fn render_logs_tab(frame: &mut Frame, area: Rect, app: &App) {
    let title = format!(" {} LOG ", app.log_source.label());
    let up = match app.log_source {
        LogSource::Gen => app.snapshot.gen.up,
        LogSource::Hermetis => app.snapshot.hermetis.up,
        LogSource::Embed => app.snapshot.embed.up,
    };
    let block = panel(&title, up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = app.current_log_lines();
    // Show the newest lines that fit; the tail is newest-last.
    let start = lines.len().saturating_sub(inner.height as usize);
    let shown: Vec<Line> = lines[start..]
        .iter()
        .map(|l| Line::raw(strip_ansi(l)))
        .collect();
    frame.render_widget(Paragraph::new(shown), inner);
}

/// Strip ANSI escape sequences from a log line for clean display.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            skip_escape(&mut chars);
        } else {
            out.push(c);
        }
    }
    out
}

/// Consume the remainder of a CSI/OSC escape sequence from the iterator.
fn skip_escape(chars: &mut std::str::Chars<'_>) {
    for n in chars {
        if n == '\u{7}' || n == '~' || n.is_ascii_alphabetic() {
            break;
        }
    }
}

/// A ratio safe against a zero denominator.
fn ratio(num: f64, den: f64) -> f64 {
    if den == 0.0 {
        0.0
    } else {
        num / den
    }
}

/// CONTROLS tab: action list on the left, streaming output on the right.
fn render_controls_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let cols =
        Layout::horizontal([Constraint::Percentage(38), Constraint::Percentage(62)]).split(area);

    render_action_list(frame, cols[0], app);
    render_control_log(frame, cols[1], app);
}

/// Render the CONTROLS action list with the cursor and running state.
fn render_action_list(frame: &mut Frame, area: Rect, app: &App) {
    let list_panel = panel_neutral(" ACTIONS ");
    let list_inner = list_panel.inner(area);
    frame.render_widget(list_panel, area);

    let actions = app.actions();
    let mut lines: Vec<Line> = Vec::with_capacity(actions.len());
    for (i, action) in actions.iter().enumerate() {
        let running = app.control_running && app.control_action == action.label();
        let glyph = action_glyph(running, i == app.selected_action);
        lines.push(Line::from(Span::styled(
            format!(" {glyph} {}", action_label(action, &app.profiles)),
            action_style(running, i == app.selected_action),
        )));
    }
    frame.render_widget(Paragraph::new(lines), list_inner);
}

/// Cursor glyph for an action-list row.
fn action_glyph(running: bool, selected: bool) -> &'static str {
    if running {
        "◐"
    } else if selected {
        "▸"
    } else {
        " "
    }
}

/// Style for an action-list row.
fn action_style(running: bool, selected: bool) -> Style {
    if running {
        theme::live()
    } else if selected {
        theme::title()
    } else {
        theme::muted()
    }
}

/// Render the streaming control-output log.
fn render_control_log(frame: &mut Frame, area: Rect, app: &App) {
    let log_panel = panel_neutral(" CONTROL LOG ");
    let log_inner = log_panel.inner(area);
    frame.render_widget(log_panel, area);

    let mut log_lines: Vec<Line> = Vec::new();
    if app.control_running {
        log_lines.push(Line::from(vec![
            Span::styled("◐ ", theme::live()),
            Span::styled(
                &app.control_action,
                theme::live().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  ▸ {}", app.control_step), theme::muted()),
        ]));
    }
    let start = app
        .control_log
        .len()
        .saturating_sub(log_inner.height as usize);
    log_lines.extend(
        app.control_log
            .iter()
            .skip(start)
            .map(|l| Line::raw(strip_ansi(l))),
    );
    frame.render_widget(Paragraph::new(log_lines), log_inner);
}

/// CONTROLS action label, appending the profile description and source tag
/// for profile-load entries.
fn action_label(action: &Action, profiles: &[crate::profile::Profile]) -> String {
    let Action::LoadProfile(name) = action else {
        return action.label();
    };
    let Some(profile) = profiles.iter().find(|p| p.name == *name) else {
        return action.label();
    };
    let mut label = action.label();
    if !profile.description.is_empty() {
        label.push_str("  — ");
        label.push_str(&profile.description);
    }
    let src = match profile.source {
        crate::profile::ProfileSource::Bundled => "bundled",
        crate::profile::ProfileSource::Installed => "installed",
    };
    label.push_str(&format!("  [{src}]"));
    label
}

/// HERMETIS tab: stats + recent stores on the left, search on the right.
fn render_hermetis_tab(frame: &mut Frame, area: Rect, app: &App) {
    let cols =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);

    let rows = Layout::vertical([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)]).split(cols[0]);
    render_hermetis_stats(frame, rows[0], app);
    render_hermetis_recent(frame, rows[1], app);
    render_hermetis_search(frame, cols[1], app);
}

/// Stats panel: service status + episode/node/session counts.
fn render_hermetis_stats(frame: &mut Frame, area: Rect, app: &App) {
    let h = &app.snapshot.hermetis;
    let title = format!(" {} STATS ", theme::GLYPH_HERM);
    let block = panel(&title, h.up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        status_line(h.up, "memory server"),
        count_line("episodes", h.episodes),
        count_line("nodes", h.nodes),
        count_line("sessions", h.sessions),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Recent stores panel: the latest stored episodes.
fn render_hermetis_recent(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_neutral(" RECENT STORES ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let recent = &app.snapshot.hermetis.recent;
    if recent.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no episodes yet", theme::muted()))),
            inner,
        );
        return;
    }
    let lines: Vec<Line> = recent
        .iter()
        .map(|r| {
            Line::from(vec![
                Span::styled(format!("#{:<4}", r.id), theme::muted()),
                Span::styled(format!("[{:<7}] ", r.role), theme::muted()),
                Span::styled(&r.snippet, theme::text()),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Search panel: query input + result list.
fn render_hermetis_search(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_neutral(" SEARCH ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).split(inner);

    let query_style = if app.search_in_flight {
        theme::live()
    } else {
        theme::text()
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("query  ", theme::muted()),
            Span::styled(
                format!(
                    "{}{}",
                    app.search_query,
                    if app.search_in_flight { "" } else { "▌" }
                ),
                query_style,
            ),
        ])),
        rows[0],
    );

    let mut lines: Vec<Line> = Vec::new();
    if app.search_results.is_empty() && !app.search_in_flight {
        lines.push(Line::from(Span::styled(
            "type a query, Enter to search",
            theme::muted(),
        )));
    }
    for hit in &app.search_results {
        let kind = if hit.kind == "node" {
            "node"
        } else {
            "episode"
        };
        lines.push(Line::from(vec![Span::styled(
            format!("[{:.2}] {kind}", hit.score),
            theme::live(),
        )]));
        if !hit.source.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("  ↳ {}", hit.source),
                theme::muted(),
            )));
        }
        lines.push(Line::from(Span::styled(hit.content.clone(), theme::text())));
    }
    frame.render_widget(Paragraph::new(lines), rows[1]);
}

/// A status line: colored dot + word + service description.
fn status_line(up: bool, what: &str) -> Line<'static> {
    let (dot, word, color) = if up {
        ("●", "running", theme::GREEN)
    } else {
        ("○", "down", theme::RED)
    };
    Line::from(vec![
        Span::styled(
            format!("{dot} {word} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("— {what}"), theme::muted()),
    ])
}

/// A label/value line.
fn kv_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<9}", label = label), theme::muted()),
        Span::styled(value.to_string(), theme::text()),
    ])
}

/// A label/count line.
fn count_line(label: &str, count: Option<u64>) -> Line<'static> {
    let value = count.map(|c| c.to_string()).unwrap_or_else(|| "—".into());
    kv_line(label, &value)
}

/// Trim a process path to its basename, falling back to the raw string.
fn short_name(name: &str) -> String {
    name.rsplit('/')
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| name.to_string())
}

/// SUBSYSTEMS tab: one row per Tria Prima service + alchemical layer, glyph,
/// liveness dot, live value, and the config keys that drive it (read-only).
fn render_subsystems_tab(frame: &mut Frame, area: Rect, app: &App) {
    let rows = subsystems::rows(&app.cfg, &app.snapshot);
    let title = format!(" {} ALCHEMICAL LAYERS ", theme::GLYPH_GPU);
    let block = panel_neutral(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut inner_rows = Vec::new();
    for _ in &rows {
        inner_rows.push(Constraint::Length(2));
    }
    inner_rows.push(Constraint::Min(0));
    let lines = Layout::vertical(inner_rows).split(inner);

    for (i, row) in rows.iter().enumerate() {
        let dot = match row.status {
            subsystems::Status::Up => Span::styled("●", theme::live()),
            subsystems::Status::Down => Span::styled("●", Style::default().fg(theme::RED)),
            subsystems::Status::Unknown => Span::styled("◌", theme::muted()),
        };
        let group = if row.group == subsystems::GROUP_SERVICES {
            theme::title()
        } else {
            theme::gold_muted()
        };
        let line = Line::from(vec![
            dot,
            Span::styled(format!(" {}", row.glyph), group),
            Span::styled(format!(" {:<10}", row.name), theme::title()),
            Span::styled(format!(" {}", row.value), theme::text()),
            Span::styled("   … ", theme::muted()),
            Span::styled(row.config.join(" · "), theme::muted()),
        ]);
        frame.render_widget(Paragraph::new(line), lines[i]);
    }
}

/// PROFILES tab: editable active-config rows. Enter edits inline, d removes,
/// r reloads from disk; the edit buffer is shown in place of the value.
fn render_profiles_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" ACTIVE CONFIG ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let n = app.config_file.entries.len();
    if n == 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no config entries (empty ~/.vitriol/config)",
                theme::muted(),
            ))),
            inner,
        );
        return;
    }

    let mut inner_rows = Vec::with_capacity(n);
    for _ in 0..n {
        inner_rows.push(Constraint::Length(1));
    }
    inner_rows.push(Constraint::Min(0));
    let lines = Layout::vertical(inner_rows).split(inner);

    for (i, e) in app.config_file.entries.iter().enumerate() {
        let row = profile_row(app, e, i);
        frame.render_widget(Paragraph::new(row), lines[i]);
    }
}

/// Compute one config row line (key span + value/buffer span) for index `i`.
fn profile_row(app: &App, e: &crate::config_edit::Entry, i: usize) -> Line<'static> {
    let selected = i == app.profile_selection;
    let key = full_key(e);
    let editing = selected && app.profile_edit.is_some();
    let value = if editing {
        app.profile_edit.clone().unwrap_or_default()
    } else {
        e.value.clone()
    };
    Line::from(vec![
        key_span(&key, selected),
        value_span(&value, selected, editing),
        if editing {
            Span::styled(" █", theme::live())
        } else {
            Span::styled("", theme::text())
        },
    ])
}

/// The `section.key` display form of an entry.
fn full_key(e: &crate::config_edit::Entry) -> String {
    if e.section.is_empty() {
        format!("{}:", e.key)
    } else {
        format!("{}.{}", e.section, e.key)
    }
}

/// Style + pad the key span, reversed when selected.
fn key_span(key: &str, selected: bool) -> Span<'static> {
    let style = if selected {
        theme::title().add_modifier(Modifier::REVERSED)
    } else {
        theme::muted()
    };
    Span::styled(format!("{key:<24}"), style)
}

/// Style the value span: live while editing, selected style when cursor is on
/// the row, plain text otherwise.
fn value_span(value: &str, selected: bool, editing: bool) -> Span<'static> {
    let style = if editing {
        theme::live()
    } else if selected {
        theme::title().add_modifier(Modifier::REVERSED)
    } else {
        theme::text()
    };
    Span::styled(value.to_string(), style)
}

/// GUIDE tab: doc index on the left, scrolled rendered markdown on the right.
fn render_guide_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let cols = Layout::horizontal([Constraint::Percentage(35), Constraint::Min(0)]).split(area);

    render_guide_index(frame, cols[0], app);
    render_guide_reader(frame, cols[1], app);
}

/// Left pane: the discoverable doc index.
fn render_guide_index(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_neutral(" INDEX ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.guide_docs.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no docs", theme::muted()))),
            inner,
        );
        return;
    }

    let mut rows = Vec::with_capacity(app.guide_docs.len());
    for _ in 0..app.guide_docs.len() {
        rows.push(Constraint::Length(1));
    }
    rows.push(Constraint::Min(0));
    let lines = Layout::vertical(rows).split(inner);

    for (i, doc) in app.guide_docs.iter().enumerate() {
        let selected = i == app.guide_selection;
        let style = if selected {
            theme::title().add_modifier(Modifier::REVERSED)
        } else {
            theme::text()
        };
        let kind = Span::styled(format!("{:>10} ", doc.kind.label()), theme::muted());
        let title = Span::styled(doc.title.clone(), style);
        frame.render_widget(Paragraph::new(Line::from(vec![kind, title])), lines[i]);
    }
}

/// Right pane: the selected doc's body, scrolled. The provenance footer line is
/// shown above the body when the doc carries one.
fn render_guide_reader(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" DOCUMENT ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let body = app.guide_body();
    if body.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "select a doc on the left",
                theme::muted(),
            ))),
            inner,
        );
        return;
    }

    let provenance = app
        .guide_docs
        .get(app.guide_selection)
        .and_then(|d| d.provenance.clone());

    let n = body.len();
    let height = inner.height as usize;
    let max_scroll = n.saturating_sub(height);
    if app.guide_scroll > max_scroll {
        app.guide_scroll = max_scroll;
    }

    let mut lines: Vec<Line> = body
        .iter()
        .skip(app.guide_scroll)
        .map(|l| {
            let style = if l.starts_with('#') {
                theme::title()
            } else {
                theme::text()
            };
            Line::from(Span::styled(l.clone(), style))
        })
        .collect();
    if let Some(p) = provenance {
        lines.push(Line::from(Span::styled(
            format!("PROVENANCE: {p}"),
            theme::gold_muted(),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).scroll((app.guide_scroll as u16, 0)),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::short_name;

    #[test]
    fn basename_of_path() {
        assert_eq!(short_name("/usr/bin/llama-server"), "llama-server");
        assert_eq!(short_name("python3"), "python3");
        assert_eq!(short_name(""), "");
    }
}
