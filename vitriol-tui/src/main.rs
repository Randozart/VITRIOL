//! VITRIOL terminal operations dashboard.
//!
//! A standalone Ratatui TUI for operating VITRIOL: live status, GPU telemetry,
//! log tails, and control. Themed "Vitriolum" — dark alchemical green + gold.
//! See `.opencode/plans/2026-08-07-vitriol-tui.md`.

mod app;
mod braille;
mod config;
mod config_edit;
mod control;
mod guide;
mod markdown;
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
                        if app.tab != app::Tab::Profiles
                            && (app.tab != app::Tab::Hermetis || app.search_query.is_empty()) =>
                    {
                        refresh_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    KeyCode::Tab => app.next_tab(),
                    KeyCode::BackTab => app.prev_tab(),
                    // Tab-specific keys dispatch to a handler per tab (keeps
                    // main's complexity under the Praetor gate).
                    _ => match app.tab {
                        app::Tab::Logs => handle_logs_key(&mut app, key),
                        app::Tab::Controls => handle_controls_key(&mut app, key, &ctrl_tx),
                        app::Tab::Hermetis => handle_hermetis_key(&mut app, key, &search_tx),
                        app::Tab::Profiles => handle_profiles_key(&mut app, key),
                        app::Tab::Guide => handle_guide_key(&mut app, key),
                        _ => {}
                    },
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

/// LOGS tab keys: pick which service log to tail (1/2/3).
fn handle_logs_key(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('1') => app.log_source = app::LogSource::Gen,
        KeyCode::Char('2') => app.log_source = app::LogSource::Hermetis,
        KeyCode::Char('3') => app.log_source = app::LogSource::Embed,
        _ => {}
    }
}

/// CONTROLS tab keys: navigate the action list, run, or abort.
fn handle_controls_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    ctrl_tx: &mpsc::Sender<control::Event>,
) {
    match key.code {
        KeyCode::Char('x') => app.abort_control(),
        KeyCode::Char('j') | KeyCode::Down => app.move_selection(1),
        KeyCode::Char('k') | KeyCode::Up => app.move_selection(-1),
        KeyCode::Enter => app.run_selected_action(ctrl_tx),
        _ => {}
    }
}

/// HERMETIS tab keys: search typing + run/clear.
fn handle_hermetis_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    search_tx: &mpsc::Sender<Vec<search::SearchHit>>,
) {
    match key.code {
        KeyCode::Enter => app.run_search(search_tx),
        KeyCode::Backspace => app.backspace_search(),
        KeyCode::Esc => app.clear_search(),
        KeyCode::Char(c) => app.type_search_char(c),
        _ => {}
    }
}

/// GUIDE tab keys: move the index and scroll the reader.
fn handle_guide_key(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.guide_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.guide_move(-1),
        KeyCode::Char('p') | KeyCode::PageDown => app.guide_scroll_lines(20, 40),
        KeyCode::Char('n') | KeyCode::PageUp => app.guide_scroll_lines(-20, 40),
        _ => {}
    }
}

/// PROFILES tab keys: dispatch by mode (prompt / list / config panes).
fn handle_profiles_key(app: &mut App, key: crossterm::event::KeyEvent) {
    if app.profile_prompt.is_some() {
        handle_profile_prompt(app, key);
        return;
    }
    match app.profile_focus {
        app::ProfileFocus::List => handle_profile_list(app, key),
        app::ProfileFocus::Config => handle_profile_config(app, key),
    }
}

/// PROFILES save-as name prompt keys.
fn handle_profile_prompt(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char(c) => app.profile_save_type(c),
        KeyCode::Backspace => app.profile_save_backspace(),
        KeyCode::Enter => {
            let _ = app.profile_save_commit();
        }
        KeyCode::Esc => app.profile_save_cancel(),
        _ => {}
    }
}

/// PROFILES list-pane keys: navigate, load, delete, reload.
fn handle_profile_list(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char(',') | KeyCode::Char('.') => app.profile_pane_toggle(),
        KeyCode::Char('j') | KeyCode::Down => app.profile_list_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.profile_list_move(-1),
        KeyCode::Enter | KeyCode::Char('l') => {
            let _ = app.profile_load_selected();
        }
        KeyCode::Char('d') => {
            let _ = app.profile_delete_selected();
        }
        KeyCode::Char('r') => app.profile_reload_list(),
        _ => {}
    }
}

/// PROFILES config-pane keys: the form-style entry editor.
fn handle_profile_config(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char(',') | KeyCode::Char('.') => app.profile_pane_toggle(),
        KeyCode::Char('j') | KeyCode::Down => app.profile_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.profile_move(-1),
        KeyCode::Enter => {
            if app.profile_edit.is_some() {
                let _ = app.profile_commit();
            } else {
                app.profile_edit_selected();
            }
        }
        KeyCode::Esc => app.profile_cancel_edit(),
        KeyCode::Char('s') if app.profile_edit.is_none() => app.profile_save_start(),
        KeyCode::Char('d') if app.profile_edit.is_none() => {
            let _ = app.profile_remove_selected();
        }
        KeyCode::Char('r') if app.profile_edit.is_none() => app.profile_reload(),
        KeyCode::Backspace => app.profile_backspace(),
        KeyCode::Char(c) => app.profile_type(c),
        _ => {}
    }
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
