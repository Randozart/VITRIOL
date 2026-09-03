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
        let mut words = src.split_whitespace();
        if let Some(w) = words.next() {
            match w {
                "off" | "0" => state.fire_on = false,
                "on" | "1" => state.fire_on = true,
                _ => {}
            }
        }
        if let Some(w) = words.next() {
            if let Some(style) = crate::fire::FireStyle::parse(w) {
                state.fire_style = style;
            }
        } else if let Some(style) = crate::fire::FireStyle::parse(src.trim()) {
            // A lone style word means "on, this voice".
            state.fire_style = style;
            state.fire_on = true;
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
    let _ = std::fs::write(fire_path(), format!("{}\n", line));
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
    // GUARANTEED quit: Ctrl+Q always, regardless of streaming state.
    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.should_quit = true;
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
        KeyCode::Esc => {
            if state.is_streaming {
                let _ = bridge.request(RpcCommand::Abort { id: None }).await;
            } else {
                state.should_quit = true;
            }
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if state.is_streaming {
                let _ = bridge.request(RpcCommand::Abort { id: None }).await;
            } else {
                state.should_quit = true;
            }
        }
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::F(9) => {
            state.show_diag = !state.show_diag;
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
    use crossterm::event::MouseEventKind;
    match m.kind {
        MouseEventKind::ScrollUp => state.scroll_up(3),
        MouseEventKind::ScrollDown => state.scroll_down(3),
        _ => {} // clicks / drags are not TUI concerns
    }
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
            if let Some(msgs) = data.get("messages").and_then(|m| m.as_array()) {
                for m in msgs {
                    let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
                    let text = content_text(m.get("content").unwrap_or(&serde_json::Value::Null));
                    if text.is_empty() {
                        continue;
                    }
                    match role {
                        "user" => state.push_user(text),
                        "assistant" => state.entries.push(state::ChatEntry::Assistant(text)),
                        _ => {}
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
            let args = fields.get("args").cloned().unwrap_or(serde_json::Value::Null);
            state.tool_start(name, &args);
        }
        "tool_execution_end" => {
            let name = fields.get("toolName").and_then(|v| v.as_str()).unwrap_or("tool");
            let is_err = fields.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
            state.tool_end(name, is_err);
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
