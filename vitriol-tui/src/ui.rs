//! Ratatui rendering: the dashboard layout in the Vitriolum theme.
//!
//! V1 ships the DASHBOARD tab only. Layout: a VITRIOL banner + active tab, a
//! row of three service cards (GEN / HERMETIS / EMBED), a GPU card with
//! btop-style gauges, and a live decode-velocity braille card. All rendering is
//! snapshot-driven and never panics on a down service.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, LogSource, Tab};
use crate::control::Action;
use crate::model::Snapshot;
use crate::{braille, subsystems, theme};

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
        Tab::Officina => render_officina_tab(frame, rows[1], app),
    }
    render_footer(frame, rows[2], app);
}

/// Top banner: gold VITRIOL title + clickable tab bar + project id. Each tab
/// label is drawn into its own rect and recorded in `app.tab_hits` so a mouse
/// click switches straight to it.
fn render_header(frame: &mut Frame, area: Rect, app: &mut App) {
    app.reset_tab_hits();
    let tab_labels: Vec<(Tab, String)> = Tab::ALL.iter().map(|t| (*t, format!(" {} ", t.label()))).collect();
    let mut constraints = vec![Constraint::Length((" VITRIOL ".len()) as u16)];
    for (_, label) in &tab_labels {
        constraints.push(Constraint::Length(label.chars().count() as u16));
    }
    constraints.push(Constraint::Min(0));
    let segs = Layout::horizontal(constraints).split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " VITRIOL ",
            theme::banner(),
        )])),
        segs[0],
    );

    for ((tab, label), seg) in tab_labels.iter().zip(segs.iter().skip(1)) {
        app.tab_hits.push((*seg, *tab));
        let style = if *tab == app.tab {
            theme::title().add_modifier(Modifier::UNDERLINED)
        } else {
            theme::muted()
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(label.clone(), style)])),
            *seg,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("   {}", app.cfg.project_id),
            theme::muted(),
        )])),
        segs[segs.len() - 1],
    );
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
    // Slot occupancy from /slots, when the endpoint answers.
    let slots = &app.snapshot.gen.slots;
    if !slots.is_empty() {
        let busy = slots.iter().filter(|s| s.is_processing).count();
        spans.push(Span::styled(
            format!("  ·  slots {}/{}", busy, slots.len()),
            if busy > 0 {
                theme::gold_muted()
            } else {
                theme::muted()
            },
        ));
    }
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

    // Port hint when the poller adopted a discovered llama-server port.
    let mut lines = lines;
    if let Some(port) = g.effective_port {
        lines.push(Line::from(vec![
            Span::styled("port    ", theme::muted()),
            Span::styled(format!(":{port} (discovered)"), theme::gold_muted()),
        ]));
    }

    // Speculative-draft acceptance from /props, when the server reports it.
    if let Some(d) = &g.draft {
        let acc = d.acceptance_rate();
        let value = match acc {
            Some(rate) => format!("{:.0}% ({}k drafts)", rate * 100.0, d.n_total / 1000),
            None => "—".to_string(),
        };
        lines.push(Line::from(vec![
            Span::styled("mtp acc ", theme::muted()),
            Span::styled(value, theme::info()),
        ]));
    }

    // VITRIOL decode breakdown ([PERF] line), when the server emits it.
    if let Some(p) = &g.perf {
        lines.push(Line::from(vec![
            Span::styled("decode  ", theme::muted()),
            Span::styled(
                format!(
                    "{:.1}ms [build {:.1} | compute {:.1} | post {:.1}]",
                    p.total_ms, p.build_ms, p.compute_ms, p.post_ms
                ),
                theme::info(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("graph   ", theme::muted()),
            Span::styled(
                format!(
                    "{}C/{}R sync {}x({:.1}ms)",
                    p.n_capture, p.n_replay, p.n_sync, p.sync_ms
                ),
                if p.n_capture > p.n_replay {
                    theme::warn()
                } else {
                    theme::info()
                },
            ),
        ]));
    }

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

/// GPU card: one compact btop-style block per GPU (VRAM + UTIL gauges), then
/// the compute-process table.
fn render_gpu_card(frame: &mut Frame, area: Rect, snap: &Snapshot) {
    let up = !snap.gpus.is_empty();
    let title = format!(" {} GPU ", theme::GLYPH_GPU);
    let block = panel(&title, up);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if snap.gpus.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nvidia-smi unavailable",
                theme::muted(),
            ))),
            inner,
        );
        return;
    }

    // Three rows per GPU (header/VRAM/UTIL), then a process table filling
    // whatever remains.
    let mut constraints = Vec::new();
    for _ in &snap.gpus {
        constraints.push(Constraint::Length(3));
    }
    constraints.push(Constraint::Min(0));
    let rows = Layout::vertical(constraints).split(inner);

    for (i, gpu) in snap.gpus.iter().enumerate() {
        let g_rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(rows[i]);
        let short = short_name(&gpu.name)
            .replace("NVIDIA GeForce ", "")
            .replace("NVIDIA ", "");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("[{}] {}", gpu.index, short), theme::text()),
                Span::styled(
                    format!("  {:.0}W  {}°C", gpu.power_w, gpu.temp_c),
                    theme::muted(),
                ),
            ])),
            g_rows[0],
        );
        let vram_ratio = if gpu.vram_total_mib > 0 {
            gpu.vram_used_mib as f64 / gpu.vram_total_mib as f64
        } else {
            0.0
        };
        render_gauge_row(
            frame,
            g_rows[1],
            &format!(
                "VRAM  {:.2}/{:.2} GiB  {:.0}%",
                gpu.vram_used_mib as f64 / 1024.0,
                gpu.vram_total_mib as f64 / 1024.0,
                vram_ratio * 100.0
            ),
            vram_ratio,
            theme::BrailleRamp::Capacity,
        );
        render_gauge_row(
            frame,
            g_rows[2],
            &format!("UTIL  {}%", gpu.util_pct),
            gpu.util_pct as f64 / 100.0,
            theme::BrailleRamp::Activity,
        );
    }

    let proc_area = rows[snap.gpus.len()];
    let mut proc_lines = vec![Line::from(Span::styled("PROCESSES", theme::muted()))];
    proc_lines.extend(
        snap.gpu_processes
            .iter()
            .take(proc_area.height.saturating_sub(1) as usize)
            .map(process_line),
    );
    frame.render_widget(Paragraph::new(proc_lines), proc_area);
}

/// One compact process-table row: pid, name, GPU index, VRAM.
fn process_line(p: &crate::model::GpuProcess) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {:<6} ", p.pid), theme::muted()),
        Span::styled(format!("{:<24}", short_name(&p.name)), theme::text()),
        Span::styled(
            format!(
                "GPU{} ",
                p.gpu_index.map(|i| i.to_string()).unwrap_or_else(|| "?".into())
            ),
            theme::muted(),
        ),
        Span::styled(
            format!("{:.1} GiB", p.vram_mib as f64 / 1024.0),
            theme::live(),
        ),
    ])
}

/// One btop-style gauge row: a muted label line above a braille-dot fill bar.
fn render_gauge_row(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    ratio: f64,
    ramp: theme::BrailleRamp,
) {
    let cols =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            label,
            theme::gauge_label_style(ramp, ratio),
        ))),
        cols[0],
    );
    render_braille_bar(frame, cols[1], ratio, ramp);
}

/// A braille-dot gradient gauge filling `area` at `ratio`, colored by `ramp`.
/// Cells span the full width; empty cells render blank, lit cells get the ramp
/// color at their position fraction.
fn render_braille_bar(frame: &mut Frame, area: Rect, ratio: f64, ramp: theme::BrailleRamp) {
    let bar = braille::bar(ratio, area.width as usize);
    let spans: Vec<Span> = bar
        .iter()
        .map(|c| {
            let style = if c.lit {
                Style::new().fg(ramp.color(c.t)).bg(theme::BG)
            } else {
                theme::muted()
            };
            Span::styled(c.ch.to_string(), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// DECODE card: a braille-dot velocity gauge that lights only while a slot is
/// actively decoding, plus the live rate and session peak. Each active slot
/// also gets a per-request progress bar (`task N · decoded/total tok`). While
/// idle the card collapses to a dim status line so the panel really stops when
/// the stack does.
fn render_decode_card(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel_neutral(" DECODE ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let speed = app.snapshot.gen.decode_speed;
    let peak = app.decode_history.iter().copied().fold(0.0f64, f64::max);
    let active: Vec<&crate::model::SlotSnapshot> = app
        .snapshot
        .gen
        .slots
        .iter()
        .filter(|s| s.is_processing)
        .collect();

    if speed <= 0.0 && active.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                if peak > 0.0 {
                    format!("idle   (session peak {peak:.1} t/s)")
                } else {
                    "idle — no decode yet".to_string()
                },
                theme::muted(),
            )])),
            inner,
        );
        return;
    }

    // One row per active slot, then the velocity line + gauge.
    let mut constraints = Vec::with_capacity(active.len() + 2);
    for _ in &active {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(1));
    constraints.push(Constraint::Min(1));
    let rows = Layout::vertical(constraints).split(inner);

    for (i, slot) in active.iter().enumerate() {
        render_slot_progress_row(frame, rows[i], slot);
    }

    let vel_line = rows[active.len()];
    let bar_area = rows[active.len() + 1];

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{speed:.1} t/s   "),
                theme::live().add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("peak {peak:.1}"), theme::muted()),
        ])),
        vel_line,
    );

    // Fill and color the gauge against the session peak: slow fills red, a
    // full bar toward the peak glows green.
    let ratio = if peak > 0.0 { speed / peak } else { 0.0 };
    render_braille_bar(frame, bar_area, ratio, theme::BrailleRamp::Velocity);
}

/// One active-slot progress row: task id, braille fill of decoded/total, and
/// the token counts.
fn render_slot_progress_row(frame: &mut Frame, area: Rect, slot: &crate::model::SlotSnapshot) {
    let total = slot.n_decoded + slot.n_remain;
    let progress = slot.progress().unwrap_or(0.0);
    let cols =
        Layout::horizontal([Constraint::Length(12), Constraint::Min(0), Constraint::Length(14)])
            .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("task {:<7}", slot.id_task.unwrap_or(slot.id).to_string()),
            theme::muted(),
        ))),
        cols[0],
    );
    render_braille_bar(frame, cols[1], progress, theme::BrailleRamp::Velocity);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{}/{} tok", slot.n_decoded, total),
            theme::gauge_value_style(theme::BrailleRamp::Velocity, progress),
        ))),
        cols[2],
    );
}

/// Full btop-style GPU panel: one gauge section per GPU on top, merged process
/// table (with GPU column) below.
fn render_gpu_tab(frame: &mut Frame, area: Rect, app: &App) {
    if app.snapshot.gpus.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "nvidia-smi unavailable",
                theme::muted(),
            ))),
            area,
        );
        return;
    }

    // 8 rows per GPU section (header + 6 metrics + spacer), plus borders.
    let gauges_height = (app.snapshot.gpus.len() as u16) * 8 + 2;
    let rows = Layout::vertical([Constraint::Length(gauges_height), Constraint::Min(0)]).split(area);

    let g_title = format!(" {} GAUGES ", theme::GLYPH_GPU);
    let gauge_panel = panel_neutral(&g_title);
    let g_inner = gauge_panel.inner(rows[0]);
    frame.render_widget(gauge_panel, rows[0]);

    let mut sections = Vec::new();
    for _ in &app.snapshot.gpus {
        sections.push(Constraint::Length(8));
    }
    sections.push(Constraint::Min(0));
    let sections = Layout::vertical(sections).split(g_inner);

    for (i, gpu) in app.snapshot.gpus.iter().enumerate() {
        let g_rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(sections[i]);

        let short = short_name(&gpu.name)
            .replace("NVIDIA GeForce ", "")
            .replace("NVIDIA ", "");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!("[{}] {}", gpu.index, short), theme::title()),
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
            MetricRow {
                label: "VRAM",
                ratio: vram_ratio,
                value: format!(
                    "{:.2}/{:.2} GiB {:.0}%",
                    gpu.vram_used_mib as f64 / 1024.0,
                    gpu.vram_total_mib as f64 / 1024.0,
                    vram_ratio * 100.0
                ),
                ramp: theme::BrailleRamp::Capacity,
            },
        );
        render_metric_row(
            frame,
            g_rows[2],
            MetricRow {
                label: "UTIL",
                ratio: gpu.util_pct as f64 / 100.0,
                value: format!("{}%", gpu.util_pct),
                ramp: theme::BrailleRamp::Activity,
            },
        );
        render_metric_row(
            frame,
            g_rows[3],
            MetricRow {
                label: "TEMP",
                ratio: gpu.temp_c as f64 / 100.0,
                value: format!("{}°C", gpu.temp_c),
                ramp: theme::BrailleRamp::Heat,
            },
        );
        render_metric_row(
            frame,
            g_rows[4],
            MetricRow {
                label: "SM CLK",
                ratio: gpu.sm_clock_mhz as f64 / 2000.0,
                value: format!("{} MHz", gpu.sm_clock_mhz),
                ramp: theme::BrailleRamp::Pulse,
            },
        );
        render_metric_row(
            frame,
            g_rows[5],
            MetricRow {
                label: "MEM CLK",
                ratio: gpu.mem_clock_mhz as f64 / 8000.0,
                value: format!("{} MHz", gpu.mem_clock_mhz),
                ramp: theme::BrailleRamp::Pulse,
            },
        );
        let power_ratio = ratio(gpu.power_w, gpu.power_limit_w);
        render_metric_row(
            frame,
            g_rows[6],
            MetricRow {
                label: "POWER",
                ratio: power_ratio,
                value: format!("{:.0}W / {:.0}W", gpu.power_w, gpu.power_limit_w),
                ramp: theme::BrailleRamp::Power,
            },
        );
    }

    let proc_panel = panel_neutral(" PROCESSES ");
    let p_inner = proc_panel.inner(rows[1]);
    frame.render_widget(proc_panel, rows[1]);

    let mut lines = vec![Line::from(vec![
        Span::styled("  PID      ", theme::muted()),
        Span::styled(format!("{:<24}", "NAME"), theme::muted()),
        Span::styled(format!("{:<5}", "GPU"), theme::muted()),
        Span::styled("VRAM", theme::muted()),
    ])];
    lines.extend(app.snapshot.gpu_processes.iter().map(|p| {
        Line::from(vec![
            Span::styled(format!("  {:<8} ", p.pid), theme::text()),
            Span::styled(format!("{:<24}", short_name(&p.name)), theme::text()),
            Span::styled(
                format!(
                    "{:<5}",
                    p.gpu_index.map(|i| i.to_string()).unwrap_or_else(|| "?".into())
                ),
                theme::muted(),
            ),
            Span::styled(
                format!("{:.1} GiB", p.vram_mib as f64 / 1024.0),
                theme::live(),
            ),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), p_inner);
}

/// One GPU metric row's data: label, value text, ratio, and color ramp.
/// Bundled to keep `render_metric_row` at 5 params (AGENTS §5.3).
struct MetricRow {
    label: &'static str,
    value: String,
    ratio: f64,
    ramp: theme::BrailleRamp,
}

/// A GPU metric row: label | braille gauge | value.
fn render_metric_row(frame: &mut Frame, area: Rect, metric: MetricRow) {
    let cols = Layout::horizontal([
        Constraint::Length(8),
        Constraint::Min(0),
        Constraint::Length(18),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(metric.label, theme::muted()))),
        cols[0],
    );
    render_braille_bar(frame, cols[1], metric.ratio, metric.ramp);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            metric.value,
            theme::gauge_value_style(metric.ramp, metric.ratio),
        ))),
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
/// for profile-bearing entries (selected Start, sweep, sweep+save).
fn action_label(action: &Action, profiles: &[crate::profile::Profile]) -> String {
    let name = match action {
        Action::Start {
            selected: Some(name),
        } => Some(name.as_str()),
        Action::RunSweep(name) | Action::SweepAndSave(name) => Some(name.as_str()),
        _ => None,
    };
    let Some(name) = name else {
        return action.label();
    };
    let Some(profile) = profiles.iter().find(|p| p.name == name) else {
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
fn render_subsystems_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows = subsystems::rows(&app.cfg, &app.snapshot);
    if app.ascensus_edit.is_some() {
        let v = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(area);
        render_subsystem_rows(frame, v[0], app, &rows);
        render_ascensus_editor(frame, v[1], app);
    } else {
        render_subsystem_rows(frame, area, app, &rows);
    }
}

/// The SUBSYSTEMS row list, with the selected row highlighted.
fn render_subsystem_rows(frame: &mut Frame, area: Rect, app: &mut App, rows: &[subsystems::Row]) {
    let title = format!(" {} ALCHEMICAL LAYERS ", theme::GLYPH_GPU);
    let block = panel_neutral(&title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut inner_rows = Vec::new();
    for _ in rows {
        inner_rows.push(Constraint::Length(2));
    }
    inner_rows.push(Constraint::Min(0));
    let lines = Layout::vertical(inner_rows).split(inner);

    for (i, row) in rows.iter().enumerate() {
        let selected = i == app.subsystem_selection;
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
        let name_style = if selected {
            theme::title().add_modifier(Modifier::REVERSED)
        } else {
            theme::title()
        };
        let line = Line::from(vec![
            dot,
            Span::styled(format!(" {}", row.glyph), group),
            Span::styled(format!(" {:<10}", row.name), name_style),
            Span::styled(format!(" {}", row.value), theme::text()),
            Span::styled("   … ", theme::muted()),
            Span::styled(row.config.join(" · "), theme::muted()),
        ]);
        frame.render_widget(Paragraph::new(line), lines[i]);
    }
}

/// The ASCENSUS key/model editor form.
fn render_ascensus_editor(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" ASCENSUS SECRETS ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(edit) = &app.ascensus_edit else {
        return;
    };
    let key_style = if edit.key_field {
        theme::live()
    } else {
        theme::muted()
    };
    let model_style = if !edit.key_field {
        theme::live()
    } else {
        theme::muted()
    };
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  api_key  ", theme::title()),
            Span::styled(edit.api_key.clone(), key_style),
            if edit.key_field {
                Span::styled("█", theme::live())
            } else {
                Span::styled("", theme::text())
            },
        ])),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  model    ", theme::title()),
            Span::styled(edit.model.clone(), model_style),
            if !edit.key_field {
                Span::styled("█", theme::live())
            } else {
                Span::styled("", theme::text())
            },
        ])),
        rows[1],
    );
}

/// PROFILES tab: editable active-config rows. Enter edits inline, d removes,
/// r reloads from disk; the edit buffer is shown in place of the value.
/// PROFILES footer height: two rows of key-badge buttons.
const PROFILE_FOOTER_ROWS: u16 = 2;

fn render_profiles_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    app.reset_profile_buttons();
    if app.profile_prompt.is_some() {
        let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(area);
        render_profile_panes(frame, rows[0], app);
        let buf = app.profile_prompt.clone().unwrap_or_default();
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" profile name: {buf}█   (Enter save, Esc cancel) "),
                theme::gold_muted(),
            ))),
            rows[1],
        );
    } else {
        let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(PROFILE_FOOTER_ROWS)])
            .split(area);
        render_profile_panes(frame, rows[0], app);
        render_profile_footer(frame, rows[1], app);
    }
}

/// PROFILES footer: two rows of mouse-clickable key-badge buttons. Row 1 is
/// pane-agnostic (switch, add, duplicate, delete, reload); row 2 is profile
/// list actions (load, start, overwrite, sweep). Each button records its rect
/// into `app.profile_buttons` so clicks hit-test reliably.
fn render_profile_footer(frame: &mut Frame, area: Rect, app: &mut App) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(area);
    let list_focused = app.profile_focus == crate::app::ProfileFocus::List;
    let delete_label = if list_focused { "delete profile" } else { "remove entry" };
    render_button_row(
        frame,
        rows[0],
        app,
        &[
            ("←→", "switch pane", crate::app::ProfileAction::SwitchPane),
            ("s", "add profile", crate::app::ProfileAction::Add),
            ("c", "duplicate", crate::app::ProfileAction::Duplicate),
            ("d", delete_label, crate::app::ProfileAction::Delete),
            ("r", "reload", crate::app::ProfileAction::Reload),
        ],
    );
    render_button_row(
        frame,
        rows[1],
        app,
        &[
            ("l", "load", crate::app::ProfileAction::Load),
            ("t", "start target", crate::app::ProfileAction::Start),
            ("w", "overwrite", crate::app::ProfileAction::Overwrite),
            ("z", "sweep", crate::app::ProfileAction::Sweep),
        ],
    );
}

/// Draw one row of footer buttons, recording each hit-box.
fn render_button_row(
    frame: &mut Frame,
    area: Rect,
    app: &mut App,
    buttons: &[(&'static str, &'static str, crate::app::ProfileAction)],
) {
    let mut constraints = Vec::with_capacity(buttons.len());
    for (key, label, _) in buttons {
        constraints.push(Constraint::Length((key.chars().count() + label.chars().count() + 5) as u16));
    }
    let segs = Layout::horizontal(constraints).split(area);
    for ((key, label, action), seg) in buttons.iter().zip(segs.iter()) {
        app.profile_buttons
            .push(crate::app::ProfileButton {
                action: *action,
                area: *seg,
            });
        let line = Line::from(vec![
            Span::styled(format!("[{key}] "), theme::gold_muted()),
            Span::styled(format!("{label} "), theme::muted()),
        ]);
        frame.render_widget(Paragraph::new(line), *seg);
    }
}

/// The two profile panes: active config rows + profile list.
fn render_profile_panes(frame: &mut Frame, area: Rect, app: &mut App) {
    let cols =
        Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)]).split(area);
    render_config_pane(frame, cols[0], app);
    render_profile_pane(frame, cols[1], app);
}

/// Left pane: the active-config entry list (form-style editor).
fn render_config_pane(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.profile_focus == crate::app::ProfileFocus::Config;
    let title = if focused {
        " ACTIVE CONFIG "
    } else {
        " CONFIG "
    };
    let block = panel_neutral(title);
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
        let selected = focused && i == app.profile_selection;
        let row = profile_row(app, e, selected);
        frame.render_widget(Paragraph::new(row), lines[i]);
    }
}

/// Right pane: the profile list with load/delete targets.
fn render_profile_pane(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.profile_focus == crate::app::ProfileFocus::List;
    let block = panel_neutral(" PROFILES ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let n = app.profiles.len();
    if n == 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "no profiles  (s = save current config)",
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

    for (i, p) in app.profiles.iter().enumerate() {
        let selected = focused && i == app.profile_list_selection;
        let style = if selected {
            theme::title().add_modifier(Modifier::REVERSED)
        } else {
            theme::text()
        };
        let src = match p.source {
            crate::profile::ProfileSource::Bundled => " [bundled]",
            crate::profile::ProfileSource::Installed => "",
        };
        let mut spans = vec![Span::styled(p.name.clone(), style)];
        if app.selected_profile.as_deref() == Some(p.name.as_str()) {
            spans.push(Span::styled(" ▸ start", theme::gold_muted()));
        }
        spans.push(Span::styled(src, theme::muted()));
        if !p.description.is_empty() {
            spans.push(Span::styled(format!("  {}", p.description), theme::muted()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), lines[i]);
    }
}

/// Compute one config row line (key span + value/buffer span) for index `i`.
fn profile_row(app: &App, e: &crate::config_edit::Entry, selected: bool) -> Line<'static> {
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
        let mut spans = vec![Span::styled("⚗ ", theme::gold_muted())];
        spans.push(Span::styled(doc.title.clone(), style));
        if let Some(summary) = &doc.summary {
            spans.push(Span::styled(format!("  {summary}"), theme::muted()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), lines[i]);
    }
}

/// Right pane: the selected doc's body, scrolled. The provenance footer line is
/// shown after the body when the doc carries one.
fn render_guide_reader(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" DOCUMENT ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    app.guide_width = inner.width as usize;

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

    let mut lines: Vec<Line> = body.into_iter().skip(app.guide_scroll).collect();
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

/// OFFICINA tab: left = REPL (output + two-line ALKA-☿ prompt), right = the
/// Spagyric Journal sidebar.
fn render_officina_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let sidebar = app.officina.config.sidebar_width.min(60);
    let cols =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(sidebar as u16)]).split(area);
    render_officina_repl(frame, cols[0], app);
    render_officina_journal(frame, cols[1], app);
}

/// Left pane: output scrollback + the prompt + completion bar.
fn render_officina_repl(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" OFFICINA ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::vertical([
        Constraint::Min(0),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .split(inner);
    let (start, end) = app.officina.output_window(rows[0].height as usize);
    let body: Vec<Line> = app
        .officina
        .output
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .map(|l| Line::from(Span::styled(l.clone(), theme::text())))
        .collect();
    frame.render_widget(Paragraph::new(body), rows[0]);

    let ctx = app.officina_ctx();
    let header = app.officina.prompt_header(&ctx);
    let mut p1 = vec![Span::styled("┌──(", theme::CYAN)];
    let logo = if app.officina.config.bold_logo {
        theme::title().add_modifier(Modifier::BOLD)
    } else {
        theme::title()
    };
    p1.push(Span::styled("☿ ALKA", logo));
    p1.push(Span::styled(")-[", theme::CYAN));
    p1.push(Span::styled(header, theme::muted()));
    p1.push(Span::styled("]", theme::CYAN));
    let prompt_line = Line::from(p1);

    let input = format!("{}█", app.officina.input);
    let p2 = Line::from(vec![
        Span::styled("└───> ", theme::CYAN),
        Span::styled(input, theme::text()),
    ]);
    let prompt = Paragraph::new(vec![prompt_line, p2]);
    frame.render_widget(prompt, rows[1]);

    let completions = app.officina.completions();
    if completions.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  Tab = autofill · PgUp/PgDn = scroll",
                theme::muted(),
            ))),
            rows[2],
        );
    } else {
        let shown: Vec<&str> = completions.iter().take(6).map(|s| s.as_str()).collect();
        let label = format!("  ⧉ {} (Tab cycles)", shown.join(" · "));
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(label, theme::gold_muted()))),
            rows[2],
        );
    }
}

/// Right pane: the Spagyric Journal (mem arenas, transformation log, cognition).
fn render_officina_journal(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel_neutral(" SPAGYRIC JOURNAL ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(" MEM ARENAS", theme::gold_muted())));
    let (used_raw, total_raw) = app.snapshot.vram_totals();
    let used = used_raw as f64 / 1024.0;
    let total = total_raw as f64 / 1024.0;
    lines.push(Line::from(Span::styled(
        format!("  VRAM: {used:.1}/{total:.1} GiB"),
        theme::muted(),
    )));
    lines.push(Line::from(Span::styled(
        format!("  decode: {:.1} t/s", app.snapshot.gen.decode_t_s),
        theme::muted(),
    )));
    lines.push(Line::from(Span::styled(
        format!("  context: {}", app.snapshot.gen.n_ctx.unwrap_or(0)),
        theme::muted(),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "  hermetis: {} ep / {} nodes",
            app.snapshot.hermetis.episodes.unwrap_or(0),
            app.snapshot.hermetis.nodes.unwrap_or(0)
        ),
        theme::muted(),
    )));

    lines.push(Line::from(Span::styled(" MASKS", theme::gold_muted())));
    let mask_names = crate::officina::mask::list(&app.cfg.home_dir);
    if mask_names.is_empty() {
        lines.push(Line::from(Span::styled("  (none)", theme::muted())));
    } else {
        for name in mask_names.iter().take(5) {
            let path = crate::officina::mask::mask_path(&app.cfg.home_dir, name);
            let pct = crate::officina::mask::MaskFile::load(&path)
                .map(|m| m.stats(64).active_fraction() * 100.0)
                .unwrap_or(0.0);
            lines.push(Line::from(Span::styled(
                format!("  {name:<24} [{pct:.0}% active]"),
                theme::text(),
            )));
        }
    }

    lines.push(Line::from(Span::styled(
        " TRANSFORMATION LOG",
        theme::gold_muted(),
    )));
    if app.officina.journal.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none committed)",
            theme::muted(),
        )));
    } else {
        for (i, entry) in app.officina.journal.iter().enumerate().rev().take(8) {
            lines.push(Line::from(Span::styled(
                format!("  [{i}] {}", entry.text),
                theme::text(),
            )));
        }
    }
    let rec = app
        .officina
        .recording
        .clone()
        .unwrap_or_else(|| "off".into());
    lines.push(Line::from(Span::styled(
        format!("  recording: {rec}"),
        theme::muted(),
    )));

    lines.push(Line::from(Span::styled(
        format!(" DRIFT: {:.4}", app.officina.drift),
        theme::gold_muted(),
    )));
    let state = if app.officina.model_dirty {
        "dirty"
    } else {
        "clean"
    };
    lines.push(Line::from(Span::styled(
        format!(" model: {state}"),
        theme::muted(),
    )));
    lines.push(Line::from(Span::styled("  (HELP in REPL)", theme::muted())));

    frame.render_widget(Paragraph::new(lines), inner);
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
