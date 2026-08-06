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

use crate::app::App;
use crate::model::Snapshot;
use crate::theme;

/// Draw the whole UI for the current app state.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    render_header(frame, rows[0], &app.cfg.project_id);
    render_dashboard(frame, rows[1], app);
    render_footer(frame, rows[2], app);
}

/// Top banner: gold VITRIOL title + the active tab + project id.
fn render_header(frame: &mut Frame, area: Rect, project_id: &str) {
    let line = Line::from(vec![
        Span::styled(" VITRIOL ", theme::banner()),
        Span::styled(" ▸ DASHBOARD ", theme::title()),
        Span::styled(format!("   {project_id}"), theme::muted()),
    ]);
    let para = Paragraph::new(line);
    frame.render_widget(para, area);
}

/// Bottom keybinding bar.
fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled(" [q] quit ", theme::muted()),
        Span::styled("[r] refresh", theme::muted()),
        Span::styled("  ·  vitriol-tui v0.1.0", theme::muted()),
    ];
    if app.snapshot.is_empty() {
        spans.push(Span::styled(
            "  ·  stack unreachable — nothing on :8279/:8090/:8081",
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
