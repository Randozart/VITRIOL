//! Terminal operations console for VITRIOL.

mod app;
mod braille;
mod config;
mod control;
mod intel;
mod logs;
mod model;
mod nvidia;
mod poller;
mod profile;
mod theme;
mod ui;

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

use crate::app::{App, LogSource, Menu, Tab};
use crate::control::{Action, ServiceAction};

const POLL_INTERVAL: Duration = Duration::from_secs(2);

fn main() -> io::Result<()> {
    let cfg = config::Config::from_env();
    let (tx, rx) = mpsc::channel();
    let (ctrl_tx, ctrl_rx) = mpsc::channel();
    let refresh = Arc::new(AtomicBool::new(false));
    poller::spawn(cfg.clone(), tx, Arc::clone(&refresh));
    let mut terminal = ratatui::init();
    let _ = theme::init_from_environment();
    let result = (|| {
        crossterm::execute!(io::stdout(), event::EnableMouseCapture)?;
        let mut app = App::new(cfg);
        loop {
            for snapshot in rx.try_iter() {
                app.apply_snapshot(snapshot);
            }
            for result in ctrl_rx.try_iter() {
                app.finish_action(result);
                refresh.store(true, Ordering::Relaxed);
            }
            terminal.draw(|frame| ui::draw(frame, &mut app))?;
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if !handle_key(&mut app, key, &ctrl_tx, &refresh) {
                        break;
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        if let Some(tab) = app.tab_click(mouse.column, mouse.row) {
                            app.set_tab(tab);
                        }
                    }
                    MouseEventKind::ScrollUp if app.logs_visible() => {
                        app.log_offset = app.log_offset.saturating_add(3);
                    }
                    MouseEventKind::ScrollDown if app.logs_visible() => {
                        app.log_offset = app.log_offset.saturating_sub(3);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(())
    })();
    let _ = crossterm::execute!(io::stdout(), event::DisableMouseCapture);
    ratatui::restore();
    result
}

fn handle_key(
    app: &mut App,
    key: KeyEvent,
    tx: &mpsc::Sender<control::Event>,
    refresh: &AtomicBool,
) -> bool {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    if app.search_open {
        match key.code {
            KeyCode::Enter => app.search_open = false,
            KeyCode::Esc => {
                app.search_open = false;
                app.log_query.clear();
            }
            KeyCode::Backspace => {
                app.log_query.pop();
            }
            KeyCode::Char(c) => app.log_query.push(c),
            _ => {}
        }
        app.log_offset = 0;
        return true;
    }
    match key.code {
        KeyCode::Char('q' | 'Q') => return false,
        KeyCode::Tab | KeyCode::BackTab => app.next_tab(),
        KeyCode::Char('T') => {
            let _ = theme::cycle();
        }
        KeyCode::Char('r') => refresh.store(true, Ordering::Relaxed),
        _ if app.menu.is_some() => menu_key(app, key.code, tx),
        KeyCode::Char('l') if app.tab == Tab::Dashboard => app.toggle_log_sidebar(),
        KeyCode::Char('a') if app.tab == Tab::Dashboard && !app.action_running => {
            app.menu = Some(Menu::Actions(0));
        }
        _ if app.logs_visible() => log_key(app, key.code),
        _ => {}
    }
    true
}

fn log_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char(c @ '1'..='5') => {
            app.set_log_source(LogSource::ALL[c as usize - '1' as usize])
        }
        KeyCode::Char('[') => app.cycle_log_slot(-1),
        KeyCode::Char(']') => app.cycle_log_slot(1),
        KeyCode::Left => app.cycle_log_source(-1),
        KeyCode::Right => app.cycle_log_source(1),
        KeyCode::Char('v') => app.logs_verbose = !app.logs_verbose,
        KeyCode::Char(' ') => app.toggle_pause(),
        KeyCode::Char('/') => app.search_open = true,
        KeyCode::Esc => {
            app.log_query.clear();
            app.log_offset = 0;
        }
        KeyCode::Up => app.log_offset = app.log_offset.saturating_add(1),
        KeyCode::Down => app.log_offset = app.log_offset.saturating_sub(1),
        KeyCode::PageUp => app.log_offset = app.log_offset.saturating_add(12),
        KeyCode::PageDown => app.log_offset = app.log_offset.saturating_sub(12),
        KeyCode::End => app.log_offset = 0,
        _ => {}
    }
}

fn menu_key(app: &mut App, key: KeyCode, tx: &mpsc::Sender<control::Event>) {
    let Some(menu) = app.menu.as_mut() else {
        return;
    };
    let (index, len) = match menu {
        Menu::Actions(index) => (index, 4),
        Menu::Profiles(index, profiles) => (index, profiles.len()),
    };
    match key {
        KeyCode::Esc => app.menu = None,
        KeyCode::Up if len > 0 => *index = (*index + len - 1) % len,
        KeyCode::Down if len > 0 => *index = (*index + 1) % len,
        KeyCode::Enter => match menu {
            Menu::Actions(0) => {
                let profiles = profile::discover(&app.cfg);
                let index = profiles
                    .iter()
                    .position(|p| Some(&p.name) == app.snapshot.configured_profile.as_ref())
                    .unwrap_or(0);
                app.menu = Some(Menu::Profiles(index, profiles));
            }
            Menu::Actions(index) => {
                let action = [
                    ServiceAction::Start,
                    ServiceAction::Restart,
                    ServiceAction::Stop,
                ][*index - 1];
                app.run_action(Action::Service(action), tx);
            }
            Menu::Profiles(index, profiles) => {
                if let Some(name) = profiles.get(*index).map(|profile| profile.name.clone()) {
                    app.run_action(Action::Load(name), tx);
                }
            }
        },
        _ => {}
    }
}
