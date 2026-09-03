pub mod ansi;
pub mod layout;
pub mod state;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_lite::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::rpc::bridge::{RpcBridge, WireMessage};
use crate::rpc::protocol::{RpcCommand, SlashCommand};

use self::layout::render;
use self::state::AppState;

pub async fn run(bridge: &mut RpcBridge, cwd: std::path::PathBuf) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Mouse capture: wheel scrolling through session history (owner request
    // 2026-09-02). Trade-off: terminals usually need Shift+drag to select
    // text while captured.
    execute!(stdout, EnterAlternateScreen, crossterm::event::EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::default();
    state.cwd = cwd;
    load_glimmer(&mut state);
    load_fire(&mut state);
    load_tools_config(&mut state);
    let mut rx = bridge.start_reader();

    // Dedicated input task: crossterm EventStream → channel. Never poll
    // from the select! loop — concurrent event::poll calls race and eat
    // keys. Keys AND mouse both route through here (wheel = scrollback).
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    tokio::spawn(async move {
        let mut stream = EventStream::new();
        while let Some(Ok(ev)) = stream.next().await {
            if input_tx.send(ev).is_err() {
                break;
            }
        }
    });

    // Initial handshake: state, transcript, commands
    let _ = bridge.request(RpcCommand::GetState { id: None }).await;
    let _ = bridge.request(RpcCommand::GetMessages { id: None }).await;
    let _ = bridge.request(RpcCommand::GetCommands { id: None }).await;

    let result = run_loop(&mut terminal, &mut state, &mut rx, &mut input_rx, bridge).await;
    disable_raw_mode()?;
    disable_raw_mode()?;
    // Mouse capture OFF before the alternate screen exit (reverse order).
    execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen
    )?;

    result
}

// ── Glimmer persistence (owner request 2026-09-02) ───────────────────────
// OFFICINA_GLIMMER env > ~/.vitriol/officina/tui-glimmer file > Shimmer.

fn glimmer_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
        .join(".vitriol")
        .join("officina")
        .join("tui-glimmer")
}

fn load_glimmer(state: &mut AppState) {
    if let Ok(m) = std::env::var("OFFICINA_GLIMMER") {
        if let Some(g) = crate::watermark::GlimmerMode::parse(m.trim()) {
            state.glimmer = g;
            return;
        }
    }
    if let Ok(s) = std::fs::read_to_string(glimmer_path()) {
        if let Some(g) = crate::watermark::GlimmerMode::parse(s.trim()) {
            state.glimmer = g;
        }
    }
}

fn persist_glimmer(state: &AppState) {
    if let Some(dir) = glimmer_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(glimmer_path(), format!("{}\n", state.glimmer.label()));
}

// ── Fire persistence (owner request 2026-09-02) ──────────────────────────
// OFFICINA_FIRE / OFFICINA_FIRE_STYLE env > ~/.vitriol/officina/tui-fire
// ("<on|off> <style>"; bare on/off lines from earlier builds stay valid)
// > on + emerald.

fn fire_path() -> std::path::PathBuf {
    glimmer_path().with_file_name("tui-fire")
}

fn load_fire(state: &mut AppState) {
    if std::env::var("OFFICINA_FIRE").map(|v| v == "0").unwrap_or(false) {
        state.fire_on = false;
    }
    if let Ok(s) = std::env::var("OFFICINA_FIRE_STYLE") {
        if let Some(style) = crate::fire::FireStyle::parse(s.trim()) {
            state.fire_style = style;
        }
    }
    if let Ok(src) = std::fs::read_to_string(fire_path()) {
        let tokens: Vec<&str> = src.split_whitespace().collect();
        let mut words = tokens.iter();
        if let Some(&w) = words.next() {
            match w {
                "off" | "0" => state.fire_on = false,
                "on" | "1" => state.fire_on = true,
                _ => {}
            }
        }
        if let Some(&w) = words.next() {
            if let Some(style) = crate::fire::FireStyle::parse(w) {
                state.fire_style = style;
            }
        } else if let Some(style) = crate::fire::FireStyle::parse(src.trim()) {
            // A lone style word means "on, this voice".
            state.fire_style = style;
            state.fire_on = true;
        }
        // Text tint (owner request 2026-09-03) — `tint on|off` anywhere in
        // the file; absent = the default (on).
        if let Some(i) = tokens.iter().position(|&w| w == "tint") {
            if let Some(&v) = tokens.get(i + 1) {
                state.fire_tint = v != "off";
            }
        }
    }
}

fn persist_fire(state: &AppState) {
    if let Some(dir) = fire_path().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let line = if state.fire_on {
        format!("on {}", state.fire_style.label())
    } else {
        "off".to_string()
    };
    let tint = if state.fire_tint { "tint on" } else { "tint off" };
    let _ = std::fs::write(fire_path(), format!("{line}\n{tint}\n"));
}

// ── Tool verbosity persistence (owner request 2026-09-03) ─────────────────
// ~/.vitriol/officina/tui-tools — one directive per line:
//   default <mode>
//   <tool_name> <mode>[!|+|-]

fn tools_path() -> std::path::PathBuf {
    glimmer_path().with_file_name("tui-tools")
}

fn load_tools_config(state: &mut state::AppState) {
    let path = tools_path();
    if let Ok(src) = std::fs::read_to_string(&path) {
        for line in src.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("default ") {
                if let Some(mode) = state::ToolVerbosity::parse(rest.trim()) {
                    state.tool_default = mode;
                }
            } else if let Some((name, mode_s)) = line.split_once(' ') {
                let name = name.trim().to_string();
                let mode_s = mode_s.trim();
                if mode_s == "clear" {
                    state.tool_overrides.remove(&name);
                } else if let Some(o) = state::ToolOverride::parse(mode_s) {
                    state.tool_overrides.insert(name, o);
                }
            }
        }
    }
}

fn persist_tools_config(state: &state::AppState) {
    let path = tools_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut lines = vec![format!("default {}", state.tool_default.label())];
    for (name, o) in &state.tool_overrides {
        lines.push(format!("{} {}", name, o.encode()));
    }
    let _ = std::fs::write(&path, format!("{}\n", lines.join("\n")));
}

/// /tools handler — bare opens the modal; args CLI-set overrides:
///   /tools write block+     at-least (bumps up when global less verbose)
///   /tools bash line-       at-most  (caps when global more verbose)
///   /tools read full!       pinned   (ignores global)
///   /tools write clear      remove override
///   /tools default full     set global default
fn tools_command(state: &mut state::AppState, args: &str) {
    if args.is_empty() {
        state.tools_modal_open = true;
        state.tools_modal_sel = 0;
        return;
    }
    if let Some((name, mode_s)) = args.split_once(' ') {
        let name = name.trim();
        let mode_s = mode_s.trim();
        if name == "default" {
            match state::ToolVerbosity::parse(mode_s) {
                Some(m) => {
                    state.tool_default = m;
                    state.tools_gen += 1;
                    persist_tools_config(state);
                    state.notice = Some((format!("tools default: {}", m.label()), "info".to_string()));
                }
                None => tools_usage(state),
            }
            return;
        }
        if mode_s == "clear" {
            if state.tool_overrides.remove(name).is_some() {
                state.tools_gen += 1;
                persist_tools_config(state);
                state.notice = Some((format!("{}: override cleared", name), "info".to_string()));
            } else {
                state.notice = Some((format!("{}: no override", name), "info".to_string()));
            }
            return;
        }
        match state::ToolOverride::parse(mode_s) {
            Some(o) => {
                state.tool_overrides.insert(name.to_string(), o);
                state.tools_gen += 1;
                persist_tools_config(state);
                state.notice = Some((format!("{}: {}", name, o.encode()), "info".to_string()));
            }
            None => tools_usage(state),
        }
        return;
    }
    // Single word — could be a bare mode (global default) or an unknown tool.
    if let Some(m) = state::ToolVerbosity::parse(args) {
        state.tool_default = m;
        state.tools_gen += 1;
        persist_tools_config(state);
        state.notice = Some((format!("tools default: {}", m.label()), "info".to_string()));
        return;
    }
    tools_usage(state);
}

fn tools_usage(state: &mut state::AppState) {
    state.entries.push(state::ChatEntry::Diag(
        "usage: /tools [<tool> <mode>[!|+|-] | <tool> clear | default <mode>]".to_string(),
    ));
    state.entries.push(state::ChatEntry::Diag(
        "modes: line | block | full · strictness: ! pinned, + at-least, - at-most".to_string(),
    ));
}

async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<WireMessage>,
    input_rx: &mut tokio::sync::mpsc::UnboundedReceiver<Event>,
    bridge: &mut RpcBridge,
) -> Result<()> {
    let mut last_frame = std::time::Instant::now();
    loop {
        if state.show_diag {
            state.diag_view = bridge
                .diag
                .snapshot()
                .into_iter()
                .filter(|l| {
                    !l.is_empty()
                        && !l.contains("applyModeTheme") // extension theme debug
                        && !l.contains("]777;") // OSC 777 notify echo
                })
                .collect();
        }
        // Fire low-pass — dt-based so the flame breathes at the same tempo
        // at 2 fps and 11 fps alike.
        let now = std::time::Instant::now();
        let dt = now.duration_since(last_frame).as_secs_f64();
        last_frame = now;
        state.step_fire(dt);
        terminal.draw(|frame| render(frame, state))?;

        // Dynamic redraw cadence: fresh screens (glimmer) and live fires
        // animate at ~11 fps; busy screens ride the 500 ms heartbeat.
        let delay = if state.entries.is_empty() || state.fire_level > 0.0 {
            Duration::from_millis(90)
        } else {
            Duration::from_millis(500)
        };

        tokio::select! {
            maybe_input = input_rx.recv() => {
                match maybe_input {
                    Some(Event::Key(key)) => {
                        handle_key(key, state, bridge).await;
                        if state.should_quit {
                            terminal.draw(|frame| render(frame, state))?;
                            break;
                        }
                    }
                    Some(Event::Mouse(m)) => handle_mouse(m, state).await,
                    Some(_) => {} // resize / focus events — next draw picks them up
                    None => break, // input stream ended
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(WireMessage::Response(resp)) => {
                        if handle_response(state, &resp) {
                            let _ = bridge.request(RpcCommand::GetState { id: None }).await;
                            let _ = bridge.request(RpcCommand::GetMessages { id: None }).await;
                        }
                    }
                    Some(WireMessage::Event(event)) => {
                        handle_event(state, &event.event_type, &event.fields);
                    }
                    Some(WireMessage::ExtensionUiRequest(req)) => {
                        handle_ui_request(state, &req.method, &req.fields, bridge).await;
                    }
                    None => {
                        state.entries.push(state::ChatEntry::Diag(
                            "agent process exited".to_string(),
                        ));
                        terminal.draw(|frame| render(frame, state))?;
                        tokio::time::sleep(Duration::from_millis(1200)).await;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep(delay) => {
                // heartbeat redraw — keeps streaming indicator/gauges alive
                // (90 ms on fresh screens so the watermark glimmer animates)
            }
        }
    }

    Ok(())
}

async fn handle_key(key: KeyEvent, state: &mut AppState, bridge: &mut RpcBridge) {
    // 2026-09-03 keymap (owner: "just so you don't accidentally quit"):
    // ^q and ^d REMOVED — the only quit paths are ^esc and /quit, both
    // deliberate. ^c copies (selection, else last reply) — never quits.
    // Esc aborts streaming but NEVER quits when idle.

    // /help modal (F1) — read-only reference; any of esc/enter/q closes.
    if state.help_open {
        let len = state::help_rows().len();
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.help_open = false,
            KeyCode::Up => state.help_sel = state.help_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.help_sel + 1 < len {
                    state.help_sel += 1;
                }
            }
            KeyCode::Char('k') => state.help_sel = state.help_sel.saturating_sub(1),
            KeyCode::Char('j') => {
                if state.help_sel + 1 < len {
                    state.help_sel += 1;
                }
            }
            _ => {}
        }
        return;
    }

    // /tools modal captures navigation while open.
    if state.tools_modal_open {
        let row_count = state::KNOWN_TOOLS.len() + 1; // + global row
        match key.code {
            KeyCode::Esc => state.tools_modal_open = false,
            KeyCode::Up => state.tools_modal_sel = state.tools_modal_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.tools_modal_sel + 1 < row_count {
                    state.tools_modal_sel += 1;
                }
            }
            KeyCode::Enter => {
                // Row 0 = global default; rows 1.. = known tools.
                if state.tools_modal_sel == 0 {
                    state.tool_default = state.tool_default.next();
                } else {
                    let name = state::KNOWN_TOOLS[state.tools_modal_sel - 1];
                    match state.tool_overrides.get_mut(name) {
                        Some(o) => {
                            o.mode = o.mode.next();
                        }
                        None => {
                            // First press: start at global, bumped one step,
                            // pinned — the modal is where you pin tools.
                            let start = state.tool_default.next();
                            state.tool_overrides.insert(
                                name.to_string(),
                                state::ToolOverride {
                                    mode: start,
                                    strictness: state::Strictness::Pinned,
                                },
                            );
                        }
                    }
                }
                state.tools_gen += 1;
                persist_tools_config(state);
            }
            // Tab cycles strictness on a tool row (global row: no-op).
            KeyCode::Tab => {
                if state.tools_modal_sel > 0 {
                    let name = state::KNOWN_TOOLS[state.tools_modal_sel - 1];
                    if let Some(o) = state.tool_overrides.get_mut(name) {
                        o.strictness = match o.strictness {
                            state::Strictness::Pinned => state::Strictness::AtLeast,
                            state::Strictness::AtLeast => state::Strictness::AtMost,
                            state::Strictness::AtMost => state::Strictness::Pinned,
                        };
                        state.tools_gen += 1;
                        persist_tools_config(state);
                    }
                }
            }
            // Backspace/Delete clears the override on a tool row.
            KeyCode::Backspace | KeyCode::Delete => {
                if state.tools_modal_sel > 0 {
                    let name = state::KNOWN_TOOLS[state.tools_modal_sel - 1];
                    if state.tool_overrides.remove(name).is_some() {
                        state.tools_gen += 1;
                        persist_tools_config(state);
                    }
                }
            }
            _ => {}
        }
        return;
    }

    // /resume modal captures navigation while open.
    if state.resume_open {
        match key.code {
            KeyCode::Esc => state.resume_open = false,
            KeyCode::Up => state.resume_sel = state.resume_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.resume_sel + 1 < state.resume_entries.len() {
                    state.resume_sel += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = state.resume_entries.get(state.resume_sel) {
                    let path = entry.path.to_string_lossy().to_string();
                    state.resume_open = false;
                    if state.is_streaming {
                        state
                            .entries
                            .push(state::ChatEntry::Diag("dissolve first — session not switched".into()));
                        return;
                    }
                    let _ = bridge
                        .request(RpcCommand::SwitchSession {
                            id: None,
                            session_path: path,
                        })
                        .await;
                    // Success response triggers clear + rehydrate (refresh flag).
                } else {
                    state.resume_open = false;
                }
            }
            _ => {}
        }
        return;
    }

    match key.code {
        // ^esc — quit. The only physical quit chord (owner-approved; some
        // desktops reserve it, /quit always works). MUST precede the plain
        // Esc arm or the guard never fires.
        KeyCode::Esc if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Esc => {
            if state.is_streaming {
                let _ = bridge.request(RpcCommand::Abort { id: None }).await;
            }
            // Idle Esc: close-overlays is handled above (modals); here it
            // is a deliberate NO-OP — it used to quit (owner report:
            // accidental quits).
        }
        // ^c — copy (owner request 2026-09-03). Selection first, else the
        // last assistant reply. Never aborts, never quits.
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            let text = state
                .extract_selection()
                .or_else(|| state.last_assistant());
            match text.filter(|t| !t.trim().is_empty()) {
                Some(t) => {
                    let n = t.chars().count();
                    let via = copy_to_clipboard(&t);
                    state.notice = Some((format!("copied {} chars via {}", n, via), "info".to_string()));
                }
                None => {
                    state.notice = Some(("nothing to copy".to_string(), "info".to_string()));
                }
            }
        }
        // ctrl+v — cycle global tool verbosity (owner request 2026-09-03:
        // V for verbose). Line → Block → Full → Line. Persisted.
        KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.tool_default = state.tool_default.next();
            state.tools_gen += 1;
            let label = state.tool_default.label();
            state.notice = Some((format!("tools: {}", label), "info".to_string()));
            persist_tools_config(state);
        }
        // ctrl+t — /tools modal picker (owner request 2026-09-03).
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.tools_modal_open = !state.tools_modal_open;
            state.tools_modal_sel = 0;
        }
        KeyCode::F(9) => {
            state.show_diag = !state.show_diag;
        }
        // F1 — help modal (owner request 2026-09-03).
        KeyCode::F(1) => {
            state.help_open = !state.help_open;
            state.help_sel = 0;
        }
        // Tab: complete selection when the popup is open; otherwise cycle
        // agent mode — "/mode <next>" (bare /mode only lists, agent-mode.ts).
        KeyCode::Tab => {
            if state.cand_sel_logic_open() {
                state.complete_command();
            } else if let Some(next) = state.next_agent_mode() {
                let m = format!("/mode {}", next);
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: m,
                        images: None,
                        streaming_behavior: None,
                    })
                    .await;
            }
        }
        KeyCode::Up => {
            state.cycle_candidate(false);
        }
        KeyCode::Down => {
            state.cycle_candidate(true);
        }
        // Scrollback (owner request 2026-09-02): PgUp/PgDn page through
        // session history.
        KeyCode::PageUp => {
            state.scroll_up(20);
        }
        KeyCode::PageDown => {
            state.scroll_down(20);
        }
        KeyCode::Enter => {
            // Resume modal open → handled above (modal key capture).
            let msg = std::mem::take(&mut state.input);
            state.cursor = 0;
            state.cand_sel = 0;
            if msg.is_empty() {
                return;
            }
            // /fire — pure-local UI state (composer flames); never RPC.
            if msg == "/fire" || msg.starts_with("/fire ") {
                let args = msg["/fire".len()..].trim();
                let label = state.set_fire(args);
                state.notice = Some((format!("fire: {}", label), "info".to_string()));
                persist_fire(state);
                return;
            }
            // /glimmer — pure-local UI state (watermark glimmer); never RPC.
            if msg == "/glimmer" || msg.starts_with("/glimmer ") {
                let args = msg["/glimmer".len()..].trim();
                let label = state.set_glimmer(args);
                state.notice = Some((format!("glimmer: {}", label), "info".to_string()));
                persist_glimmer(state);
                return;
            }
            // /settings (alias /config) — consolidated UI settings.
            if msg.starts_with("/settings") || msg.starts_with("/config") {
                let start = if msg.starts_with("/settings") { "/settings" } else { "/config" };
                let rest = msg[start.len()..].trim();
                if rest.is_empty() {
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "\u{2699} glimmer: {}",
                        state.glimmer.label(),
                    )));
                    let fire_desc = if state.fire_on {
                        format!("on {}", state.fire_style.label())
                    } else {
                        "off".to_string()
                    };
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "\u{2699} fire: {}",
                        fire_desc,
                    )));
                    state.entries.push(state::ChatEntry::Diag(
                        "\u{2699} /settings {glimmer|fire} <args>".to_string(),
                    ));
                    return;
                }
                if let Some((key, args)) = rest.split_once(' ') {
                    match key {
                        "glimmer" => {
                            let label = state.set_glimmer(args);
                            state.notice =
                                Some((format!("glimmer: {}", label), "info".to_string()));
                            persist_glimmer(state);
                            return;
                        }
                        "fire" => {
                            let label = state.set_fire(args);
                            state.notice =
                                Some((format!("fire: {}", label), "info".to_string()));
                            persist_fire(state);
                            return;
                        }
                        _ => {}
                    }
                }
                state.entries.push(state::ChatEntry::Diag(
                    "usage: /settings {glimmer|fire} <args>".to_string(),
                ));
                return;
            }
            // /tools — tool verbosity config. Bare opens the modal picker;
            // args do CLI-style sets (never RPC).
            if msg == "/tools" || msg.starts_with("/tools ") {
                let args = msg["/tools".len()..].trim();
                tools_command(state, args);
                return;
            }
            // /help — sidebar glossary + keys + commands (never RPC).
            if msg == "/help" {
                state.help_open = true;
                state.help_sel = 0;
                return;
            }
            // Local fixed commands.
            match msg.as_str() {
                "/quit" | "/q" => {
                    state.should_quit = true;
                    return;
                }
                "/diag" => {
                    state.show_diag = !state.show_diag;
                    return;
                }
                "/clear" => {
                    state.entries.clear();
                    state.md_cache_clear();
                    return;
                }
                _ => {}
            }
            // Popup open → Enter completes to the selection (second Enter sends).
            if msg.starts_with('/') && !msg.contains(' ') {
                let word = msg.trim_start_matches('/').to_string();
                let cands = state.command_candidates_for(&msg);
                let exact = cands.iter().any(|c| c.name == word);
                let partial = cands.iter().any(|c| c.name.starts_with(&word));
                if !exact && partial {
                    state.input = msg;
                    state.cursor = state.input.chars().count();
                    state.complete_command();
                    state.cand_sel = 0;
                    return;
                }
                // Unknown command with a loaded registry → reject locally.
                // (pi passes unknown /text through to the model.)
                if !exact && !state.commands.is_empty() {
                    state
                        .entries
                        .push(state::ChatEntry::Diag(format!("unknown command: {}", msg)));
                    return;
                }
            }
            // Sending returns the view to the live tail — you came here to
            // watch the reply.
            state.scroll = 0;
            // Local RPC commands — dispatched directly, never prompt text.
            // Slash invocations don't echo into the chat log.
            if state.is_local_command(&msg) {
                match state.local_dispatch(&msg) {
                    Some(cmd) => {
                        let _ = bridge.request(cmd).await;
                    }
                    None => {
                        // /resume — open the picker (scan runs off-thread).
                        let cwd = state.cwd.clone();
                        let scan =
                            tokio::task::spawn_blocking(move || crate::rpc::sessions::list(&cwd))
                                .await;
                        match scan {
                            Ok(Ok(list)) if !list.is_empty() => {
                                state.resume_entries = list;
                                state.resume_sel = 0;
                                state.resume_open = true;
                            }
                            Ok(Ok(_)) => {
                                state
                                    .entries
                                    .push(state::ChatEntry::Diag("no sessions found".into()));
                            }
                            _ => {
                                state
                                    .entries
                                    .push(state::ChatEntry::Diag("session scan failed".into()));
                            }
                        }
                    }
                }
                return;
            }
            // Slash commands (extension-consumed) don't echo into the chat.
            let echo = !msg.starts_with('/');
            if state.is_streaming {
                if echo {
                    state.push_user(msg.clone());
                }
                let _ = bridge
                    .request(RpcCommand::Steer { id: None, message: msg })
                    .await;
            } else {
                if echo {
                    state.push_user(msg.clone());
                }
                // is_streaming flips on the agent_start event (Prompt path);
                // extension commands return without one — flag must not stick.
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: msg,
                        images: None,
                        streaming_behavior: None,
                    })
                    .await;
            }
        }
        KeyCode::Backspace => {
            if state.cursor > 0 {
                state.cursor -= 1;
                remove_char_at(&mut state.input, state.cursor);
                state.cand_sel = 0;
            }
        }
        KeyCode::Delete => {
            if state.cursor < state.input.chars().count() {
                remove_char_at(&mut state.input, state.cursor);
                state.cand_sel = 0;
            }
        }
        KeyCode::Left => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            if state.cursor < state.input.chars().count() {
                state.cursor += 1;
            }
        }
        // Home/End: cursor movement while typing; history jump when the
        // input is empty (oldest / live tail).
        KeyCode::Home => {
            if state.input.is_empty() {
                state.scroll_home();
            } else {
                state.cursor = 0;
            }
        }
        KeyCode::End => {
            if state.input.is_empty() {
                state.scroll_end();
            } else {
                state.cursor = state.input.chars().count();
            }
        }
        _ => {
            if let KeyCode::Char(c) = key.code {
                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
                    insert_char_at(&mut state.input, state.cursor, c);
                    state.cursor += 1;
                    state.cand_sel = 0;
                }
            }
        }
    }
}

async fn handle_mouse(m: crossterm::event::MouseEvent, state: &mut AppState) {
    use crossterm::event::{MouseButton, MouseEventKind};
    match m.kind {
        MouseEventKind::ScrollUp => {
            state.clear_selection();
            state.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            state.clear_selection();
            state.scroll_down(3);
        }
        // Text selection (owner request 2026-09-03): left-drag across the
        // transcript anchors a row range; ^c copies it. Plain click clears.
        MouseEventKind::Down(MouseButton::Left) => {
            state.clear_selection();
            if let Some(a) = state.last_chat_area {
                if m.row >= a.y && m.row < a.y + a.height {
                    state.sel_anchor = Some((m.column, m.row));
                    state.sel_head = Some((m.column, m.row));
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if state.sel_anchor.is_some() {
                state.sel_head = Some((m.column, m.row));
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            // A click without a drag was never a selection intent.
            if state.sel_anchor == state.sel_head {
                state.clear_selection();
            }
        }
        _ => {}
    }
}

/// Minimal base64 (RFC 4648, padded) — avoids a crate for one use.
fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Copy to the system clipboard: shell tools first (wl-copy / xclip /
/// xsel — real clipboards), then the OSC52 escape sequence as fallback
/// for terminals that honor it (kitty, alacritty, wezterm, foot, tmux).
/// Returns what worked, for the notice line.
fn copy_to_clipboard(text: &str) -> &'static str {
    use std::io::Write;
    use std::process::{Command, Stdio};
    const TOOLS: [(&str, [&str; 2]); 3] = [
        ("wl-copy", ["--", ""]),
        ("xclip", ["-selection", "clipboard"]),
        ("xsel", ["--clipboard", "--input"]),
    ];
    for (bin, args) in TOOLS {
        let args: Vec<&str> = args.into_iter().filter(|a| !a.is_empty()).collect();
        if let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut si) = child.stdin.take() {
                let _ = si.write_all(text.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return bin;
            }
        }
    }
    // OSC52 — fire-and-forget; capped so huge texts don't choke terminals.
    let bytes = text.as_bytes();
    if bytes.len() <= 100_000 {
        let mut out = io::stdout();
        let _ = write!(out, "\x1b]52;c;{}\x07", b64(bytes));
        let _ = out.flush();
        return "osc52";
    }
    "nowhere"
}

fn insert_char_at(s: &mut String, idx: usize, c: char) {
    let byte = s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len());
    s.insert(byte, c);
}

fn remove_char_at(s: &mut String, idx: usize) {
    let byte = s.char_indices().nth(idx).map(|(b, _)| b).unwrap_or(s.len());
    s.remove(byte);
}

/// Extract text from a content field (string or array of blocks).
fn content_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Returns true when the caller should re-request state + transcript
/// (session switched or replaced).
fn handle_response(state: &mut AppState, resp: &crate::rpc::protocol::RpcResponse) -> bool {
    if resp.success != Some(true) {
        if let Some(err) = &resp.error {
            state.entries.push(state::ChatEntry::Diag(format!(
                "rpc {} failed: {}",
                resp.command.as_deref().unwrap_or("?"),
                err
            )));
        }
        return false;
    }
    let data = match &resp.data {
        Some(d) => d,
        None => return false,
    };
    match resp.command.as_deref() {
        Some("switch_session") | Some("new_session") => {
            // Session replaced — clear local transcript + widgets, rehydrate.
            state.entries.clear();
            state.md_cache_clear();
            state.widgets.clear();
            state.input.clear();
            state.cursor = 0;
            state.resume_entries.clear();
            return true;
        }
        Some("get_state") => {
            state.apply_state(
                data.get("model").and_then(|m| serde_json::from_value(m.clone()).ok()),
                data.get("thinkingLevel").and_then(|v| v.as_str()).map(String::from),
                data.get("isStreaming").and_then(|v| v.as_bool()).unwrap_or(false),
                data.get("isCompacting").and_then(|v| v.as_bool()).unwrap_or(false),
                data.get("sessionId").and_then(|v| v.as_str()).map(String::from),
                data.get("sessionName").and_then(|v| v.as_str()).map(String::from),
                data.get("messageCount").and_then(|v| v.as_u64()),
            );
        }
        Some("get_messages") => {
            // Full-fidelity replay (owner bug 2026-09-03: "resumed a
            // session, but it seems to have lost part of it" — the old
            // handler replayed user + assistant text ONLY, so every tool
            // call — most of a coding session's substance — vanished from
            // the transcript). Walk content blocks: assistant text blocks
            // become Assistant entries, toolCall blocks become completed
            // Tool entries, toolResult messages fill their outputs.
            if let Some(msgs) = data.get("messages").and_then(|m| m.as_array()) {
                for m in msgs {
                    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    match role {
                        "user" => {
                            let text = content_text(m.get("content").unwrap_or(&serde_json::Value::Null));
                            if !text.is_empty() {
                                state.push_user(text);
                            }
                        }
                        "assistant" => {
                            let empty = serde_json::Value::Null;
                            let content = m.get("content").unwrap_or(&empty);
                            if let Some(blocks) = content.as_array() {
                                for b in blocks {
                                    match b.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                                        "text" => {
                                            if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                                                if !t.trim().is_empty() {
                                                    state.entries.push(state::ChatEntry::Assistant(t.to_string()));
                                                }
                                            }
                                        }
                                        "toolCall" => {
                                            let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                                            let id = b.get("id").and_then(|v| v.as_str())
                                                .or_else(|| b.get("toolCallId").and_then(|v| v.as_str()));
                                            let args = b
                                                .get("arguments")
                                                .or_else(|| b.get("args"))
                                                .cloned()
                                                .unwrap_or(serde_json::Value::Null);
                                            state.tool_start(id, name, &args);
                                            // NOT completed here — the paired
                                            // toolResult message fills output;
                                            // end-of-record sweep closes any
                                            // that never got one.
                                        }
                                        _ => {} // thinking blocks stay private to the record
                                    }
                                }
                            } else {
                                let text = content_text(content);
                                if !text.is_empty() {
                                    state.entries.push(state::ChatEntry::Assistant(text));
                                }
                            }
                        }
                        "toolResult" => {
                            let id = m.get("toolCallId").and_then(|v| v.as_str())
                                .or_else(|| m.get("id").and_then(|v| v.as_str()));
                            let name = m.get("toolName").and_then(|v| v.as_str()).unwrap_or("");
                            let text = content_text(m.get("content").unwrap_or(&serde_json::Value::Null));
                            let is_err = m.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
                            // id-matched completion; name only when the record
                            // lacks ids (legacy sessions).
                            state.tool_end(id, name, &text, is_err);
                        }
                        _ => {}
                    }
                }
                // History ends where it ends: any tool still "running" has
                // no recorded result — close it (outcome unknown → not an
                // error). Live streaming can't interleave here: replay only
                // happens at startup / session switch.
                for e in state.entries.iter_mut().rev() {
                    if let state::ChatEntry::Tool { running, .. } = e {
                        if *running {
                            *running = false;
                        }
                    }
                }
            }
        }
        Some("get_commands") => {
            if let Some(cmds) = data.get("commands").and_then(|c| c.as_array()) {
                let mut parsed: Vec<SlashCommand> = cmds
                    .iter()
                    .filter_map(|c| serde_json::from_value::<SlashCommand>(c.clone()).ok())
                    .collect();
                parsed.sort_by(|a, b| a.name.cmp(&b.name));
                state.commands = parsed;
            }
        }
        _ => {}
    }
    false
}

fn handle_event(state: &mut AppState, event_type: &str, fields: &serde_json::Value) {
    match event_type {
        "agent_start" => state.is_streaming = true,
        "agent_end" | "agent_settled" => {
            state.is_streaming = false;
        }
        "message_update" => {
            if let Some(ae) = fields.get("assistantMessageEvent") {
                let et = ae.get("type").and_then(|v| v.as_str()).unwrap_or("");
                match et {
                    "text_delta" => {
                        if let Some(d) = ae.get("delta").and_then(|v| v.as_str()) {
                            state.push_text_delta(d);
                        }
                    }
                    "thinking_delta" => {
                        if let Some(d) = ae.get("delta").and_then(|v| v.as_str()) {
                            state.push_thinking_delta(d);
                        }
                    }
                    _ => {}
                }
            }
        }
        "message_end" => {
            let msg = fields.get("message");
            let role = msg
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            if role == "assistant" {
                if let Some(content) = msg.and_then(|m| m.get("content")) {
                    let text = content_text(content);
                    let have_streamed = matches!(&state.entries.last(), Some(e)
                        if matches!(e, state::ChatEntry::Assistant(t) if !t.is_empty())
                            || matches!(e, state::ChatEntry::Thinking(t) if !t.is_empty()));
                    if !text.is_empty() && !have_streamed {
                        state.entries.push(state::ChatEntry::Assistant(text));
                    }
                }
            }
        }
        "tool_execution_start" => {
            let name = fields.get("toolName").and_then(|v| v.as_str()).unwrap_or("tool");
            let tool_call_id = fields.get("toolCallId").and_then(|v| v.as_str());
            let args = fields.get("args").cloned().unwrap_or(serde_json::Value::Null);
            state.tool_start(tool_call_id, name, &args);
        }
        "tool_execution_update" => {
            let name = fields.get("toolName").and_then(|v| v.as_str()).unwrap_or("tool");
            let tool_call_id = fields.get("toolCallId").and_then(|v| v.as_str());
            // partialResult can be a string or an object with a "content" array.
            let text = fields
                .get("partialResult")
                .and_then(|v| v.get("content"))
                .and_then(|c| {
                    // Extract text from content array: [{type:"text", text:"..."}]
                    if let Some(arr) = c.as_array() {
                        Some(
                            arr.iter()
                                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    } else {
                        c.as_str().map(String::from)
                    }
                })
                .or_else(|| fields.get("partialResult").and_then(|v| v.as_str()).map(String::from))
                .unwrap_or_default();
            state.tool_update(tool_call_id, name, &text);
        }
        "tool_execution_end" => {
            let name = fields.get("toolName").and_then(|v| v.as_str()).unwrap_or("tool");
            let tool_call_id = fields.get("toolCallId").and_then(|v| v.as_str());
            let is_err = fields.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
            // Extract result text from result.content[]
            let result_text = fields
                .get("result")
                .and_then(|r| r.get("content"))
                .and_then(|c| {
                    if let Some(arr) = c.as_array() {
                        Some(
                            arr.iter()
                                .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    } else {
                        c.as_str().map(String::from)
                    }
                })
                .or_else(|| {
                    fields
                        .get("result")
                        .and_then(|r| r.get("output"))
                        .and_then(|o| o.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            state.tool_end(tool_call_id, name, &result_text, is_err);
        }
        "extension_error" => {
            let path = fields.get("extensionPath").and_then(|v| v.as_str()).unwrap_or("?");
            let err = fields.get("error").and_then(|v| v.as_str()).unwrap_or("?");
            state.entries.push(state::ChatEntry::Diag(format!(
                "extension {}: {}",
                path.rsplit('/').next().unwrap_or(path),
                err
            )));
        }
        "auto_retry_start" => {
            let attempt = fields.get("attempt").and_then(|v| v.as_u64()).unwrap_or(0);
            let reason = fields.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            state.entries.push(state::ChatEntry::Diag(format!(
                "retry #{} {}",
                attempt, reason
            )));
            state.is_streaming = true;
        }
        "auto_retry_end" => {
            state.entries.push(state::ChatEntry::Diag("retry settled".to_string()));
        }
        "compaction_start" => state.is_compacting = true,
        "compaction_end" => state.is_compacting = false,
        "model_select" => {
            if let Ok(m) = serde_json::from_value::<crate::rpc::protocol::Model>(
                fields.get("model").cloned().unwrap_or(serde_json::Value::Null),
            ) {
                state.model = Some(m);
            }
        }
        "session_start" => {
            state.entries.clear();
            state.md_cache_clear();
        }
        _ => {}
    }
}

async fn handle_ui_request(
    state: &mut AppState,
    method: &str,
    fields: &serde_json::Value,
    bridge: &mut RpcBridge,
) {
    match method {
        "setWidget" => {
            let key = fields.get("widgetKey").and_then(|v| v.as_str()).unwrap_or("widget");
            let lines: Vec<String> = fields
                .get("widgetLines")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            state.set_widget(key, lines);
        }
        "setTitle" => {
            if let Some(t) = fields.get("title").and_then(|v| v.as_str()) {
                state.title = t.to_string();
                let _ = execute!(io::stdout(), crossterm::terminal::SetTitle(t));
            }
        }
        "notify" => {
            let msg = fields.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let ntype = fields.get("notifyType").and_then(|v| v.as_str()).unwrap_or("info");
            if ntype == "error" || ntype == "warning" {
                state.entries.push(state::ChatEntry::Diag(msg.to_string()));
            } else {
                state.notice = Some((msg.to_string(), ntype.to_string()));
            }
        }
        "setStatus" => {
            let key = fields.get("statusKey").and_then(|v| v.as_str()).unwrap_or("");
            let text = fields.get("statusText").and_then(|v| v.as_str()).unwrap_or("");
            if key == "engine" {
                // vitriol engine status surfaces as a notice line
                state.notice = Some((text.to_string(), "info".to_string()));
            }
        }
        "select" | "confirm" | "input" | "editor" => {
            // Interactive dialogs not yet supported — cancel so the agent
            // never hangs waiting for a response that would never come.
            if let Some(id) = fields.get("id").and_then(|v| v.as_str()) {
                let payload = serde_json::json!({
                    "type": "extension_ui_response", "id": id, "cancelled": true
                });
                let _ = bridge.send_raw(&payload.to_string()).await;
                state.entries.push(state::ChatEntry::Diag(format!(
                    "{} dialog auto-cancelled (unsupported)",
                    method
                )));
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_rfc4648_vectors() {
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"foob"), "Zm9vYg==");
        assert_eq!(b64(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
        assert_eq!(b64(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn selection_rows_order_agnostic() {
        let mut s = AppState::default();
        assert_eq!(s.selection_rows(), None);
        s.sel_anchor = Some((5, 10));
        s.sel_head = Some((2, 3));
        assert_eq!(s.selection_rows(), Some((3, 10)));
        s.sel_head = Some((7, 15));
        assert_eq!(s.selection_rows(), Some((10, 15)));
        s.clear_selection();
        assert_eq!(s.selection_rows(), None);
    }

    #[test]
    fn extract_selection_rows_map_to_lines() {
        use ratatui::layout::Rect;
        let mut s = AppState::default();
        s.push_user("alpha".into());
        s.push_user("beta".into());
        s.push_user("gamma".into());
        // Viewport tall enough for every line (6): "☿ alpha", "", "☿ beta",
        // "", "☿ gamma", "" — offset-from-bottom anchoring with scroll 0
        // maps screen rows 1:1 onto lines.
        s.last_chat_area = Some(Rect { x: 0, y: 0, width: 40, height: 6 });
        s.sel_anchor = Some((0, 0));
        s.sel_head = Some((30, 0));
        let t = s.extract_selection().unwrap();
        assert_eq!(t, "☿ alpha");
        // Drag down two rows: alpha + blank + beta head.
        s.sel_head = Some((30, 2));
        let t = s.extract_selection().unwrap();
        assert!(t.starts_with("☿ alpha"), "got {:?}", t);
        assert!(t.contains("☿ beta"), "got {:?}", t);
    }

    #[test]
    fn last_assistant_picks_latest_nonempty() {
        let mut s = AppState::default();
        assert_eq!(s.last_assistant(), None);
        s.entries.push(state::ChatEntry::Assistant("first".into()));
        s.entries.push(state::ChatEntry::Assistant("   ".into())); // whitespace-only skipped
        s.entries.push(state::ChatEntry::User("hi".into()));
        s.entries.push(state::ChatEntry::Assistant("second".into()));
        assert_eq!(s.last_assistant().as_deref(), Some("second"));
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::rpc::protocol::RpcResponse;

    /// Owner bug 2026-09-03: "resumed a session, but it seems to have lost
    /// part of it" — the replay must restore tool calls and their outputs,
    /// not just the prose.
    #[test]
    fn resume_replay_restores_tool_history() {
        let mut s = AppState::default();
        let resp: RpcResponse = serde_json::from_value(serde_json::json!({
            "type": "response",
            "command": "get_messages",
            "success": true,
            "data": { "messages": [
                { "role": "user", "content": "fix the flux capacitor" },
                { "role": "assistant", "content": [
                    { "type": "thinking", "thinking": "hmm" },
                    { "type": "toolCall", "id": "tc1", "name": "write",
                      "arguments": { "path": "a.rs" } },
                    { "type": "text", "text": "Done." }
                ]},
                { "role": "toolResult", "toolCallId": "tc1", "toolName": "write",
                  "content": [{ "type": "text", "text": "wrote 12 lines" }],
                  "isError": false }
            ]}
        }))
        .unwrap();
        let _ = handle_response(&mut s, &resp);
        let mut users = 0;
        let mut assistants = 0;
        let mut tools: Vec<(bool, bool, Vec<String>)> = Vec::new();
        for e in &s.entries {
            match e {
                state::ChatEntry::User(t) => {
                    users += 1;
                    assert_eq!(t, "fix the flux capacitor");
                }
                state::ChatEntry::Assistant(t) => {
                    assistants += 1;
                    assert_eq!(t, "Done.");
                }
                state::ChatEntry::Tool { running, error, output, .. } => {
                    tools.push((*running, *error, output.clone()));
                }
                _ => {}
            }
        }
        assert_eq!(users, 1);
        assert_eq!(assistants, 1, "thinking block stays out of the transcript");
        assert_eq!(tools.len(), 1, "historic tool call restored");
        assert!(!tools[0].0, "historic tool is complete, not spinning");
        assert!(!tools[0].1);
        assert_eq!(tools[0].2, vec!["wrote 12 lines"], "output replayed");
    }
}
