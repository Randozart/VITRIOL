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
mod officina;
mod poller;
mod profile;
mod search;
mod secrets;
mod subsystems;
mod theme;
mod ui;

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
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
    crossterm::execute!(io::stdout(), event::EnableMouseCapture)?;
    let mut app = App::new(cfg, 120);

    loop {
        drain_snapshots(&rx, &mut app);
        drain_control(&ctrl_rx, &mut app);
        drain_search(&search_rx, &mut app);

        terminal.draw(|frame| ui::draw(frame, &mut app))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                    // SWEEP tab consumes navigation/typing keys itself.
                    Event::Key(key)
                        if key.kind == KeyEventKind::Press
                            && app.tab == app::Tab::Sweep
                            && !app.control_running
                            && key.code != KeyCode::Tab
                            && key.code != KeyCode::BackTab
                    => match key.code {
                            KeyCode::Up => app.sweep.focus_up(),
                            KeyCode::Down => app.sweep.focus_down(),
                            KeyCode::Left => app.sweep.adjust(-1),
                            KeyCode::Right => app.sweep.adjust(1),
                            KeyCode::Enter => {
                                let action = app.sweep_action();
                                app.run_action(action, &ctrl_tx);
                            }
                            KeyCode::Backspace => app.sweep.backspace(),
                            KeyCode::Char(c) => {
                                if c == 'q' { break; }
                                app.sweep.type_char(c);
                            }
                            _ => {}
                        }
                    Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q')
                        if app.tab != app::Tab::Officina
                            && (app.tab != app::Tab::Hermetis || app.search_query.is_empty()) =>
                    {
                        break;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break;
                    }
                    KeyCode::Char('r')
                        if app.tab != app::Tab::Officina
                            && app.tab != app::Tab::Profiles
                            && (app.tab != app::Tab::Hermetis || app.search_query.is_empty()) =>
                    {
                        refresh_flag.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    KeyCode::Tab if app.tab != app::Tab::Officina => app.next_tab(),
                    KeyCode::BackTab => app.prev_tab(),
                    // Shift+arrows cycle tabs so plain arrows stay free for
                    // in-screen navigation.
                    KeyCode::Right if key.modifiers.contains(KeyModifiers::SHIFT) => app.next_tab(),
                    KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => app.prev_tab(),
                    // Tab-specific keys dispatch to a handler per tab (keeps
                    // main's complexity under the Praetor gate).
                    _ => match app.tab {
                        app::Tab::Logs => handle_logs_key(&mut app, key),
                        app::Tab::Controls => handle_controls_key(&mut app, key, &ctrl_tx),
                        app::Tab::Hermetis => handle_hermetis_key(&mut app, key, &search_tx),
                        app::Tab::Profiles => handle_profiles_key(&mut app, key, &ctrl_tx),
                        app::Tab::Guide => handle_guide_key(&mut app, key),
                        app::Tab::Subsystems => handle_subsystems_key(&mut app, key),
                        app::Tab::Officina => handle_officina_key(&mut app, key),
                        _ => {}
                    },
                },
                Event::Resize(_, _) => {}
                Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                    handle_mouse(&mut app, mouse, &ctrl_tx);
                }
                _ => {}
            }
        }

        if app.should_tick(POLL_INTERVAL) {
            // Tick marker: nothing to do in V1 beyond redrawing; kept so later
            // phases can hook per-tick work (cursor blink, sweep progress).
        }
    }

    crossterm::execute!(io::stdout(), event::DisableMouseCapture)?;
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

/// OFFICINA tab keys: type, edit, run on Enter, history via arrows, Tab
/// autofills, PgUp/PgDn scroll the output.
fn handle_officina_key(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Enter => {
            let line = std::mem::take(&mut app.officina.input);
            if line.trim().is_empty() {
                return;
            }
            app.officina.output.push_back(format!("▶ {line}"));
            let cfg = &app.cfg;
            let snap = &app.snapshot;
            let model_path = app
                .config_file
                .entries
                .iter()
                .find(|e| e.section == "model" && e.key == "path")
                .map(|e| std::path::PathBuf::from(&e.value));
            let ctx = crate::officina::OpCtx {
                cfg,
                snap,
                model_path,
                profile: None,
            };
            let out = app.officina.run(&line, &ctx);
            for l in out {
                app.officina.output.push_back(l);
            }
            if app.officina.output.len() > 500 {
                let excess = app.officina.output.len() - 500;
                for _ in 0..excess {
                    app.officina.output.pop_front();
                }
            }
        }
        KeyCode::Tab => {
            let model_path = app
                .config_file
                .entries
                .iter()
                .find(|e| e.section == "model" && e.key == "path")
                .map(|e| std::path::PathBuf::from(&e.value));
            let model_path = model_path.as_deref();
            app.officina.ensure_catalog(model_path);
            let cfg = &app.cfg;
            let snap = &app.snapshot;
            let ctx = crate::officina::OpCtx {
                cfg,
                snap,
                model_path: model_path.map(std::path::Path::to_path_buf),
                profile: None,
            };
            app.officina.cycle_complete(&ctx);
        }
        KeyCode::PageUp => app.officina.output_scroll_lines(10, 20),
        KeyCode::PageDown => app.officina.output_scroll_lines(-10, 20),
        KeyCode::Backspace => app.officina.backspace(),
        KeyCode::Up => app.officina.history_nav(-1),
        KeyCode::Down => app.officina.history_nav(1),
        KeyCode::Char(c) => app.officina.type_char(c),
        _ => {}
    }
}

/// SUBSYSTEMS tab keys: navigate rows; Enter on ASCENSUS opens/advances the
/// key+model editor.
fn handle_subsystems_key(app: &mut App, key: crossterm::event::KeyEvent) {
    if app.ascensus_edit.is_some() {
        handle_ascensus_edit(app, key);
        return;
    }
    match key.code {
        KeyCode::Char('j') | KeyCode::Down => app.subsystem_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.subsystem_move(-1),
        KeyCode::Enter => app.ascensus_edit_start(),
        _ => {}
    }
}

/// ASCENSUS editor keys (SUBSYSTEMS tab).
fn handle_ascensus_edit(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char(',') | KeyCode::Char('.') | KeyCode::Tab => app.ascensus_edit_toggle_field(),
        KeyCode::Enter => {
            let _ = app.ascensus_edit_next();
        }
        KeyCode::Backspace => app.ascensus_edit_backspace(),
        KeyCode::Esc => app.ascensus_edit_cancel(),
        KeyCode::Char(c) => app.ascensus_edit_type(c),
        _ => {}
    }
}

/// PROFILES tab keys: dispatch by mode (prompt / list / config panes).
fn handle_profiles_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    ctrl: &mpsc::Sender<control::Event>,
) {
    if app.profile_prompt.is_some() {
        handle_profile_prompt(app, key);
        return;
    }
    match app.profile_focus {
        app::ProfileFocus::List => handle_profile_list(app, key, ctrl),
        app::ProfileFocus::Config => handle_profile_config(app, key),
    }
}

/// PROFILES save-as name prompt keys.
fn handle_profile_prompt(app: &mut App, key: crossterm::event::KeyEvent) {
    match key.code {
        KeyCode::Char(c) => app.profile_save_type(c),
        KeyCode::Backspace => app.profile_save_backspace(),
        KeyCode::Enter => {
            if app.profile_dup_source.is_some() {
                let _ = app.profile_duplicate_commit();
            } else {
                let _ = app.profile_save_commit();
            }
        }
        KeyCode::Esc => app.profile_save_cancel(),
        _ => {}
    }
}

/// PROFILES list-pane keys: navigate, load, select-for-start, overwrite, sweep, delete, reload.
fn handle_profile_list(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    ctrl: &mpsc::Sender<control::Event>,
) {
    match key.code {
        KeyCode::Char(',') | KeyCode::Char('.') | KeyCode::Left | KeyCode::Right => {
            app.profile_pane_toggle()
        }
        KeyCode::Char('j') | KeyCode::Down => app.profile_list_move(1),
        KeyCode::Char('k') | KeyCode::Up => app.profile_list_move(-1),
        KeyCode::Enter | KeyCode::Char('l') => {
            let _ = app.profile_load_selected();
        }
        KeyCode::Char('c') => app.profile_duplicate_start(),
        KeyCode::Char('t') => {
            let name = app
                .profiles
                .get(app.profile_list_selection)
                .map(|p| p.name.clone());
            app.select_profile(name);
        }
        KeyCode::Char('w') => {
            let _ = app.profile_overwrite_selected();
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
        KeyCode::Char(',') | KeyCode::Char('.') | KeyCode::Left | KeyCode::Right => {
            app.profile_pane_toggle()
        }
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

/// Route a mouse event. Header tabs are clickable on every tab; the PROFILES
/// footer buttons dispatch only while the PROFILES tab is shown (and no save/
/// duplicate prompt is swallowing the click).
fn handle_mouse(app: &mut App, mouse: MouseEvent, ctrl: &mpsc::Sender<control::Event>) {
    if let Some(tab) = app.tab_click(mouse.column, mouse.row) {
        app.set_tab(tab);
        return;
    }
    if app.tab == app::Tab::Profiles && app.profile_prompt.is_none() {
        if let Some(action) = app.profile_click(mouse.column, mouse.row) {
            run_profile_action(app, action, ctrl);
        }
    }
}

/// Map a clicked PROFILES footer button to its app mutator. Sweep is the only
/// action that needs the control channel.
fn run_profile_action(
    app: &mut App,
    action: app::ProfileAction,
    ctrl: &mpsc::Sender<control::Event>,
) {
    use app::ProfileAction::*;
    match action {
        SwitchPane => app.profile_pane_toggle(),
        Add => app.profile_save_start(),
        Duplicate => app.profile_duplicate_start(),
        Delete => match app.profile_focus {
            app::ProfileFocus::List => {
                let _ = app.profile_delete_selected();
            }
            app::ProfileFocus::Config => {
                let _ = app.profile_remove_selected();
            }
        },
        Reload => match app.profile_focus {
            app::ProfileFocus::List => app.profile_reload_list(),
            app::ProfileFocus::Config => app.profile_reload(),
        },
        Load => {
            let _ = app.profile_load_selected();
        }
        Start => {
            let name = app
                .profiles
                .get(app.profile_list_selection)
                .map(|p| p.name.clone());
            app.select_profile(name);
        }
        Overwrite => {
            let _ = app.profile_overwrite_selected();
        }
        Sweep => { /* moved to the SWEEP tab (profile-independent) */ }
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
