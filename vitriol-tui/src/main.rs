//! VITRIOL terminal operations dashboard.
//!
//! A standalone Ratatui TUI for operating VITRIOL: live status, GPU telemetry,
//! log tails, and control. Themed "Vitriolum" — dark alchemical green + gold.
//! See `.opencode/plans/2026-08-07-vitriol-tui.md`.

mod app;
mod config;
mod config_edit;
mod control;
mod model;
mod nvidia;
mod poller;
mod profile;
mod search;
mod subsystems;
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
    let (ctrl_tx, ctrl_rx) = mpsc::channel();
    let (search_tx, search_rx) = mpsc::channel::<Vec<search::SearchHit>>();

    let mut terminal = init();
    let mut app = App::new(cfg, 120);

    loop {
        drain_snapshots(&rx, &mut app);
        drain_control(&ctrl_rx, &mut app);
        drain_search(&search_rx, &mut app);

        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if app.tab != app::Tab::Hermetis || app.search_query.is_empty() =>
                    {
                        break;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break;
                    }
                    KeyCode::Char('r')
                        if app.tab != app::Tab::Hermetis || app.search_query.is_empty() =>
                    {
                        refresh_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    KeyCode::Tab => app.next_tab(),
                    KeyCode::BackTab => app.prev_tab(),
                    KeyCode::Char('1') if app.tab == app::Tab::Logs => {
                        app.log_source = app::LogSource::Gen;
                    }
                    KeyCode::Char('2') if app.tab == app::Tab::Logs => {
                        app.log_source = app::LogSource::Hermetis;
                    }
                    KeyCode::Char('3') if app.tab == app::Tab::Logs => {
                        app.log_source = app::LogSource::Embed;
                    }
                    KeyCode::Char('x') if app.tab == app::Tab::Controls => app.abort_control(),
                    KeyCode::Char('j') | KeyCode::Down if app.tab == app::Tab::Controls => {
                        app.move_selection(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up if app.tab == app::Tab::Controls => {
                        app.move_selection(-1);
                    }
                    KeyCode::Char('j') | KeyCode::Down if app.tab == app::Tab::Profiles => {
                        app.profile_move(1);
                    }
                    KeyCode::Char('k') | KeyCode::Up if app.tab == app::Tab::Profiles => {
                        app.profile_move(-1);
                    }
                    KeyCode::Enter if app.tab == app::Tab::Profiles => {
                        if app.profile_edit.is_some() {
                            let _ = app.profile_commit();
                        } else {
                            app.profile_edit_selected();
                        }
                    }
                    KeyCode::Esc if app.tab == app::Tab::Profiles => app.profile_cancel_edit(),
                    KeyCode::Char('d') if app.tab == app::Tab::Profiles => {
                        if app.profile_edit.is_none() {
                            let _ = app.profile_remove_selected();
                        }
                    }
                    KeyCode::Char('r') if app.tab == app::Tab::Profiles => {
                        if app.profile_edit.is_none() {
                            app.profile_reload();
                        }
                    }
                    KeyCode::Backspace if app.tab == app::Tab::Profiles => {
                        app.profile_backspace();
                    }
                    KeyCode::Char(c) if app.tab == app::Tab::Profiles => app.profile_type(c),
                    KeyCode::Enter if app.tab == app::Tab::Controls => {
                        app.run_selected_action(&ctrl_tx);
                    }
                    KeyCode::Enter if app.tab == app::Tab::Hermetis => {
                        app.run_search(&search_tx);
                    }
                    KeyCode::Backspace if app.tab == app::Tab::Hermetis => {
                        app.backspace_search();
                    }
                    KeyCode::Esc if app.tab == app::Tab::Hermetis => app.clear_search(),
                    KeyCode::Char(c) if app.tab == app::Tab::Hermetis => {
                        app.type_search_char(c);
                    }
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

/// Apply every control-thread event published since the last draw.
fn drain_control(rx: &mpsc::Receiver<control::Event>, app: &mut App) {
    while let Ok(event) = rx.try_recv() {
        app.apply_control_event(event);
    }
}

/// Apply every search-result batch published since the last draw.
fn drain_search(rx: &mpsc::Receiver<Vec<search::SearchHit>>, app: &mut App) {
    while let Ok(results) = rx.try_recv() {
        app.apply_search_results(results);
    }
}
