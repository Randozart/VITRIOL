//! VITRIOL terminal operations dashboard.
//!
//! A standalone Ratatui TUI for operating VITRIOL: live status, GPU telemetry,
//! log tails, and control. Themed "Vitriolum" — dark alchemical green + gold.
//! See `.opencode/plans/2026-08-07-vitriol-tui.md`.

mod app;
mod config;
mod model;
mod nvidia;
mod poller;
mod theme;
mod ui;

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::init;
use ratatui::restore;

use crate::app::App;
use crate::config::Config;

/// How often the poller wakes to refresh all telemetry.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> io::Result<()> {
    let cfg = Config::from_env();
    let (tx, rx) = mpsc::channel();
    let refresh_flag = Arc::new(AtomicBool::new(false));
    poller::spawn(cfg.clone(), tx, Arc::clone(&refresh_flag));

    let mut terminal = init();
    let mut app = App::new(cfg, 120);

    loop {
        drain_snapshots(&rx, &mut app);

        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => break,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break;
                    }
                    KeyCode::Char('r') => {
                        refresh_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    KeyCode::Tab => app.next_tab(),
                    KeyCode::BackTab => app.prev_tab(),
                    KeyCode::Char('1') => app.log_source = app::LogSource::Gen,
                    KeyCode::Char('2') => app.log_source = app::LogSource::Hermetis,
                    KeyCode::Char('3') => app.log_source = app::LogSource::Embed,
                    _ => {}
                },
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if app.should_tick(POLL_INTERVAL) {
            // Tick marker: nothing to do in V1 beyond redrawing; kept so later
            // phases can hook per-tick work (cursor blink, sweep progress).
        }
    }

    restore();
    Ok(())
}

/// Apply every snapshot the poller has published since the last draw.
fn drain_snapshots(rx: &mpsc::Receiver<model::Snapshot>, app: &mut App) {
    while let Ok(snap) = rx.try_recv() {
        app.apply_snapshot(snap);
    }
}
