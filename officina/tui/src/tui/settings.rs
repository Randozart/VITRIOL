//! Finite slash-setting menus. PROVENANCE: original Officina implementation;
//! explicit options reuse the same local/RPC handlers as typed commands.
use super::state::AppState;
use crate::{rpc::protocol::RpcCommand, theme};
use crossterm::event::KeyCode;
use ratatui::{
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::base::ClearBase;

#[derive(Debug)]
pub struct SettingMenu {
    pub title: String,
    pub choices: Vec<String>,
    pub selected: usize,
    pub current: String,
}

fn open(state: &mut AppState, title: &str, options: &[&str], current: &str) {
    state.setting_menu = Some(SettingMenu {
        title: title.into(),
        choices: options.iter().map(|s| (*s).into()).collect(),
        selected: options.iter().position(|v| *v == current).unwrap_or(0),
        current: current.into(),
    });
}

const SETTINGS: &[&str] = &[
    "fire",
    "chat-bg",
    "glimmer",
    "sidebar",
    "thinking",
    "thinking-view",
    "permissions",
    "diag",
];
const CHAT_BG: &[&str] = &["on", "off"];
const FIRE: &[&str] = &[
    "off",
    "on",
    "prismatic",
    "emerald",
    "alchemy",
    "tint on",
    "tint off",
];
const STYLES: &[&str] = &["prismatic", "emerald", "alchemy"];
const THINKING: &[&str] = &["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const VIEW: &[&str] = &[
    "show",
    "hide",
    "collapse",
    "expand",
    "default collapsed",
    "default expanded",
];

/// None means an unrelated command; Some(None) was handled locally.
pub fn command(state: &mut AppState, message: &str) -> Option<Option<RpcCommand>> {
    let (name, args) = message
        .trim()
        .split_once(' ')
        .unwrap_or((message.trim(), ""));
    let args = args.trim();
    if matches!(name, "/settings" | "/config") {
        if args.is_empty() {
            open(state, "/settings", SETTINGS, "");
            return Some(None);
        }
        let result = command(state, &format!("/{args}"));
        if result.is_none() {
            state.notice = Some(("settings: unknown setting".into(), "warning".into()));
        }
        return Some(result.flatten());
    }
    if matches!(name, "/yolo" | "/permissions" | "/perms") && args.is_empty() {
        state.open_permissions_modal();
        return Some(None);
    }
    // `/theme` has a DYNAMIC option list (one entry per Theme static), so it
    // builds its own menu instead of going through open_setting's static table.
    if message.trim() == "/theme" {
        let names: Vec<&'static str> = crate::theme::ALL.iter().map(|t| t.name).collect();
        open(state, "/theme", &names, crate::theme::current().name);
        return Some(None);
    }
    if open_setting(state, message.trim()) {
        return Some(None);
    }
    apply(state, name, args)
}

fn apply(state: &mut AppState, name: &str, args: &str) -> Option<Option<RpcCommand>> {
    let label = match name {
        "/theme" => match state.set_theme(args, true) {
            Some(applied) => format!("theme: {applied}"),
            None => {
                let names: Vec<&str> = crate::theme::ALL.iter().map(|t| t.name).collect();
                state.notice = Some((
                    format!("theme: unknown '{args}' — try {}", names.join(", ")),
                    "warning".into(),
                ));
                return Some(None);
            }
        },
        "/fire" => {
            let label = state.set_fire(args);
            super::persist_fire(state);
            format!("fire: {label}")
        }
        // User-turn background band (owner request 2026-11-16).
        "/chat-bg" => {
            let label = state.set_user_turn_bg(args);
            super::persist_chat_bg(state);
            format!("chat-bg: {label}")
        }
        "/glimmer" | "/shimmer" => {
            let label = state.set_glimmer(args);
            super::persist_glimmer(state);
            format!("glimmer: {label}")
        }
        "/sidebar" => {
            state.sidebar_force = match args {
                "auto" => None,
                "show" | "on" => Some(true),
                "hide" | "off" => Some(false),
                _ => {
                    state.notice =
                        Some(("sidebar: use auto | show | hide".into(), "warning".into()));
                    return Some(None);
                }
            };
            format!("sidebar: {args}")
        }
        "/diag" if matches!(args, "on" | "off") => {
            state.show_diag = args == "on";
            format!("diagnostics: {args}")
        }
        "/thinking-view" => {
            state.thinking_view_command(args);
            return Some(None);
        }
        "/thinking" if !args.is_empty() => {
            return Some(state.local_dispatch(&format!("/thinking {args}")))
        }
        _ => return None,
    };
    state.notice = Some((label, "info".into()));
    Some(None)
}

fn open_setting(state: &mut AppState, name: &str) -> bool {
    let sidebar = match state.sidebar_force {
        None => "auto",
        Some(true) => "show",
        Some(false) => "hide",
    };
    let fire = if state.fire_on {
        state.fire_style.label()
    } else {
        "off"
    };
    let tint = if state.fire_tint {
        "tint on"
    } else {
        "tint off"
    };
    let view = if state.hide_thinking { "hide" } else { "show" };
    let (title, options, current): (&str, &[&str], &str) = match name {
        "/fire" => (name, FIRE, fire),
        "/chat-bg" => (name, CHAT_BG, if state.user_turn_bg { "on" } else { "off" }),
        "/fire style" => ("/fire", STYLES, state.fire_style.label()),
        "/fire tint" => ("/fire", &["tint on", "tint off"], tint),
        "/glimmer" | "/shimmer" => (
            "/glimmer",
            &["shimmer", "breathe", "twinkle", "off"],
            state.glimmer.label(),
        ),
        "/sidebar" => (name, &["auto", "show", "hide"], sidebar),
        "/thinking" => (
            name,
            THINKING,
            state.thinking_level.as_deref().unwrap_or(""),
        ),
        "/thinking-view" => (name, VIEW, view),
        "/diag" => (
            name,
            &["on", "off"],
            if state.show_diag { "on" } else { "off" },
        ),
        _ => return false,
    };
    let current = current.to_string();
    open(state, title, options, &current);
    true
}

/// Consume menu keys and return the selected explicit command for normal dispatch.
pub fn key(state: &mut AppState, code: KeyCode) -> Option<String> {
    let menu = state.setting_menu.as_mut()?;
    match code {
        KeyCode::Esc => {
            state.setting_menu = None;
        }
        KeyCode::Up => menu.selected = menu.selected.saturating_sub(1),
        KeyCode::Down => {
            menu.selected = (menu.selected + 1).min(menu.choices.len().saturating_sub(1))
        }
        KeyCode::Enter => {
            let command = format!("{} {}", menu.title, menu.choices[menu.selected]);
            state.setting_menu = None;
            return Some(command);
        }
        _ => {}
    }
    None
}

pub fn render(frame: &mut Frame, menu: &SettingMenu, area: Rect) {
    let height = (menu.choices.len() as u16 + 3).min(area.height);
    let panel = Rect::new(
        area.x,
        area.bottom().saturating_sub(height),
        area.width.min(46),
        height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} · ↑↓ Enter · Esc ", menu.title))
        .style(Style::new().bg(theme::PANEL()).fg(theme::GOLD()));
    let inner = block.inner(panel);
    let visible = inner.height as usize;
    let start = (menu.selected + 1).saturating_sub(visible);
    let rows = menu
        .choices
        .iter()
        .enumerate()
        .skip(start)
        .take(visible)
        .map(|(i, choice)| {
            Line::from(format!(
                "{} {}{}",
                if i == menu.selected { "›" } else { " " },
                choice,
                if *choice == menu.current { " ✓" } else { "" }
            ))
        });
    frame.render_widget(ClearBase, panel);
    frame.render_widget(block, panel);
    frame.render_widget(
        Paragraph::new(rows.collect::<Vec<_>>())
            .style(Style::new().bg(theme::PANEL()).fg(theme::TEXT())),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_settings_do_not_mutate_and_escape_preserves_draft() {
        for name in [
            "/fire",
            "/glimmer",
            "/shimmer",
            "/sidebar",
            "/thinking",
            "/thinking-view",
            "/diag",
            "/settings",
            "/settings fire",
            "/config glimmer",
            "/fire tint",
            "/fire style",
        ] {
            let mut state = AppState::default();
            state.input = "my unfinished draft".into();
            let before = (
                state.fire_on,
                state.fire_style,
                state.glimmer,
                state.sidebar_force,
                state.hide_thinking,
                state.show_diag,
            );
            assert!(command(&mut state, name).is_some(), "{name}");
            assert!(state.setting_menu.is_some(), "{name}");
            key(&mut state, KeyCode::Down);
            key(&mut state, KeyCode::Esc);
            assert!(state.setting_menu.is_none());
            assert_eq!(
                before,
                (
                    state.fire_on,
                    state.fire_style,
                    state.glimmer,
                    state.sidebar_force,
                    state.hide_thinking,
                    state.show_diag
                )
            );
            assert_eq!(state.input, "my unfinished draft");
        }
    }

    #[test]
    fn picker_emits_explicit_options_and_thinking_rpc_matches_typed_command() {
        let mut state = AppState::default();
        command(&mut state, "/sidebar");
        key(&mut state, KeyCode::Down);
        let selected = key(&mut state, KeyCode::Enter).unwrap();
        assert_eq!(selected, "/sidebar show");
        command(&mut state, &selected);
        assert_eq!(state.sidebar_force, Some(true));
        command(&mut state, "/thinking");
        state.setting_menu.as_mut().unwrap().selected = 4;
        let selected = key(&mut state, KeyCode::Enter).unwrap();
        assert_eq!(selected, "/thinking high");
        assert!(
            matches!(command(&mut state, &selected), Some(Some(RpcCommand::SetThinkingLevel { level, .. })) if level == "high")
        );
        assert!(command(&mut state, "/arbitrary-extension").is_none());
        assert!(command(&mut state, "/settings-extra").is_none());
    }
}
