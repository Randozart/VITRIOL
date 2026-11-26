//! Render the operational dashboard and its log viewer.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, LogSource, Menu, Tab};
use crate::{braille, theme};

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::new().bg(theme::BG())), area);
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    render_header(frame, rows[0], app);
    match app.tab {
        Tab::Dashboard => render_dashboard(frame, rows[1], app),
        Tab::Logs => render_logs_tab(frame, rows[1], app),
    }
    render_footer(frame, rows[2], app);
    if let Some(menu) = &app.menu {
        render_menu(frame, area, menu);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &mut App) {
    app.tab_hits.clear();
    let tab_labels: Vec<(Tab, String)> = Tab::ALL
        .iter()
        .map(|t| (*t, format!(" {} ", t.label())))
        .collect();
    let mut constraints = vec![Constraint::Length((" VITRIOL ".len()) as u16)];
    for (_, label) in &tab_labels {
        constraints.push(Constraint::Length(label.chars().count() as u16));
    }
    constraints.push(Constraint::Min(0));
    let segs = Layout::horizontal(constraints).split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(" VITRIOL ", theme::banner())])),
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
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let keys = match app.tab {
        Tab::Dashboard => "[a] actions  [Tab] logs  [r] refresh  [q] quit",
        Tab::Logs => "[1–4] source  [Space] pause  [/] search  [v] detail  [Tab] dashboard",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {keys}"), theme::muted()),
            Span::styled(
                format!("   [T] {}", theme::current().name),
                theme::gold_muted(),
            ),
        ])),
        area,
    );
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(theme::BORDER_DIM()))
        .style(Style::new().bg(theme::PANEL()))
        .title(Span::styled(title, theme::gold_muted()))
}

fn render_dashboard(frame: &mut Frame, area: Rect, app: &App) {
    let height = 9 + app.snapshot.gen.slots.len().min(4) as u16;
    let rows = Layout::vertical([
        Constraint::Length(height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    let cards = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .spacing(1)
        .split(rows[0]);
    render_engine(frame, cards[0], app);
    render_decode_card(frame, cards[1], app);
    render_gpu(frame, rows[1], app);
    let status = if app.action_running {
        Some((true, "Working…"))
    } else {
        app.action_status
            .as_ref()
            .map(|(ok, text)| (*ok, text.as_str()))
    };
    if let Some((ok, text)) = status {
        let prefix = if ok { "" } else { "Action failed · " };
        let style = if ok { theme::info() } else { theme::warn() };
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {prefix}{text}"), style)),
            rows[2],
        );
    }
}

fn datum(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<11}"), theme::muted()),
        Span::styled(value.into(), theme::text()),
    ])
}

fn render_engine(frame: &mut Frame, area: Rect, app: &App) {
    let snapshot = &app.snapshot;
    let engine = &snapshot.gen;
    let block = panel(" Engine ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let port = engine
        .effective_port
        .map_or_else(|| "—".into(), |port| format!(":{port}"));
    let api = if engine.up { "ready" } else { "unreachable" };
    let service = if snapshot.service_state.is_empty() {
        "checking"
    } else {
        &snapshot.service_state
    };
    let mut lines = vec![
        datum("Service", format!("{service} · API {api} {port}")),
        datum(
            "Running",
            engine
                .model_path
                .as_deref()
                .or(engine.model.as_deref())
                .unwrap_or("—"),
        ),
        datum(
            "Selected",
            snapshot.configured_profile.as_deref().unwrap_or("—"),
        ),
    ];
    if snapshot.mismatch {
        lines.push(Line::from(Span::styled(
            "Selected config targets a different model · restart to apply",
            theme::warn(),
        )));
    } else if let Some(alias) = engine
        .model
        .as_deref()
        .filter(|alias| Some(*alias) != engine.model_path.as_deref())
    {
        lines.push(datum("Alias", alias));
    }
    let gpu = snapshot
        .gpu_processes
        .iter()
        .filter(|process| Some(process.pid) == engine.model_pid)
        .filter_map(|process| {
            Some(format!(
                "GPU{} · {:.2} GiB",
                process.gpu_index?,
                process.vram_mib as f64 / 1024.0
            ))
        })
        .collect::<Vec<_>>();
    if !gpu.is_empty() {
        lines.push(datum("Placement", gpu.join(" / ")));
    }
    if let Some(acceptance) = engine.draft.and_then(|draft| draft.acceptance_rate()) {
        lines.push(datum("Draft acc.", format!("{:.0}%", 100.0 * acceptance)));
    }
    if engine.up {
        for slot in engine.slots.iter().take(4) {
            let filled = slot
                .filled
                .map_or_else(|| "—".into(), |value| value.to_string());
            let window = slot
                .n_ctx
                .or(engine.n_ctx)
                .map_or_else(|| "—".into(), |value| value.to_string());
            let activity = if slot.is_processing { "busy" } else { "idle" };
            lines.push(datum(
                &format!("Slot {}", slot.id),
                format!("{filled}/{window} · {activity}"),
            ));
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_gpu(frame: &mut Frame, area: Rect, app: &App) {
    let gpus = &app.snapshot.gpus;
    if gpus.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(" GPU telemetry unavailable", theme::muted())),
            area,
        );
        return;
    }
    let split = if area.width >= 72 && (1..=2).contains(&gpus.len()) {
        let cards = Layout::vertical([Constraint::Length(8), Constraint::Min(0)]).split(area);
        Layout::horizontal(vec![Constraint::Ratio(1, gpus.len() as u32); gpus.len()])
            .spacing(1)
            .split(cards[0])
    } else {
        Layout::vertical(vec![Constraint::Length(8); gpus.len()]).split(area)
    };
    for (gpu, bounds) in gpus.iter().zip(split.iter().copied()) {
        let name = gpu
            .name
            .trim_start_matches("NVIDIA GeForce ")
            .trim_start_matches("NVIDIA ");
        let title = format!(" GPU{} · {} ", gpu.index, name);
        let block = panel(&title);
        let inner = block.inner(bounds);
        frame.render_widget(block, bounds);
        let rows = Layout::vertical([Constraint::Length(1); 6]).split(inner);
        let used = gpu.vram_used_mib as f64;
        let total = gpu.vram_total_mib as f64;
        if total > 0.0 {
            render_metric_row(
                frame,
                rows[0],
                MetricRow {
                    label: "VRAM",
                    ratio: ratio(used, total),
                    value: format!("{:.1}/{:.1} GiB", used / 1024.0, total / 1024.0),
                    ramp: theme::BrailleRamp::Capacity,
                },
            );
        }
        render_metric_row(
            frame,
            rows[1],
            MetricRow {
                label: "Util",
                ratio: gpu.util_pct as f64 / 100.0,
                value: format!("{}%", gpu.util_pct),
                ramp: theme::BrailleRamp::Activity,
            },
        );
        if gpu.temp_c > 0 {
            render_metric_row(
                frame,
                rows[2],
                MetricRow {
                    label: "Temp",
                    ratio: gpu.temp_c as f64 / 100.0,
                    value: format!("{} °C", gpu.temp_c),
                    ramp: theme::BrailleRamp::Heat,
                },
            );
        }
        if gpu.power_w > 0.0 {
            render_metric_row(
                frame,
                rows[3],
                MetricRow {
                    label: "Power",
                    ratio: ratio(gpu.power_w, gpu.power_limit_w),
                    value: format!("{:.0} W", gpu.power_w),
                    ramp: theme::BrailleRamp::Power,
                },
            );
        }
        if gpu.sm_clock_mhz > 0 {
            render_metric_row(
                frame,
                rows[4],
                MetricRow {
                    label: "Clocks",
                    ratio: gpu.sm_clock_mhz as f64 / 2000.0,
                    value: format!("{}/{} MHz", gpu.sm_clock_mhz, gpu.mem_clock_mhz),
                    ramp: theme::BrailleRamp::Pulse,
                },
            );
        }
        let processes = app
            .snapshot
            .gpu_processes
            .iter()
            .filter(|process| process.gpu_index == Some(gpu.index))
            .map(|process| {
                format!(
                    "{} {} MiB",
                    process.name.rsplit('/').next().unwrap_or(&process.name),
                    process.vram_mib
                )
            })
            .take(3)
            .collect::<Vec<_>>()
            .join(" · ");
        frame.render_widget(
            Paragraph::new(Span::styled(processes, theme::muted())),
            rows[5],
        );
    }
}
fn render_braille_bar(frame: &mut Frame, area: Rect, ratio: f64, ramp: theme::BrailleRamp) {
    let bar = braille::bar(ratio, area.width as usize);
    let spans: Vec<Span> = bar
        .iter()
        .map(|c| {
            let style = if c.lit {
                Style::new().fg(ramp.color(c.t)).bg(theme::PANEL())
            } else {
                Style::new().fg(theme::current().faint).bg(theme::PANEL())
            };
            Span::styled(c.ch.to_string(), style)
        })
        .collect();
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_decode_card(frame: &mut Frame, area: Rect, app: &App) {
    let block = panel(" Decode ");
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

    let mut summary = vec![datum(
        "Slots",
        app.snapshot
            .gen
            .n_parallel
            .map_or_else(|| "—".into(), |n| format!("{}/{n} active", active.len())),
    )];
    let engine = &app.snapshot.gen;
    if engine.decode_t_s > 0.0 {
        summary.push(datum(
            "Last decode",
            format!("{:.1} t/s", engine.decode_t_s),
        ));
    }
    let totals = engine.totals;
    let seen = totals.prompt_tokens_total + totals.prompt_tokens_cached_total;
    if seen > 0 {
        summary.push(datum(
            "Cache hit",
            format!(
                "{:.0}% · {} generated",
                100.0 * totals.prompt_tokens_cached_total as f64 / seen as f64,
                totals.tokens_predicted_total
            ),
        ));
    }
    if let Some(perf) = &engine.perf {
        summary.push(datum(
            "Step",
            format!("{:.1} ms · {:.1} compute", perf.total_ms, perf.compute_ms),
        ));
    }
    if speed <= 0.0 && active.is_empty() {
        summary.push(Line::from(Span::styled(
            if engine.up {
                "Idle"
            } else {
                "Waiting for engine"
            },
            theme::muted(),
        )));
        frame.render_widget(Paragraph::new(summary), inner);
        return;
    }

    // One row per active slot, then the velocity line + gauge.
    let mut constraints = Vec::with_capacity(active.len() + 2);
    constraints.extend(vec![Constraint::Length(1); active.len()]);
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
/// the token counts. 2026-09-01: slots still in prompt-eval (processing but
/// zero tokens decoded) show a phase label instead of a meaningless 0% bar.
fn render_slot_progress_row(frame: &mut Frame, area: Rect, slot: &crate::model::SlotSnapshot) {
    let cols = Layout::horizontal([
        Constraint::Length(12),
        Constraint::Min(0),
        Constraint::Length(14),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("task {:<7}", slot.id_task.unwrap_or(slot.id).to_string()),
            theme::muted(),
        ))),
        cols[0],
    );
    if slot.is_prompt_eval() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("prompt-eval …", theme::info()))),
            cols[1],
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("— tok", theme::muted()))),
            cols[2],
        );
        return;
    }
    let total = slot.total_tokens();
    let progress = slot.progress().unwrap_or(0.0);
    render_braille_bar(frame, cols[1], progress, theme::BrailleRamp::Velocity);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!("{}/{} tok", slot.n_decoded, total),
            theme::gauge_value_style(theme::BrailleRamp::Velocity, progress),
        ))),
        cols[2],
    );
}

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
        Paragraph::new(Line::from(Span::styled(
            metric.label,
            theme::gauge_label_style(metric.ramp, metric.ratio),
        ))),
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

fn render_menu(frame: &mut Frame, area: Rect, menu: &Menu) {
    let (title, index, choices) = match menu {
        Menu::Actions(index) => (
            " Actions ",
            *index,
            vec![
                "Switch profile".into(),
                "Start service".into(),
                "Restart service".into(),
                "Stop service".into(),
            ],
        ),
        Menu::Profiles(index, profiles) => (
            " Select profile ",
            *index,
            profiles
                .iter()
                .map(|p| {
                    if p.description.is_empty() {
                        p.name.clone()
                    } else {
                        format!("{}  ·  {}", p.name, p.description)
                    }
                })
                .collect(),
        ),
    };
    let width = area.width.min(72);
    let height = area.height.min((choices.len() + 3).clamp(5, 17) as u16);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = panel(title);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);
    let capacity = inner.height.saturating_sub(1) as usize;
    let start = index.saturating_add(1).saturating_sub(capacity);
    let mut lines = choices
        .iter()
        .enumerate()
        .skip(start)
        .take(capacity)
        .map(|(i, choice)| {
            let selected = i == index;
            let prefix = if selected { " › " } else { "   " };
            let style = if selected {
                theme::title().bg(theme::current().user_bg)
            } else {
                theme::text()
            };
            Line::from(Span::styled(format!("{prefix}{choice}"), style))
        })
        .collect::<Vec<_>>();
    if choices.is_empty() {
        lines.push(Line::from(Span::styled(
            "No saved profiles",
            theme::muted(),
        )));
    }
    lines.push(Line::from(Span::styled(
        " ↑↓ move   Enter select   Esc close",
        theme::muted(),
    )));
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_logs_tab(frame: &mut Frame, area: Rect, app: &mut App) {
    let block = panel(" Logs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);
    let mut chips = LogSource::ALL
        .iter()
        .enumerate()
        .map(|(i, source)| {
            let style = if *source == app.log_source {
                theme::title().bg(theme::current().user_bg)
            } else {
                theme::muted()
            };
            Span::styled(format!(" {} {} ", i + 1, source.label()), style)
        })
        .collect::<Vec<_>>();
    if app.paused_log.is_some() {
        chips.push(Span::styled("   Paused", theme::warn()));
    }
    frame.render_widget(Paragraph::new(Line::from(chips)), rows[0]);

    let query = app.log_query.to_lowercase();
    let lines = filtered_lines(&app.log_lines(), &query, app.logs_verbose);
    let capacity = rows[1].height as usize;
    app.log_offset = app.log_offset.min(lines.len().saturating_sub(capacity));
    let end = lines.len() - app.log_offset;
    let start = end.saturating_sub(capacity);
    let mut visible = lines[start..end]
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                line.as_str(),
                if line.contains("error") || line.starts_with("Journal unavailable") {
                    theme::warn()
                } else {
                    theme::text()
                },
            ))
        })
        .collect::<Vec<_>>();
    if visible.is_empty() {
        visible.push(Line::from(Span::styled(
            if query.is_empty() {
                "No entries yet"
            } else {
                "No matching entries"
            },
            theme::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(visible), rows[1]);
    let search = if app.search_open {
        format!(" Search: {}▏   Enter filter · Esc clear", app.log_query)
    } else if !query.is_empty() {
        format!(" Filter: {}   Esc clear", app.log_query)
    } else {
        format!(
            " {} · ↑↓ scroll · End latest",
            if app.logs_verbose {
                "Detail"
            } else {
                "Concise"
            }
        )
    };
    frame.render_widget(
        Paragraph::new(Span::styled(search, theme::muted())),
        rows[2],
    );
}

fn filtered_lines(lines: &[String], query: &str, verbose: bool) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| {
            if !verbose
                && query.is_empty()
                && (line.contains("decode heartbeat") || line.contains("\"meta_only\""))
            {
                return None;
            }
            let line = strip_ansi(line);
            if line.to_lowercase().contains(query) {
                Some(line)
            } else {
                None
            }
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::model::{DraftSnapshot, GpuProcess, GpuSnapshot, SlotSnapshot};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn aether_paints_panel_and_full_track_and_shows_live_engine_identity() {
        let previous = theme::current().name;
        theme::set("aether").unwrap();
        let mut app = App::new(Config::from_env());
        app.snapshot.configured_profile = Some("kintsugi-gpu1".into());
        app.snapshot.service_state = "active".into();
        app.snapshot.gen.up = true;
        app.snapshot.gen.model = Some("Lapis Occultus".into());
        app.snapshot.gen.model_path = Some("Kintsugi-27B-12G.gguf".into());
        app.snapshot.gen.model_pid = Some(812);
        app.snapshot.gen.draft = Some(DraftSnapshot {
            n_total: 10,
            n_accepted: 8,
        });
        app.snapshot.gen.slots.push(SlotSnapshot {
            filled: Some(4118),
            n_ctx: Some(65536),
            ..Default::default()
        });
        app.snapshot.gpus = (0..2)
            .map(|index| GpuSnapshot {
                index,
                name: "NVIDIA GeForce RTX 3060".into(),
                vram_total_mib: 12288,
                vram_used_mib: if index == 1 { 11008 } else { 1536 },
                ..Default::default()
            })
            .collect();
        app.snapshot.gpu_processes.push(GpuProcess {
            pid: 812,
            name: "vitriol-server".into(),
            vram_mib: 10960,
            gpu_index: Some(1),
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 25)).unwrap();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = buffer
            .content
            .chunks(100)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Kintsugi-27B-12G.gguf"));
        assert!(text.contains("Lapis Occultus"));
        assert!(text.contains("kintsugi-gpu1"));
        assert!(text.contains("GPU1 · 10.70 GiB"));
        assert!(text.contains("4118/65536"));
        assert!(text.contains("Draft acc. 80%"));
        assert!(buffer
            .content
            .iter()
            .all(|cell| cell.bg == theme::AETHERSHIP.bg || cell.bg == theme::AETHERSHIP.panel));
        let track = buffer
            .content
            .iter()
            .filter(|cell| cell.symbol() == "⠀")
            .collect::<Vec<_>>();
        assert!(!track.is_empty());
        assert!(track.iter().all(|cell| cell.bg == theme::AETHERSHIP.panel));
        assert!(buffer
            .content
            .iter()
            .any(|cell| cell.symbol() == "⠿" && cell.bg == theme::AETHERSHIP.panel));
        theme::set(previous).unwrap();
    }

    #[test]
    fn log_search_keeps_unicode_and_can_find_quiet_lines() {
        let lines = vec![
            format!("{}🎯", "界".repeat(157)),
            "decode heartbeat 41 tok/s".into(),
        ];
        assert_eq!(filtered_lines(&lines, "", false), [lines[0].clone()]);
        assert_eq!(
            filtered_lines(&lines, "heartbeat", false),
            [lines[1].clone()]
        );
    }
}
