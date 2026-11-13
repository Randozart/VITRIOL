pub mod ansi;
pub mod cache;
pub mod caduceus;
pub mod cost;
pub mod engine_config;
pub mod layout;
pub mod paste;
pub mod review;
pub mod state;

use std::io::{self, IsTerminal};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_lite::StreamExt;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::rpc::bridge::{RpcBridge, WireMessage};
use crate::rpc::protocol::{RpcCommand, SlashCommand};
use crate::session_guard;

use self::layout::render;
use self::paste as tui_paste;
use self::state::AppState;

pub async fn run(bridge: &mut RpcBridge, cwd: std::path::PathBuf) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    // Mouse capture: wheel scrolling through session history (owner request
    // 2026-09-02). Trade-off: terminals usually need Shift+drag to select
    // text while captured.
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::default();
    state.perm_mode = crate::envfile::read_permissions_mode();
    state.tool_watchdog_secs = env_secs("OFFICINA_TOOL_WATCHDOG_SECS", state.tool_watchdog_secs);
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
    if std::env::var("OFFICINA_FIRE")
        .map(|v| v == "0")
        .unwrap_or(false)
    {
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
    let tint = if state.fire_tint {
        "tint on"
    } else {
        "tint off"
    };
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
                    state.notice =
                        Some((format!("tools default: {}", m.label()), "info".to_string()));
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

/// ^o (codex-qol stage 9): pull the clipboard image into the composer.
/// Probes wl-paste (Wayland) or xclip (X11), validates magic bytes, enforces
/// the caps; every failure is a notice (I5).
fn attach_clipboard_image(state: &mut AppState) {
    if state.pending_images.len() >= tui_paste::MAX_IMAGES {
        state.notice = Some((
            format!("images: cap is {} — alt+o clears", tui_paste::MAX_IMAGES),
            "warning".into(),
        ));
        return;
    }
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    for (cmd, args, mime) in tui_paste::clipboard_candidates(wayland) {
        let out = std::process::Command::new(&cmd).args(&args).output();
        let Ok(out) = out else { continue };
        if !out.status.success() || out.stdout.is_empty() {
            continue;
        }
        if !tui_paste::magic_ok(&out.stdout, &mime) {
            continue;
        }
        if out.stdout.len() > tui_paste::MAX_IMAGE_BYTES {
            state.notice = Some((
                format!("image too large ({} KiB > cap)", out.stdout.len() / 1024),
                "warning".into(),
            ));
            return;
        }
        match tui_paste::save_temp(&out.stdout, &mime) {
            Ok(path) => {
                let kb = out.stdout.len() / 1024;
                let n = state.pending_images.len() + 1;
                state.attach_image(path, mime.clone());
                state.notice = Some((
                    format!(
                        "attached image {n}/{} · {} · {} KiB",
                        tui_paste::MAX_IMAGES,
                        mime,
                        kb
                    ),
                    "info".into(),
                ));
            }
            Err(e) => {
                state.notice = Some((format!("image save failed ({e})"), "warning".into()));
            }
        }
        return;
    }
    state.notice = Some((
        "no image in the clipboard (text paste works as usual)".into(),
        "info".into(),
    ));
}

/// Encode + drop the pending images for a send (I5: failures are notices;
/// the pending set is cleared either way so a bad image cannot stick).
fn images_for_send(state: &mut AppState) -> Option<Vec<crate::rpc::protocol::ImageContent>> {
    if state.pending_images.is_empty() {
        return None;
    }
    let mut images = Vec::new();
    for img in &state.pending_images {
        match tui_paste::encode_image(&img.path, &img.mime) {
            Ok(content) => images.push(content),
            Err(e) => {
                state.notice = Some((format!("image dropped ({e})"), "warning".into()));
            }
        }
    }
    state.clear_images();
    if images.is_empty() {
        None
    } else {
        Some(images)
    }
}

/// Alt+E (codex-qol stage 4): stage the composer draft for $VISUAL/$EDITOR.
/// The original draft is kept in the request, so a failed spawn/reload
/// restores it verbatim (I4).
fn request_editor(state: &mut AppState) {
    if state.editor_suspend.is_some() {
        return;
    }
    let editor = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_string());
    let mut argv: Vec<String> = editor.split_whitespace().map(String::from).collect();
    if argv.is_empty() {
        state.notice = Some((
            "editor: empty $VISUAL/$EDITOR — draft kept".into(),
            "warning".into(),
        ));
        return;
    }
    let tmp = std::env::temp_dir().join(format!("officina-compose-{}.md", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &state.input) {
        state.notice = Some((
            format!("editor: temp write failed ({e}) — draft kept"),
            "warning".into(),
        ));
        return;
    }
    argv.push(tmp.to_string_lossy().to_string());
    state.editor_suspend = Some((argv, tmp, state.input.clone()));
}

/// Local date as YYYY-MM-DD without chrono: UTC day math (ledger stamps are
/// UTC, so the modal matches the ledger's own day grouping).
fn chrono_day() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86_400);
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let ml = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    let mut dd = d;
    while dd >= ml[m] {
        dd -= ml[m];
        m += 1;
    }
    format!("{y}-{m:02}-{:02}", dd + 1)
}

/// Hand the terminal to an external interactive flow (signup wizard, OAuth
/// login), then restore raw mode + alternate screen. Non-TTY refuses: a failed
/// resume is worse than no suspend, and the page falls back to printing the
/// command.
fn suspend_external(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    argv: &[String],
) -> io::Result<()> {
    if argv.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty argv"));
    }
    if !io::stdout().is_terminal() {
        return Err(io::Error::new(io::ErrorKind::Other, "not a TTY"));
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        crossterm::event::DisableMouseCapture,
        LeaveAlternateScreen,
        crossterm::cursor::Show
    )?;
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status();
    enable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    terminal.clear()?;
    status.map(|_| ())
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
                        if let Some(argv) = state.caduceus_suspend.take() {
                            match suspend_external(terminal, &argv) {
                                Ok(()) => {
                                    if let Some(page) = state.caduceus.as_mut() {
                                        page.reload();
                                    }
                                }
                                Err(e) => {
                                    let cmd = argv.join(" ");
                                    if let Some(page) = state.caduceus.as_mut() {
                                        page.note = Some(format!("suspend failed ({e}) · run: {cmd}"));
                                    }
                                }
                            }
                            last_frame = std::time::Instant::now();
                        }
                        if let Some((argv, tmp, original)) = state.editor_suspend.take() {
                            let spawn_ok = suspend_external(terminal, &argv).is_ok();
                            let mut note: Option<(String, String)> = None;
                            match std::fs::read_to_string(&tmp) {
                                Ok(text) => {
                                    let edited =
                                        text.strip_suffix('\n').unwrap_or(&text).to_string();
                                    if !spawn_ok {
                                        note = Some((
                                            "editor failed — draft kept".into(),
                                            "warning".into(),
                                        ));
                                    } else if edited == original {
                                        note = Some(("editor: no change".into(), "info".into()));
                                    } else {
                                        state.cursor = edited.chars().count();
                                        let n = state.cursor;
                                        state.input = edited;
                                        note = Some((
                                            format!("editor: draft loaded ({n} chars)"),
                                            "info".into(),
                                        ));
                                    }
                                }
                                Err(e) => {
                                    note = Some((
                                        format!("editor: reload failed ({e}) — draft kept"),
                                        "warning".into(),
                                    ));
                                }
                            }
                            let _ = std::fs::remove_file(&tmp);
                            if let Some(n) = note {
                                state.notice = Some(n);
                            }
                            last_frame = std::time::Instant::now();
                        }
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
                        let auto_continue = handle_event(state, &event.event_type, &event.fields);
                        if auto_continue && state.queued_prompts.is_empty() {
                            // Guarded auto-continue (owner choice B, plan
                            // officina-compaction-continue-2026-09-03): a
                            // threshold compaction ends the run by design
                            // (pi: "user continues manually") — we pick the
                            // work back up, once per compaction.
                            let _ = bridge
                                .request(RpcCommand::Prompt {
                                    id: None,
                                    message: CONTINUE_PROMPT.to_string(),
                                    images: None,
                                    streaming_behavior: None,
                                })
                                .await;
                            state.notice = Some((
                                "auto-continued after compaction".to_string(),
                                "info".to_string(),
                            ));
                        }
                        // Refresh the context fill for the bottom status row.
                        // pi's wire agent_end does NOT carry contextUsage (only
                        // {type, messages, willRetry}); the number lives in
                        // getSessionStats, surfaced via the get_session_stats
                        // RPC. Fire it on every turn end so the row tracks the
                        // live fill. agent_settled would double-fetch, so only
                        // agent_end triggers this.
                        if event.event_type == "agent_end" {
                            let _ = bridge
                                .request(RpcCommand::GetSessionStats { id: None })
                                .await;
                        }
                        // Deliver a prompt that was typed while a tool was
                        // still running. It was echoed already, so send it
                        // directly rather than re-pushing the user transcript.
                        // Only on agent_end: agent_settled for the previous
                        // run can race a Prompt we just sent.
                        if event.event_type == "agent_end"
                            && !state.is_streaming
                            && !state.queued_prompts.is_empty()
                        {
                            let message = state.queued_prompts.remove(0);
                            let _ = bridge
                                .request(RpcCommand::Prompt {
                                    id: None,
                                    message,
                                    images: None,
                                    streaming_behavior: None,
                                })
                                .await;
                            state.notice = Some((
                                "delivered queued message".to_string(),
                                "info".to_string(),
                            ));
                        }
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
                state.probe_liveness(bridge.pi_pid());
                maybe_tool_watchdog(state, bridge).await;
            }
        }
    }

    Ok(())
}

/// /engine + the Vitriol tab share this control path (systemd unit only —
/// the polkit rule scopes randozart to the two vitriol units; no sudo).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineAction {
    Start,
    Stop,
    Restart,
    Status,
    Help,
}

pub fn parse_engine_action(msg: &str) -> EngineAction {
    let arg = msg
        .strip_prefix("/engine")
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match arg.as_str() {
        "start" => EngineAction::Start,
        "stop" => EngineAction::Stop,
        "restart" => EngineAction::Restart,
        "status" | "" => EngineAction::Status,
        _ => EngineAction::Help,
    }
}

async fn engine_ctl(state: &mut AppState, action: EngineAction) {
    match action {
        EngineAction::Status => {
            let out = tokio::process::Command::new("systemctl")
                .args(["is-active", "vitriol-server.service"])
                .output()
                .await;
            let state_txt = match out {
                Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
                Err(e) => format!("systemctl failed: {e}"),
            };
            let extra = if state_txt == "active" {
                " · config: your live daily driver (unit as-is)"
            } else {
                " · start it with /engine start"
            };
            state.entries.push(state::ChatEntry::Diag(format!(
                "engine: {state_txt}{extra}"
            )));
        }
        EngineAction::Start => {
            state.entries.push(state::ChatEntry::Diag(
                "engine: starting vitriol-server.service (daily-driver config; the unit emits the VITRIOL-FINGERPRINT — watch the journal)".into(),
            ));
            let out = tokio::process::Command::new("systemctl")
                .args(["start", "vitriol-server.service"])
                .output()
                .await;
            let ok = matches!(&out, Ok(o) if o.status.success());
            let verify = tokio::process::Command::new("systemctl")
                .args(["is-active", "vitriol-server.service"])
                .output()
                .await;
            let state_txt = match verify {
                Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
                Err(_) => "unknown".to_string(),
            };
            if ok && state_txt == "active" {
                state.entries.push(state::ChatEntry::Diag(
                    "engine: vitriol-server.service is active — model loading; check readiness with /engine status or /cost".into(),
                ));
            } else {
                let err = match out {
                    Ok(o) => String::from_utf8_lossy(&o.stderr).trim().to_string(),
                    Err(e) => format!("{e}"),
                };
                state.entries.push(state::ChatEntry::Diag(format!(
                    "engine: start FAILED (state {state_txt}) — journalctl -u vitriol-server: {}",
                    if err.is_empty() {
                        "see journal".to_string()
                    } else {
                        err.chars().take(200).collect()
                    }
                )));
            }
        }
        EngineAction::Stop => {
            let out = tokio::process::Command::new("systemctl")
                .args(["stop", "vitriol-server.service"])
                .output()
                .await;
            state.entries.push(state::ChatEntry::Diag(format!(
                "engine: stop {}",
                matches!(&out, Ok(o) if o.status.success())
                    .then_some("ok — warm KV slot mirror persists for the next start")
                    .unwrap_or("FAILED — see journal")
            )));
        }
        EngineAction::Restart => {
            state.entries.push(state::ChatEntry::Diag(
                "engine: restarting vitriol-server.service (warm KV slot mirror restores the session on the next turn)".into(),
            ));
            let out = tokio::process::Command::new("systemctl")
                .args(["restart", "vitriol-server.service"])
                .output()
                .await;
            let ok = matches!(&out, Ok(o) if o.status.success());
            let verify = tokio::process::Command::new("systemctl")
                .args(["is-active", "vitriol-server.service"])
                .output()
                .await;
            let state_txt = match verify {
                Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
                Err(_) => "unknown".to_string(),
            };
            state.entries.push(state::ChatEntry::Diag(format!(
                "engine: restart {} (state {state_txt})",
                if ok { "done" } else { "FAILED — see journal" }
            )));
        }
        EngineAction::Help => {
            state.entries.push(state::ChatEntry::Diag(
                "usage: /engine [start|stop|restart|status] — the unit runs your live config (daily driver); config changes stay with `vitriol config load <name>` outside the TUI".into(),
            ));
        }
    }
}

async fn handle_caduceus_key(key: KeyEvent, state: &mut AppState, bridge: &mut RpcBridge) {
    use crate::tui::caduceus::Tab;
    let mut close = false;
    let mut reload = false;
    let mut suspend: Option<Vec<String>> = None;
    {
        let Some(page) = state.caduceus.as_mut() else {
            return;
        };
        if page.filtering {
            match key.code {
                KeyCode::Esc => {
                    page.filtering = false;
                    page.filter.clear();
                    page.sel = 0;
                }
                KeyCode::Enter => {
                    page.filtering = false;
                    page.clamp_sel();
                }
                KeyCode::Backspace => {
                    page.filter.pop();
                    page.sel = 0;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    page.filter.push(c);
                    page.sel = 0;
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc => {
                if page.detail {
                    page.detail = false;
                } else if !page.filter.is_empty() {
                    page.filter.clear();
                    page.sel = 0;
                } else {
                    close = true;
                }
            }
            KeyCode::Char('q') => close = true,
            KeyCode::Tab => page.set_tab(page.tab.next()),
            KeyCode::BackTab => page.set_tab(page.tab.prev()),
            KeyCode::Char(c @ '1'..='5') => {
                if let Some(tab) = crate::tui::caduceus::Tab::from_number((c as u8 - b'0') as usize)
                {
                    page.set_tab(tab);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if page.tab == Tab::Vitriol {
                    page.config_scroll = page.config_scroll.saturating_sub(1);
                } else {
                    page.move_sel(-1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if page.tab == Tab::Vitriol {
                    page.config_scroll += 1;
                } else {
                    page.move_sel(1);
                }
            }
            KeyCode::PageUp => {
                if page.tab == Tab::Vitriol {
                    page.config_scroll = page.config_scroll.saturating_sub(10);
                } else {
                    page.move_sel(-10);
                }
            }
            KeyCode::PageDown => {
                if page.tab == Tab::Vitriol {
                    page.config_scroll += 10;
                } else {
                    page.move_sel(10);
                }
            }
            KeyCode::Home | KeyCode::Char('g') => page.sel = 0,
            KeyCode::End | KeyCode::Char('G') => {
                page.sel = page.row_count().saturating_sub(1);
            }
            KeyCode::Char(' ') => {
                page.toggle();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                page.adjust(1);
            }
            KeyCode::Char('-') => {
                page.adjust(-1);
            }
            KeyCode::Enter => page.detail = true,
            KeyCode::Char('/') => page.filtering = true,
            KeyCode::Char('r') => reload = true,
            KeyCode::Char('h') => {
                page.toggle_hidden();
            }
            KeyCode::Char('H') => page.toggle_show_hidden(),
            KeyCode::Backspace | KeyCode::Delete => {
                page.clear_model_state();
            }
            // Vitriol tab: S/X/R engine controls with press-twice confirm
            // (stop/restart interrupt active turns).
            KeyCode::Char('S') | KeyCode::Char('X') | KeyCode::Char('R') => {
                if page.tab == Tab::Vitriol {
                    let want = match key.code {
                        KeyCode::Char('S') => "start",
                        KeyCode::Char('X') => "stop",
                        _ => "restart",
                    };
                    if page.engine_arm == Some(want) {
                        page.engine_arm = None;
                        let action = match want {
                            "start" => EngineAction::Start,
                            "stop" => EngineAction::Stop,
                            _ => EngineAction::Restart,
                        };
                        engine_ctl(state, action).await;
                        if let Some(pg) = state.caduceus.as_mut() {
                            pg.engine = Some(crate::tui::caduceus::probe_engine());
                        }
                    } else {
                        page.engine_arm = Some(want);
                        page.note = Some(format!(
                            "press {} again to confirm engine {}",
                            want.chars().next().unwrap_or('?'),
                            want
                        ));
                    }
                }
            }
            // a — quick key (extension dialog; key visible while typing).
            KeyCode::Char('a') => {
                if page.tab == Tab::Providers {
                    if let Some(p) = page.provider_view().get(page.sel) {
                        let id = p.id.clone();
                        let _ = bridge
                            .request(RpcCommand::Prompt {
                                id: None,
                                message: format!("/caduceus-add {id}"),
                                images: None,
                                streaming_behavior: None,
                            })
                            .await;
                        page.note = Some(format!("add key · {id} · follow the dialog"));
                    }
                }
            }
            // A — suspend the TUI and run the hidden-entry signup wizard.
            KeyCode::Char('A') => {
                if page.tab == Tab::Providers {
                    if let Some(p) = page.provider_view().get(page.sel) {
                        suspend = Some(vec![
                            "vitriol".to_string(),
                            "caduceus".to_string(),
                            "signup".to_string(),
                            "--only".to_string(),
                            p.id.clone(),
                        ]);
                        page.note = Some(format!("wizard · {}", p.id));
                    }
                }
            }
            // o — OAuth login (suspend) or open the signup page.
            KeyCode::Char('o') => {
                if page.tab == Tab::Providers {
                    if let Some(p) = page.provider_view().get(page.sel) {
                        let id = p.id.clone();
                        let oauth = p.category == "oauth"
                            || matches!(
                                p.method.as_str(),
                                "browser" | "device" | "import" | "social" | "callback" | "oauth"
                            );
                        let website = p.website.clone();
                        if oauth {
                            suspend = Some(vec![
                                "vitriol".to_string(),
                                "caduceus".to_string(),
                                "accounts".to_string(),
                                "oauth".to_string(),
                                "login".to_string(),
                                id.clone(),
                            ]);
                            page.note = Some(format!("oauth login · {id}"));
                        } else if !website.is_empty() {
                            let _ = std::process::Command::new("xdg-open").arg(&website).spawn();
                            page.note = Some(format!("opened {website}"));
                        } else {
                            page.note = Some("no website on record".to_string());
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if suspend.is_some() {
        state.caduceus_suspend = suspend;
    }
    if reload {
        if let Some(page) = state.caduceus.as_mut() {
            page.reload();
        }
    }
    if close {
        state.caduceus_close();
    }
}

async fn handle_key(key: KeyEvent, state: &mut AppState, bridge: &mut RpcBridge) {
    // Enhanced keyboard protocols report Press AND Release. Only Press/Repeat
    // are actions; a stray Esc key-release used to look like a second abort
    // (or dismiss a freshly opened dialog).
    if key.kind == KeyEventKind::Release {
        return;
    }
    // 2026-09-03 keymap (owner: "just so you don't accidentally quit"):
    // ^d REMOVED — the only quit paths are ^esc and /quit, both deliberate.
    // ^c copies (selection, else last reply) — never quits. ^q now toggles
    // the plan agent (2026-11-10). Esc aborts streaming but NEVER quits.

    // Pending extension dialog owns every key (trust, permissions, plan
    // questions, ask, rewind). It must be first or it can be shadowed.
    if !state.dialogs.is_empty() {
        handle_dialog_key(key, state, bridge).await;
        return;
    }

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

    // /permissions modal captures arrows/enter.
    if state.perm_modal_open {
        let n = AppState::PERM_MODES.len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => state.perm_modal_open = false,
            KeyCode::Up | KeyCode::Char('k') => {
                state.perm_modal_sel = state.perm_modal_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if state.perm_modal_sel + 1 < n {
                    state.perm_modal_sel += 1;
                }
            }
            KeyCode::Enter => {
                let mode = AppState::PERM_MODES[state.perm_modal_sel].0;
                state.set_permissions_mode(mode);
                state.perm_modal_open = false;
            }
            _ => {}
        }
        return;
    }

    // /tree listing captures keys while open: ↑↓ scroll the view, esc/q
    // closes. Fork addressing stays in the composer (`/fork <n>`) so the
    // printed numbers remain stable while the panel is up.
    if state.tree_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                state.note_phase(state::UiEvent::CloseModal);
                state.tree_open = false;
            }
            KeyCode::Up => state.tree_sel = state.tree_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.tree_sel + 1 < state.tree_lines.len() {
                    state.tree_sel += 1;
                }
            }
            KeyCode::Home => state.tree_sel = 0,
            KeyCode::End => {
                state.tree_sel = state.tree_lines.len().saturating_sub(1);
            }
            _ => {}
        }
        return;
    }

    // /cache manager captures keys while open: d deletes the highlighted
    // entry, w wipes every non-protected entry, r re-scans.
    if state.cache_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => state.cache_open = false,
            KeyCode::Up => state.cache_sel = state.cache_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.cache_sel + 1 < state.cache_entries.len() {
                    state.cache_sel += 1;
                }
            }
            KeyCode::Home => state.cache_sel = 0,
            KeyCode::End => {
                state.cache_sel = state.cache_entries.len().saturating_sub(1);
            }
            KeyCode::Char('d') | KeyCode::Delete => state.cache_delete_selected(),
            // Destructive wipe needs Shift (accidental lowercase w would
            // delete every regenerable multi-GB mirror).
            KeyCode::Char('p') => state.cache_wipe(Some("7d")),
            KeyCode::Char('W') => state.cache_wipe(None),
            KeyCode::Char('r') => state.cache_refresh(),
            _ => {}
        }
        return;
    }

    // /sessions — cross-project picker: type-to-filter, enter resumes
    // current-project rows; foreign rows print a launch hint (I4).
    if state.xsessions_open {
        match key.code {
            KeyCode::Esc => {
                if state.xsessions_filter.is_empty() {
                    state.xsessions_close();
                } else {
                    state.xsessions_filter.clear();
                    state.xsessions_sel = 0;
                }
            }
            KeyCode::Backspace => {
                state.xsessions_filter.pop();
                state.xsessions_sel = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                state.xsessions_filter.push(c);
                state.xsessions_sel = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.xsessions_sel = state.xsessions_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => state.xsessions_sel += 1,
            KeyCode::PageUp => state.xsessions_sel = state.xsessions_sel.saturating_sub(10),
            KeyCode::PageDown => state.xsessions_sel += 10,
            KeyCode::Enter => {
                let view = state.xsessions_view();
                if let Some(a) = view.get(state.xsessions_sel) {
                    let project = a.project.clone();
                    let own = project == state.cwd.to_string_lossy();
                    if !own {
                        let path = a.entry.path.display().to_string();
                        state.xsessions_close();
                        state.entries.push(state::ChatEntry::Diag(format!(
                            "foreign project — read-only (I4). Resume it with: cd {project} && officina   [session: {path}]"
                        )));
                        return;
                    }
                    let path = a.entry.path.to_string_lossy().to_string();
                    if let Some(note) = session_guard::peer_fenced_notice(&a.entry.path) {
                        state.entries.push(state::ChatEntry::Diag(note));
                        return;
                    }
                    state.note_phase(state::UiEvent::CloseModal);
                    state.xsessions_open = false;
                    if state.is_streaming {
                        state.entries.push(state::ChatEntry::Diag(
                            "dissolve first — session not switched".into(),
                        ));
                        return;
                    }
                    let _ = bridge
                        .request(RpcCommand::SwitchSession {
                            id: None,
                            session_path: path,
                        })
                        .await;
                } else {
                    state.xsessions_close();
                }
            }
            _ => {}
        }
        state.clamp_xsessions_sel();
        return;
    }

    // /answers — scrollable panel; esc/q closes, r reloads.
    if state.answers_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => state.answers_close(),
            KeyCode::Up | KeyCode::Char('k') => {
                state.answers_scroll = state.answers_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => state.answers_scroll += 1,
            KeyCode::PageUp => state.answers_scroll = state.answers_scroll.saturating_sub(10),
            KeyCode::PageDown => state.answers_scroll += 10,
            KeyCode::Char('r') => state.answers_open_modal(),
            _ => {}
        }
        return;
    }

    // /cost — read-only rows; esc/q closes.
    if state.cost_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                state.note_phase(state::UiEvent::CloseModal);
                state.cost_open = false;
            }
            _ => {}
        }
        return;
    }

    // /review-findings — severity-sorted list, enter toggles detail.
    if state.review_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => state.review_close(),
            KeyCode::Up | KeyCode::Char('k') => {
                state.review_sel = state.review_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => state.review_sel += 1,
            KeyCode::PageUp => state.review_sel = state.review_sel.saturating_sub(10),
            KeyCode::PageDown => state.review_sel += 10,
            KeyCode::Home | KeyCode::Char('g') => state.review_sel = 0,
            KeyCode::End | KeyCode::Char('G') => {
                state.review_sel = usize::MAX;
            }
            _ => {}
        }
        state.clamp_review_sel();
        return;
    }

    // /status — read-only snapshot; esc/q closes.
    if state.status_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => state.status_close(),
            _ => {}
        }
        return;
    }

    // /caduceus page — tabs, selection, policy edits (esc/q closes). Pending
    // extension dialogs own keys above, so inline key prompts win.
    if state.caduceus.is_some() {
        handle_caduceus_key(key, state, bridge).await;
        return;
    }

    // /model picker captures typing while open: characters filter the list,
    // arrows move, enter switches, esc clears the filter then closes.
    if state.models_open {
        match key.code {
            KeyCode::Esc => {
                if state.models_filter.is_empty() {
                    state.note_phase(state::UiEvent::CloseModal);
                    state.models_open = false;
                } else {
                    state.models_filter.clear();
                    state.models_sel = 0;
                }
            }
            KeyCode::Up => state.models_sel = state.models_sel.saturating_sub(1),
            KeyCode::Down => {
                if state.models_sel + 1 < state.filtered_models().len() {
                    state.models_sel += 1;
                }
            }
            KeyCode::PageUp => state.models_sel = state.models_sel.saturating_sub(8),
            KeyCode::PageDown => {
                let len = state.filtered_models().len();
                if len > 0 {
                    state.models_sel = (state.models_sel + 8).min(len - 1);
                }
            }
            KeyCode::Home => state.models_sel = 0,
            KeyCode::End => {
                let len = state.filtered_models().len();
                if len > 0 {
                    state.models_sel = len - 1;
                }
            }
            KeyCode::Backspace => {
                state.models_filter.pop();
                state.models_sel = 0;
            }
            KeyCode::Enter => {
                let picked = state
                    .filtered_models()
                    .get(state.models_sel)
                    .map(|m| (*m).clone());
                if let Some(model) = picked {
                    let label = model.name.clone().unwrap_or_else(|| model.id.clone());
                    state.note_phase(state::UiEvent::CloseModal);
                    state.models_open = false;
                    let _ = bridge
                        .request(RpcCommand::SetModel {
                            id: None,
                            provider: model.provider,
                            model_id: model.id,
                        })
                        .await;
                    state.notice = Some((format!("model → {label}"), "info".to_string()));
                }
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                state.models_filter.push(c);
                state.models_sel = 0;
            }
            _ => {}
        }
        state.models_clamp_selection();
        return;
    }

    // /resume modal captures navigation while open.
    // /resume picker with typeable filter (codex-qol stage 3): characters
    // narrow the list, esc clears first and closes second.
    if state.resume_open {
        match key.code {
            KeyCode::Esc => {
                if state.resume_filter.is_empty() {
                    state.resume_open = false;
                } else {
                    state.resume_filter.clear();
                    state.resume_sel = 0;
                }
            }
            KeyCode::Backspace => {
                state.resume_filter.pop();
                state.resume_sel = 0;
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                state.resume_filter.push(c);
                state.resume_sel = 0;
            }
            KeyCode::Up => state.resume_sel = state.resume_sel.saturating_sub(1),
            KeyCode::Down => {
                let n = state.filtered_resume_entries().len();
                if state.resume_sel + 1 < n {
                    state.resume_sel += 1;
                }
            }
            KeyCode::Enter => {
                let entries = state.filtered_resume_entries();
                if let Some(entry) = entries.get(state.resume_sel) {
                    let path = entry.path.to_string_lossy().to_string();
                    // Runtime peer fence (I2): a live peer's ACTIVE session
                    // file must not gain a second pi writer.
                    if let Some(note) = session_guard::peer_fenced_notice(&entry.path) {
                        state.entries.push(state::ChatEntry::Diag(note));
                        return;
                    }
                    state.note_phase(state::UiEvent::CloseModal);
                    state.resume_open = false;
                    if state.is_streaming {
                        state.entries.push(state::ChatEntry::Diag(
                            "dissolve first — session not switched".into(),
                        ));
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
            graceful_quit(state, bridge).await;
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
            let text = state.extract_selection().or_else(|| state.last_assistant());
            match text.filter(|t| !t.trim().is_empty()) {
                Some(t) => {
                    let n = t.chars().count();
                    let via = copy_to_clipboard(&t);
                    state.notice = Some((
                        format!("copied {} chars via {}", n, via),
                        "info".to_string(),
                    ));
                }
                None => {
                    state.notice = Some(("nothing to copy".to_string(), "info".to_string()));
                }
            }
        }
        // ctrl+v — cycle global tool verbosity (owner request 2026-09-03:
        // V for verbose). Line → Block → Full → Line. Persisted.
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            attach_clipboard_image(state);
        }
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::ALT) => {
            let n = state.pending_images.len();
            state.clear_images();
            state.notice = Some((
                if n == 0 {
                    "no images attached".into()
                } else {
                    format!("cleared {n} image(s)")
                },
                "info".into(),
            ));
        }
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
        // Sidebar scrollback (owner request 2026-11-10): ^↑/^↓ or the wheel
        // while the pointer is over the panel. Independent of chat scroll.
        KeyCode::Up if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.sidebar_scroll_up(3);
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.sidebar_scroll_down(3);
        }
        // Extension shortcuts in RPC mode are commands, not key events:
        // ctrl+q -> /plan, F2 -> /deep-research (2026-11-10).
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if state.commands.iter().any(|c| c.name == "plan") {
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: "/plan".to_string(),
                        images: None,
                        streaming_behavior: None,
                    })
                    .await;
            } else {
                state.entries.push(state::ChatEntry::Diag(
                    "plan-mode extension is not loaded".to_string(),
                ));
            }
        }
        KeyCode::F(2) => {
            if state.commands.iter().any(|c| c.name == "deep-research") {
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: "/deep-research".to_string(),
                        images: None,
                        streaming_behavior: None,
                    })
                    .await;
            } else {
                state.entries.push(state::ChatEntry::Diag(
                    "deep-research extension is not loaded".to_string(),
                ));
            }
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
            } else if state.prompt_ghost.is_some() && state.input.is_empty() {
                // Codex parity (stage 4): Tab accepts the ghost as an
                // editable draft when no completions are open. The agent-mode
                // cycle stays reachable once the ghost is dismissed.
                state.accept_ghost();
            } else if let Some(next) = state.next_agent_mode() {
                let m = format!("/agent {}", next);
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
            if state.cand_sel_logic_open() {
                state.cycle_candidate(false);
            } else {
                state.history_prev();
            }
        }
        KeyCode::Down => {
            if state.cand_sel_logic_open() {
                state.cycle_candidate(true);
            } else {
                state.history_next();
            }
        }
        // Scrollback (owner request 2026-09-02): PgUp/PgDn page through
        // session history.
        KeyCode::PageUp => {
            state.scroll_up(20);
        }
        KeyCode::PageDown => {
            state.scroll_down(20);
        }
        // Alt+Enter — queue a follow-up (pi follow_up) rather than steering
        // the in-flight turn; idle it behaves like a normal send.
        KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
            let msg = std::mem::take(&mut state.input);
            state.cursor = 0;
            state.cand_sel = 0;
            let msg = if msg.trim().is_empty() {
                if state.pending_images.is_empty() {
                    return;
                }
                "(screenshot)".to_string()
            } else {
                msg
            };
            state.record_prompt_history(&msg);
            if !msg.starts_with('/') {
                state.push_user(msg.clone());
            }
            if state.is_streaming {
                let _ = bridge
                    .request(RpcCommand::FollowUp {
                        id: None,
                        message: msg,
                        images: images_for_send(state),
                    })
                    .await;
                state.notice = Some(("queued follow-up".to_string(), "info".to_string()));
            } else {
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
        KeyCode::Enter => {
            // Resume modal open → handled above (modal key capture).
            // Preserve the hovered popup row: completion must use it, and the
            // old reset-to-zero here silently forced the first entry.
            let selected_candidate = state.cand_sel;
            let msg = std::mem::take(&mut state.input);
            state.cursor = 0;
            state.cand_sel = 0;
            let msg = if msg.is_empty() {
                // Image-only send (I5): the staged clipboard images carry it.
                if state.pending_images.is_empty() {
                    return;
                }
                "(screenshot)".to_string()
            } else {
                msg
            };
            // Prompt history (2026-11-13): recallable via Up/Down.
            state.record_prompt_history(&msg);
            // /fire — pure-local UI state (composer flames); never RPC.
            // /settings (alias /config) — consolidated UI settings.
            if msg.starts_with("/settings") || msg.starts_with("/config") {
                let start = if msg.starts_with("/settings") {
                    "/settings"
                } else {
                    "/config"
                };
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
                            state.notice = Some((format!("fire: {}", label), "info".to_string()));
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
            // /cache — local KV disk cache manager (never RPC).
            if msg == "/cache" || msg.starts_with("/cache ") {
                let args = msg.strip_prefix("/cache").unwrap_or("").trim();
                state.cache_command(args);
                return;
            }
            // /answers — last INQUIRE answer panel (facts file, read-only).
            if msg == "/answers" {
                state.answers_open_modal();
                return;
            }
            // /cost — today's routing ledger + budget bars (read-only).
            if msg == "/cost" {
                let day = chrono_day();
                state.cost = Some(crate::tui::cost::load(&day));
                state.note_phase(state::UiEvent::OpenModal);
                state.cost_open = true;
                return;
            }
            // /sessions — cross-project picker (scan is blocking; defer).
            if msg == "/sessions" {
                state.notice = Some(("scanning all project stores…".into(), "info".into()));
                let scan = tokio::task::spawn_blocking(crate::rpc::sessions::list_all).await;
                match scan {
                    Ok(list) if list.is_empty() => {
                        state.notice =
                            Some(("no sessions found in any store".into(), "info".into()));
                    }
                    Ok(list) => state.xsessions_open_with(list),
                    Err(e) => state
                        .entries
                        .push(state::ChatEntry::Diag(format!("session scan failed: {e}"))),
                }
                return;
            }
            // /review-findings — last /review results (read-only modal).
            if msg == "/review-findings" {
                state.review_open_modal();
                return;
            }
            // /status — session snapshot modal (local, read-only).
            if msg == "/engine" || msg.starts_with("/engine ") {
                let action = parse_engine_action(msg.trim());
                engine_ctl(state, action).await;
                return;
            }
            if msg == "/status" {
                state.status_open_modal();
                return;
            }
            // /caduceus — gateway page (never RPC; file-backed facts +
            // TUI-owned policy).
            if msg == "/caduceus" {
                state.caduceus_open_page();
                return;
            }
            // /rename, /export, /clone — RPC session controls.
            if msg == "/rename" || msg.starts_with("/rename ") {
                let name = msg.strip_prefix("/rename").unwrap_or("").trim().to_string();
                if name.is_empty() {
                    state
                        .entries
                        .push(state::ChatEntry::Diag("usage: /rename <name>".to_string()));
                } else {
                    let _ = bridge
                        .request(RpcCommand::SetSessionName { id: None, name })
                        .await;
                    state.notice = Some(("session renamed".to_string(), "info".to_string()));
                }
                return;
            }
            if msg == "/export" || msg.starts_with("/export ") {
                let path = msg.strip_prefix("/export").unwrap_or("").trim().to_string();
                let _ = bridge
                    .request(RpcCommand::ExportHtml {
                        id: None,
                        output_path: (!path.is_empty()).then_some(path),
                    })
                    .await;
                state.notice = Some(("exporting session…".to_string(), "info".to_string()));
                return;
            }
            if msg == "/clone" {
                let _ = bridge.request(RpcCommand::Clone { id: None }).await;
                state.notice = Some(("cloning session…".to_string(), "info".to_string()));
                return;
            }
            // /tree — render the session entry tree (get_tree response fills
            // state.tree_lines). Reusable: re-run to refresh after a fork.
            if msg == "/tree" {
                let _ = bridge.request(RpcCommand::GetTree { id: None }).await;
                return;
            }
            // /fork <n> — fork the session at entry n from the /tree listing.
            // No arg: show the /fork messages count (get_fork_messages).
            if msg == "/fork" || msg.starts_with("/fork ") {
                let args = msg.strip_prefix("/fork").unwrap_or("").trim();
                if args.is_empty() {
                    let _ = bridge
                        .request(RpcCommand::GetForkMessages { id: None })
                        .await;
                    return;
                }
                let n: u32 = match args.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        state.entries.push(state::ChatEntry::Diag(
                            "usage: /fork <n>  (n from /tree)".to_string(),
                        ));
                        return;
                    }
                };
                let entry_id = match state.tree_lines.get(n as usize - 1) {
                    Some((_, id)) => id.clone(),
                    None => {
                        state.entries.push(state::ChatEntry::Diag(format!(
                            "no entry {} — run /tree first",
                            n
                        )));
                        return;
                    }
                };
                let _ = bridge
                    .request(RpcCommand::Fork { id: None, entry_id })
                    .await;
                state.notice = Some((format!("forking at entry {}…", n), "info".to_string()));
                return;
            }
            // /sidebar — auto-hide on narrow terminals, or force show/hide.
            if msg == "/sidebar" {
                state.cycle_sidebar_mode();
                return;
            }
            // /permissions and /yolo — local policy switches.
            if msg == "/permissions" || msg == "/perms" {
                state.open_permissions_modal();
                return;
            }
            if msg == "/yolo" {
                state.toggle_yolo();
                return;
            }
            // /opencode — OpenCode session importer (writes a pi JSONL,
            // then switches this TUI to it; never launches a nested
            // frontend). NOT /import: session-panel owns /import for
            // task/scratchpad state.
            if msg == "/import-opencode" || msg.starts_with("/import-opencode ") {
                let args = msg
                    .strip_prefix("/opencode")
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let cwd = state.cwd.clone();
                import_opencode(state, bridge, &cwd, &args).await;
                return;
            }
            // Local fixed commands.
            match msg.as_str() {
                "/quit" | "/q" => {
                    graceful_quit(state, bridge).await;
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
                    state.cand_sel = selected_candidate;
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
                                state.resume_filter.clear();
                                state.note_phase(state::UiEvent::OpenModal);
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
                if state.has_running_tool() {
                    // Do not steer behind a tool that may be stuck: pi queues
                    // the message in memory and never persists it. Deliver it
                    // as a fresh Prompt once this turn settles (watchdog or
                    // normal completion). Images stay staged (I5).
                    state.queued_prompts.push(msg);
                    let img_note = if state.pending_images.is_empty() {
                        String::new()
                    } else {
                        " · images kept staged".to_string()
                    };
                    state.notice = Some((
                        format!("queued — delivered when the running tool settles{img_note}"),
                        "info".to_string(),
                    ));
                } else {
                    let images = images_for_send(state);
                    let _ = bridge
                        .request(RpcCommand::Steer {
                            id: None,
                            message: msg,
                            images,
                        })
                        .await;
                }
            } else {
                if echo {
                    state.push_user(msg.clone());
                }
                let images = images_for_send(state);
                // is_streaming flips on the agent_start event (Prompt path);
                // extension commands return without one — flag must not stick.
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: msg,
                        images,
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
        KeyCode::Right
            if state.prompt_ghost.is_some() && state.cursor >= state.input.chars().count() =>
        {
            state.accept_ghost();
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
        KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::ALT) => {
            request_editor(state);
        }
        _ => {
            if let KeyCode::Char(c) = key.code {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    // Any typed character belongs to the operator — the
                    // ghost yields first (I2).
                    state.dismiss_ghost("dismissed-typed");
                    insert_char_at(&mut state.input, state.cursor, c);
                    state.cursor += 1;
                    state.cand_sel = 0;
                }
            }
        }
    }
}

/// `/import [latest|<id>|list] [full]` — bridge to `scripts/opencode.mjs`.
/// The importer converts the OpenCode SQLite session into a pi JSONL without
/// launching anything (`--no-launch`), then we switch this TUI to it.
async fn import_opencode(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    cwd: &std::path::Path,
    args: &str,
) {
    if state.is_streaming {
        state.entries.push(state::ChatEntry::Diag(
            "import: finish/dissolve the current turn first".to_string(),
        ));
        return;
    }
    let Some(script) = bridge.officina_script("opencode.mjs") else {
        state.entries.push(state::ChatEntry::Diag(
            "import: opencode.mjs not found next to this Officina build".to_string(),
        ));
        return;
    };

    let mut parts = args.split_whitespace();
    let sub = parts.next().unwrap_or("latest");
    let listing = sub == "list";
    let full = args
        .split_whitespace()
        .any(|a| a == "--full" || a == "full");
    let target = sub.to_string();
    let cwd_display = cwd.display().to_string();
    let cwd = cwd.to_path_buf();

    let run = tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("node");
        cmd.arg(&script);
        if listing {
            cmd.args(["list", "--json"]);
        } else {
            cmd.arg("import");
            cmd.arg(&target);
            cmd.args(["--json", "--no-launch"]);
            if full {
                cmd.arg("--full");
            }
        }
        cmd.current_dir(&cwd);
        cmd.output()
    })
    .await;

    let out = match run {
        Ok(Ok(out)) if out.status.success() => out,
        Ok(Ok(out)) => {
            let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
            state.entries.push(state::ChatEntry::Diag(format!(
                "import failed: {}",
                if err.is_empty() {
                    format!("exit {:?}", out.status.code())
                } else {
                    err
                }
            )));
            return;
        }
        Ok(Err(e)) => {
            state.entries.push(state::ChatEntry::Diag(format!(
                "import failed to run node: {e}"
            )));
            return;
        }
        Err(e) => {
            state
                .entries
                .push(state::ChatEntry::Diag(format!("import task failed: {e}")));
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let Ok(data) = serde_json::from_str::<serde_json::Value>(&stdout) else {
        state.entries.push(state::ChatEntry::Diag(
            "import: importer returned no JSON report".to_string(),
        ));
        return;
    };

    if listing {
        let Some(sessions) = data.as_array() else {
            state.entries.push(state::ChatEntry::Diag(
                "import list: unexpected report shape".to_string(),
            ));
            return;
        };
        if sessions.is_empty() {
            state.entries.push(state::ChatEntry::Diag(
                "import: no OpenCode sessions for this project".to_string(),
            ));
            return;
        }
        state.entries.push(state::ChatEntry::Diag(format!(
            "OpenCode sessions for {cwd_display}:"
        )));
        for session in sessions.iter().take(30) {
            let id = session.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let title = session.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let updated = session
                .get("updated")
                .and_then(|v| v.as_i64())
                .map(|ms| {
                    let secs = ms / 1000;
                    let days = (chrono_like_now().saturating_sub(secs)) / 86_400;
                    format!("{days}d ago")
                })
                .unwrap_or_default();
            state.entries.push(state::ChatEntry::Diag(format!(
                "  {id}  {title}  {updated}"
            )));
        }
        state.entries.push(state::ChatEntry::Diag(
            "import with: /import <id>   (/import latest for the newest)".to_string(),
        ));
        return;
    }

    let Some(path) = data.get("path").and_then(|v| v.as_str()) else {
        state.entries.push(state::ChatEntry::Diag(
            "import: report has no session path".to_string(),
        ));
        return;
    };
    // Runtime peer fence (I2): same gate as /resume — an import target
    // that a live peer is writing must not gain a second writer.
    if let Some(note) = session_guard::peer_fenced_notice(Path::new(path)) {
        state.entries.push(state::ChatEntry::Diag(note));
        return;
    }
    let title = data
        .get("source")
        .and_then(|s| s.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("OpenCode session");
    state.entries.push(state::ChatEntry::Diag(format!(
        "imported {title} -> {}",
        path
    )));
    let _ = bridge
        .request(RpcCommand::SwitchSession {
            id: None,
            session_path: path.to_string(),
        })
        .await;
}

/// Seconds since the Unix epoch, without a chrono dependency.
fn chrono_like_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Env override for a seconds knob; invalid/missing falls back to `default`.
fn env_secs(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

/// Ask pi to abort an in-flight turn and wait briefly for it to flush its
/// session before the TUI exits. Killing the child mid-tool is what created
/// dangling tool calls and corrupted long sessions.
async fn graceful_quit(state: &mut AppState, bridge: &mut RpcBridge) {
    // Resolve extension dialogs before abort. A pending confirm/select is a
    // promise inside pi's beforeToolCall; leaving it unanswered while the
    // child is torn down is how dangling tool calls were born.
    cancel_pending_dialogs(state, bridge).await;
    if state.is_streaming {
        let _ = bridge.request(RpcCommand::Abort { id: None }).await;
        for _ in 0..30 {
            if !state.is_streaming {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    state.should_quit = true;
}

/// Answer every queued extension dialog with `cancelled: true`.
async fn cancel_pending_dialogs(state: &mut AppState, bridge: &mut RpcBridge) {
    while let Some(dialog) = state.dialogs.pop_front() {
        let payload = dialog_cancelled(&dialog.id);
        let _ = bridge.send_raw(&payload.to_string()).await;
    }
}

/// Tool-call stall watchdog. A tool with no `tool_execution_end` leaves
/// `is_streaming` true forever, which turns later user prompts into RPC
/// `Steer` messages queued behind a turn that will never finish. After the
/// threshold, cancel any blocking dialog, abort once, and (if the abort also
/// fails to settle) surface an explicit restart hint.
async fn maybe_tool_watchdog(state: &mut AppState, bridge: &mut RpcBridge) {
    if state.tool_watchdog_secs == 0 {
        return;
    }
    let now = std::time::Instant::now();
    let mut stalled = false;

    // Stage 0: an assistant turn announced a tool call, but pi never emitted
    // `tool_execution_start`. That is the exact "persisted tool call, nothing
    // happened" failure; it has no running tool entry to age out.
    let start_limit = std::time::Duration::from_secs(state.tool_watchdog_secs.min(60).max(15));
    if let Some((id, age)) = state.oldest_expected_tool_call() {
        if age >= start_limit && state.watchdog_fired.insert(format!("expected:{id}")) {
            state.entries.push(state::ChatEntry::Diag(format!(
                "harness watchdog: tool call {} never started after {}s — cancelling and aborting the turn",
                id.chars().take(8).collect::<String>(),
                age.as_secs()
            )));
            stalled = true;
        }
    }

    if let Some((key, name, age, timeout_s)) = state.oldest_running_tool() {
        // A declared tool timeout is authoritative: wait it out plus grace.
        let base = std::time::Duration::from_secs(state.tool_watchdog_secs);
        let limit = timeout_s
            .map(|s| std::time::Duration::from_secs(s.saturating_add(60)))
            .map_or(base, |t| t.max(base));
        if age >= limit && state.watchdog_fired.insert(key) {
            state.entries.push(state::ChatEntry::Diag(format!(
                "harness watchdog: {name} idle {}s with no result — cancelling and aborting the turn",
                age.as_secs()
            )));
            stalled = true;
        }
    } else if state.expected_tool_calls.is_empty() {
        state.abort_deadline = None;
    }

    if stalled {
        cancel_pending_dialogs(state, bridge).await;
        let _ = bridge.request(RpcCommand::Abort { id: None }).await;
        state.abort_deadline = Some(now + std::time::Duration::from_secs(10));
    }

    if state.abort_deadline.is_some_and(|deadline| now >= deadline) {
        state.abort_deadline = None;
        state.entries.push(state::ChatEntry::Diag(
            "harness watchdog: abort did not settle (pi child may be stuck). \
             Session is preserved; /quit then relaunch Officina to repair it."
                .to_string(),
        ));
    }
}

/// Is the pointer inside the last-rendered sidebar panel?
fn over_sidebar(state: &AppState, column: u16, row: u16) -> bool {
    state.last_sidebar_area.is_some_and(|a| {
        column >= a.x && column < a.x + a.width && row >= a.y && row < a.y + a.height
    })
}

async fn handle_dialog_key(key: KeyEvent, state: &mut AppState, bridge: &mut RpcBridge) {
    let Some(dialog) = state.dialogs.front_mut() else {
        return;
    };
    let id = dialog.id.clone();
    let kind = dialog.kind;
    let mut reply: Option<serde_json::Value> = None;

    match kind {
        state::DialogKind::Confirm => match key.code {
            KeyCode::Esc => reply = Some(dialog_cancelled(&id)),
            KeyCode::Char('y') | KeyCode::Char('Y') => reply = Some(dialog_confirmed(&id, true)),
            KeyCode::Char('n') | KeyCode::Char('N') => reply = Some(dialog_confirmed(&id, false)),
            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                dialog.selected = if dialog.selected == 0 { 1 } else { 0 };
            }
            KeyCode::Enter => reply = Some(dialog_confirmed(&id, dialog.selected == 0)),
            _ => {}
        },
        state::DialogKind::Select => match key.code {
            KeyCode::Esc => reply = Some(dialog_cancelled(&id)),
            KeyCode::Up | KeyCode::Char('k') => dialog.selected = dialog.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                if dialog.selected + 1 < dialog.options.len() {
                    dialog.selected += 1;
                }
            }
            KeyCode::PageUp => dialog.selected = dialog.selected.saturating_sub(8),
            KeyCode::PageDown => {
                if !dialog.options.is_empty() {
                    dialog.selected = (dialog.selected + 8).min(dialog.options.len() - 1);
                }
            }
            KeyCode::Home => dialog.selected = 0,
            KeyCode::End => dialog.selected = dialog.options.len().saturating_sub(1),
            KeyCode::Enter => {
                if let Some(value) = dialog.options.get(dialog.selected).cloned() {
                    reply = Some(dialog_value(&id, value));
                }
            }
            _ => {}
        },
        state::DialogKind::Input | state::DialogKind::Editor => {
            let is_editor = kind == state::DialogKind::Editor;
            match key.code {
                KeyCode::Esc => reply = Some(dialog_cancelled(&id)),
                KeyCode::Enter
                    if !is_editor
                        || key.modifiers.contains(KeyModifiers::ALT)
                        || key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    reply = Some(dialog_value(&id, dialog.input.clone()));
                }
                KeyCode::Enter => dialog.insert_char('\n'),
                KeyCode::Backspace => dialog.backspace(),
                KeyCode::Delete => dialog.delete(),
                KeyCode::Left => dialog.move_left(),
                KeyCode::Right => dialog.move_right(),
                KeyCode::Home => dialog.cursor = 0,
                KeyCode::End => dialog.cursor = dialog.input.chars().count(),
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    dialog.input.clear();
                    dialog.cursor = 0;
                }
                KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    while dialog.cursor > 0 {
                        let prev = dialog.input.chars().nth(dialog.cursor - 1);
                        dialog.backspace();
                        if prev.is_some_and(char::is_whitespace) {
                            break;
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    dialog.insert_char(c)
                }
                _ => {}
            }
        }
    }

    if let Some(payload) = reply {
        state.note_phase(state::UiEvent::CloseDialog);
        state.dialogs.pop_front();
        let _ = bridge.send_raw(&payload.to_string()).await;
    }
}

fn dialog_cancelled(id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "extension_ui_response", "id": id, "cancelled": true })
}

fn dialog_confirmed(id: &str, confirmed: bool) -> serde_json::Value {
    serde_json::json!({ "type": "extension_ui_response", "id": id, "confirmed": confirmed })
}

fn dialog_value(id: &str, value: String) -> serde_json::Value {
    serde_json::json!({ "type": "extension_ui_response", "id": id, "value": value })
}

async fn handle_mouse(m: crossterm::event::MouseEvent, state: &mut AppState) {
    use crossterm::event::{MouseButton, MouseEventKind};
    match m.kind {
        MouseEventKind::ScrollUp => {
            state.clear_selection();
            if over_sidebar(state, m.column, m.row) {
                state.sidebar_scroll_up(3);
            } else {
                state.scroll_up(3);
            }
        }
        MouseEventKind::ScrollDown => {
            state.clear_selection();
            if over_sidebar(state, m.column, m.row) {
                state.sidebar_scroll_down(3);
            } else {
                state.scroll_down(3);
            }
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
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
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
/// Flatten a `get_tree` session-entry tree into numbered, addressable
/// display lines: depth-first walk (children pushed back so the stack
/// pops them in order), each line `"<n>  <type>[ <role>]"` paired with
/// the entry id `/fork <n>` resolves against. Isolated from the response
/// handler so the numbering contract has a unit test.
fn session_tree_lines(tree: &[serde_json::Value]) -> Vec<(String, String)> {
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut stack: Vec<&serde_json::Value> = tree.iter().rev().collect();
    while let Some(node) = stack.pop() {
        // A wrapper node without an entry still descends — only the line
        // emission is conditional, never the subtree walk.
        let entry = node.get("entry");
        if let Some(entry) = entry {
            let id = entry
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let typ = entry
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("entry");
            let role = entry
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
                .unwrap_or("");
            let label = if !role.is_empty() {
                format!("{typ} {role}")
            } else {
                typ.to_string()
            };
            lines.push((format!("{:>3}  {}", lines.len() + 1, label), id));
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            stack.extend(children.iter().rev());
        }
    }
    lines
}

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
            state.tree_lines.clear();
            state.tree_open = false;
            state.tree_sel = 0;
            return true;
        }
        Some("get_state") => {
            state.apply_state(
                data.get("model")
                    .and_then(|m| serde_json::from_value(m.clone()).ok()),
                data.get("thinkingLevel")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                data.get("isStreaming")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                data.get("isCompacting")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                data.get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                data.get("sessionName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
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
                            let text =
                                content_text(m.get("content").unwrap_or(&serde_json::Value::Null));
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
                                            if let Some(t) = b.get("text").and_then(|v| v.as_str())
                                            {
                                                if !t.trim().is_empty() {
                                                    state.entries.push(
                                                        state::ChatEntry::Assistant(t.to_string()),
                                                    );
                                                }
                                            }
                                        }
                                        "toolCall" => {
                                            let name = b
                                                .get("name")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("tool");
                                            let id = b.get("id").and_then(|v| v.as_str()).or_else(
                                                || b.get("toolCallId").and_then(|v| v.as_str()),
                                            );
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
                            let id = m
                                .get("toolCallId")
                                .and_then(|v| v.as_str())
                                .or_else(|| m.get("id").and_then(|v| v.as_str()));
                            let name = m.get("toolName").and_then(|v| v.as_str()).unwrap_or("");
                            let text =
                                content_text(m.get("content").unwrap_or(&serde_json::Value::Null));
                            let is_err =
                                m.get("isError").and_then(|v| v.as_bool()).unwrap_or(false);
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
        Some("get_available_models") => {
            let models: Vec<crate::rpc::protocol::Model> = data
                .get("models")
                .and_then(|m| m.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| serde_json::from_value(v.clone()).ok())
                        .collect()
                })
                .unwrap_or_default();
            if models.is_empty() {
                state.entries.push(state::ChatEntry::Diag(
                    "no models available — check provider/auth".to_string(),
                ));
                state.models_open = false;
                return false;
            }
            // Start on the currently selected row so Enter is a no-op unless
            // the user moves.
            let current = state
                .model
                .as_ref()
                .map(|m| (m.provider.clone(), m.id.clone()));
            state.models_sel = current
                .and_then(|(provider, id)| {
                    models
                        .iter()
                        .position(|m| m.provider == provider && m.id == id)
                })
                .unwrap_or(0);
            state.models = models;
            state.models_filter.clear();
            state.models_sel = state.models_sel.min(state.models.len().saturating_sub(1));
            state.note_phase(state::UiEvent::OpenModal);
            state.models_open = true;
        }
        Some("export_html") => {
            let path = data
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("session.html");
            state.notice = Some((format!("exported → {path}"), "info".to_string()));
        }
        Some("set_session_name") => {
            state.notice = Some(("session renamed".to_string(), "info".to_string()));
        }
        // /tree: flatten the session entry tree into numbered, addressable
        // lines so `/fork <n>` can pick an entry id by position. The tree
        // is depth-first, timestamp-sorted; each line stores (display, id).
        Some("get_tree") => {
            let tree = match data.get("tree").and_then(|t| t.as_array()) {
                Some(t) => t,
                None => {
                    state
                        .entries
                        .push(state::ChatEntry::Diag("empty session tree".to_string()));
                    return false;
                }
            };
            let lines = session_tree_lines(tree);
            if lines.is_empty() {
                state
                    .entries
                    .push(state::ChatEntry::Diag("empty session tree".to_string()));
                return false;
            }
            state.tree_lines = lines;
            state.tree_sel = 0;
            state.tree_open = true;
        }
        // /fork with no arg: how many user messages are fork-able.
        Some("get_fork_messages") => {
            let n = data
                .get("messages")
                .and_then(|m| m.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            state.notice = Some((
                format!("{} forkable message(s) — /tree then /fork <n>", n),
                "info".to_string(),
            ));
        }
        Some("fork") => {
            state.notice = Some(("forked to a new session".to_string(), "info".to_string()));
            // Fork replaces the session — clear the stale tree capture so a
            // fresh /tree re-fetches; transcript/widgets refresh via the
            // switch_session rehydrate that follows.
            state.tree_lines.clear();
            state.tree_open = false;
            state.tree_sel = 0;
        }
        Some("set_model") => {
            if let Ok(model) = serde_json::from_value::<crate::rpc::protocol::Model>(data.clone()) {
                state.model = Some(model);
            }
        }
        Some("get_session_stats") => {
            // Context fill for the bottom status row. pi's wire agent_end does
            // NOT carry contextUsage (only {type, messages, willRetry}) — the
            // number lives in getSessionStats, which the RPC surfaces as
            // get_session_stats. Parse it here, on the response path.
            if let Ok(stats) =
                serde_json::from_value::<crate::rpc::protocol::SessionStats>(data.clone())
            {
                if let Some(cu) = stats.context_usage {
                    let tokens = cu.tokens.unwrap_or(0);
                    let window = cu.context_window;
                    if window > 0 {
                        state.ctx_usage = Some((tokens, window));
                    }
                }
            }
        }
        _ => {}
    }
    false
}

/// One shot of the compaction auto-continue (owner choice B): sent once
/// per threshold compaction. Deliberately generic — the model knows what
/// it was doing; we just hand it back the wheel.
const CONTINUE_PROMPT: &str = "Continue the open work — pick up exactly where you left off.";

/// Handle one agent event. Returns true when the run loop should submit
/// the compaction auto-continue (see CONTINUE_PROMPT).
fn handle_event(state: &mut AppState, event_type: &str, fields: &serde_json::Value) -> bool {
    match event_type {
        "agent_start" => {
            state.note_phase(state::UiEvent::AgentStart);
            state.is_streaming = true;
            state.watchdog_fired.clear();
            state.abort_deadline = None;
            state.expected_tool_calls.clear();
        }
        "agent_end" | "agent_settled" => {
            state.note_phase(state::UiEvent::AgentEnd);
            state.is_streaming = false;
            state.refresh_ghost();
            state.refresh_answers_notice();
            state.abort_deadline = None;
            let expected = state.expected_tool_calls.len();
            state.expected_tool_calls.clear();
            // A turn that ends with a running tool means pi never emitted a
            // tool_execution_end (abort while the tool or its beforeToolCall
            // was pending). Close the spinner honestly; startup session
            // repair fixes the persisted dangling call on next launch.
            let abandoned = state.abandon_running_tools("turn ended before tool result");
            if abandoned > 0 || expected > 0 {
                state.entries.push(state::ChatEntry::Diag(format!(
                    "harness: {} tool call(s) ended without a result — session repair will run on next launch",
                    abandoned + expected
                )));
            }
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
                    state.note_expected_tool_calls(content);
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
            let name = fields
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("tool");
            let tool_call_id = fields.get("toolCallId").and_then(|v| v.as_str());
            let args = fields
                .get("args")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            state.note_tool_started(tool_call_id);
            state.tool_start(tool_call_id, name, &args);
        }
        "tool_execution_update" => {
            let name = fields
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("tool");
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
                .or_else(|| {
                    fields
                        .get("partialResult")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .unwrap_or_default();
            state.tool_update(tool_call_id, name, &text);
        }
        "tool_execution_end" => {
            let name = fields
                .get("toolName")
                .and_then(|v| v.as_str())
                .unwrap_or("tool");
            let tool_call_id = fields.get("toolCallId").and_then(|v| v.as_str());
            let is_err = fields
                .get("isError")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
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
            state.note_tool_finished(tool_call_id);
            state.tool_end(tool_call_id, name, &result_text, is_err);
        }
        "extension_error" => {
            let path = fields
                .get("extensionPath")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
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
            state
                .entries
                .push(state::ChatEntry::Diag("retry settled".to_string()));
        }
        "compaction_start" => {
            state.note_phase(state::UiEvent::CompactionStart);
            state.is_compacting = true;
            // Re-arm the one-shot auto-continue for THIS compaction.
            state.compaction_auto_armed = true;
        }
        "compaction_end" => {
            state.note_phase(state::UiEvent::CompactionEnd);
            state.is_compacting = false;
            let reason = fields.get("reason").and_then(|v| v.as_str()).unwrap_or("");
            let will_retry = fields
                .get("willRetry")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let aborted = fields
                .get("aborted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let err = fields
                .get("errorMessage")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());
            if let Some(e) = err {
                // Overflow-recovery failures carried this silently before —
                // the owner saw "compacted and stopped" with no explanation.
                state
                    .entries
                    .push(state::ChatEntry::Diag(format!("compaction failed: {}", e)));
            } else {
                let result = fields.get("result");
                let before = result
                    .and_then(|r| r.get("tokensBefore"))
                    .and_then(|v| v.as_u64());
                let after = result
                    .and_then(|r| r.get("estimatedTokensAfter"))
                    .and_then(|v| v.as_u64());
                let nums = match (before, after) {
                    (Some(b), Some(a)) => format!(" {} → {} tok", b, a),
                    _ => String::new(),
                };
                state.entries.push(state::ChatEntry::Diag(format!(
                    "context compacted{} ({}){}",
                    nums,
                    reason,
                    if will_retry {
                        " — auto-retry follows"
                    } else {
                        ""
                    },
                )));
            }
            // Pick-the-work-back-up: threshold compactions end the run BY
            // DESIGN (pi agent-session.js: "user continues manually") — fire
            // the one-shot continuation. Overflow compactions already
            // auto-retry; aborted/failed ones have nothing to continue.
            let fire = !aborted
                && !will_retry
                && err.is_none()
                && reason == "threshold"
                && state.compaction_auto_armed;
            state.compaction_auto_armed = false;
            if fire {
                state.entries.push(state::ChatEntry::Diag(
                    "picking the work back up…".to_string(),
                ));
                return true;
            }
        }
        "model_select" => {
            if let Ok(m) = serde_json::from_value::<crate::rpc::protocol::Model>(
                fields
                    .get("model")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
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
    false
}

async fn handle_ui_request(
    state: &mut AppState,
    method: &str,
    fields: &serde_json::Value,
    bridge: &mut RpcBridge,
) {
    match method {
        "setWidget" => {
            let key = fields
                .get("widgetKey")
                .and_then(|v| v.as_str())
                .unwrap_or("widget");
            let lines: Vec<String> = fields
                .get("widgetLines")
                .and_then(|l| l.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let placement = fields
                .get("widgetPlacement")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            state.set_widget_placed(key, lines, placement);
        }
        "setTitle" => {
            if let Some(t) = fields.get("title").and_then(|v| v.as_str()) {
                state.title = t.to_string();
                let _ = execute!(io::stdout(), crossterm::terminal::SetTitle(t));
            }
        }
        "notify" => {
            let msg = fields.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let ntype = fields
                .get("notifyType")
                .and_then(|v| v.as_str())
                .unwrap_or("info");
            state.apply_notify(msg, ntype);
        }
        "setStatus" => {
            let key = fields
                .get("statusKey")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let text = fields
                .get("statusText")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if key == "engine" {
                // vitriol engine status surfaces as a notice line
                state.notice = Some((text.to_string(), "info".to_string()));
            }
        }
        "select" | "confirm" | "input" | "editor" => {
            // Extension dialogs are real modals in the Rust TUI (2026-11-10).
            // Queue them; the response is sent when the user decides.
            let id = fields
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                return;
            }
            let title = fields
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or(method)
                .to_string();
            let message = fields
                .get("message")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            match method {
                "select" => {
                    let options: Vec<String> = fields
                        .get("options")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    if options.is_empty() {
                        let payload = serde_json::json!({
                            "type": "extension_ui_response", "id": id, "cancelled": true
                        });
                        let _ = bridge.send_raw(&payload.to_string()).await;
                    } else {
                        state.note_phase(state::UiEvent::OpenDialog);
                        state
                            .dialogs
                            .push_back(state::Dialog::select(id, title, options));
                    }
                }
                "confirm" => {
                    state.note_phase(state::UiEvent::OpenDialog);
                    state
                        .dialogs
                        .push_back(state::Dialog::confirm(id, title, message));
                }
                _ => {
                    // `placeholder` is a hint, not content — only `prefill`
                    // (editor) seeds the value.
                    let prefill = fields
                        .get("prefill")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let kind = if method == "editor" {
                        state::DialogKind::Editor
                    } else {
                        state::DialogKind::Input
                    };
                    state.note_phase(state::UiEvent::OpenDialog);
                    state
                        .dialogs
                        .push_back(state::Dialog::input(id, kind, title, prefill));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_flatten_numbers_depth_first_with_roles() {
        let tree = serde_json::json!([
            {
                "entry": {"id": "a", "type": "message", "message": {"role": "user"}},
                "children": [
                    {"entry": {"id": "b", "type": "message", "message": {"role": "assistant"}}}
                ]
            },
            {"entry": {"id": "c", "type": "tool_use"}},
            // Nodes without an entry (e.g. synthetic roots) are skipped.
            {"children": [{"entry": {"id": "d", "type": "compaction"}}]}
        ]);
        let lines = session_tree_lines(tree.as_array().unwrap());
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], ("  1  message user".to_string(), "a".to_string()));
        assert_eq!(
            lines[1],
            ("  2  message assistant".to_string(), "b".to_string())
        );
        assert_eq!(lines[2], ("  3  tool_use".to_string(), "c".to_string()));
        assert_eq!(lines[3], ("  4  compaction".to_string(), "d".to_string()));
        // /fork <n> indexes are 1-based positions in this exact order.
        let n: usize = 4;
        assert_eq!(lines[n - 1].1, "d");
    }

    #[test]
    fn tree_flatten_empty_and_entryless() {
        assert!(session_tree_lines(&[]).is_empty());
        let tree = serde_json::json!([{"children": []}, {}]);
        assert!(session_tree_lines(tree.as_array().unwrap()).is_empty());
    }

    #[test]
    fn dialog_input_edits_at_cursor() {
        let mut d = state::Dialog::input(
            "id".into(),
            state::DialogKind::Input,
            "title".into(),
            "abc".into(),
        );
        d.move_left();
        d.insert_char('X');
        assert_eq!(d.input, "abXc");
        assert_eq!(d.cursor, 3);
        d.backspace();
        assert_eq!(d.input, "abc");
        d.delete();
        assert_eq!(d.input, "ab");
    }

    #[test]
    fn dialog_responses_match_rpc_payloads() {
        assert_eq!(
            dialog_cancelled("x"),
            serde_json::json!({"type":"extension_ui_response","id":"x","cancelled":true})
        );
        assert_eq!(
            dialog_confirmed("x", false),
            serde_json::json!({"type":"extension_ui_response","id":"x","confirmed":false})
        );
        assert_eq!(
            dialog_value("x", "Allow".into()),
            serde_json::json!({"type":"extension_ui_response","id":"x","value":"Allow"})
        );
    }

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
        s.last_chat_area = Some(Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 6,
        });
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

    #[test]
    fn model_command_opens_picker_or_switches_directly() {
        let mut s = AppState::default();
        assert!(matches!(
            s.local_dispatch("/model"),
            Some(RpcCommand::GetAvailableModels { .. })
        ));
        match s.local_dispatch("/model zai/glm-4.6") {
            Some(RpcCommand::SetModel {
                provider, model_id, ..
            }) => {
                assert_eq!(provider, "zai");
                assert_eq!(model_id, "glm-4.6");
            }
            other => panic!("expected SetModel, got {other:?}"),
        }
        assert!(s.local_dispatch("/model bogus").is_none());
        assert_eq!(s.entries.len(), 1, "bad form gets one diagnostic");
    }

    #[test]
    fn session_stats_response_populates_ctx_usage() {
        let mut s = AppState::default();
        let resp: crate::rpc::protocol::RpcResponse = serde_json::from_value(serde_json::json!({
            "type": "response",
            "command": "get_session_stats",
            "success": true,
            "data": {
                "sessionId": "s1",
                "userMessages": 2,
                "assistantMessages": 2,
                "toolCalls": 1,
                "totalMessages": 4,
                "tokens": { "input": 100, "output": 40, "total": 140 },
                "contextUsage": { "tokens": 12345, "contextWindow": 262144, "percent": 4.7 }
            }
        }))
        .unwrap();
        assert!(!handle_response(&mut s, &resp));
        assert_eq!(s.ctx_usage, Some((12_345, 262_144)));
        assert_eq!(
            s.ctx_bar_label().as_deref(),
            Some("🜖 ctx 12.3k / 262.1k (4.7%)")
        );
    }

    #[test]
    fn session_stats_response_without_window_leaves_ctx_usage_none() {
        let mut s = AppState::default();
        let resp: crate::rpc::protocol::RpcResponse = serde_json::from_value(serde_json::json!({
            "type": "response",
            "command": "get_session_stats",
            "success": true,
            "data": {
                "userMessages": 0,
                "assistantMessages": 0,
                "toolCalls": 0,
                "totalMessages": 0,
                "contextUsage": { "tokens": 0, "contextWindow": 0, "percent": 0.0 }
            }
        }))
        .unwrap();
        assert!(!handle_response(&mut s, &resp));
        assert_eq!(s.ctx_usage, None, "I1: no data ⇒ row stays hidden");
    }

    #[test]
    fn available_models_response_opens_picker_on_current_model() {
        let mut s = AppState::default();
        s.model = Some(
            serde_json::from_value(serde_json::json!({"id": "b", "provider": "p", "name": "B"}))
                .unwrap(),
        );
        let resp: crate::rpc::protocol::RpcResponse = serde_json::from_value(serde_json::json!({
            "type": "response",
            "command": "get_available_models",
            "success": true,
            "data": { "models": [
                { "id": "a", "provider": "p", "name": "A" },
                { "id": "b", "provider": "p", "name": "B" }
            ]}
        }))
        .unwrap();
        assert!(!handle_response(&mut s, &resp));
        assert!(s.models_open);
        assert_eq!(s.models.len(), 2);
        assert_eq!(s.models_sel, 1, "starts on the active model");
        assert!(s.models_filter.is_empty());
    }

    #[test]
    fn model_picker_filter_and_clamp() {
        let mut s = AppState::default();
        s.models = serde_json::from_value(serde_json::json!([
            {"id": "glm-4.6", "provider": "zai", "name": "GLM 4.6", "contextWindow": 200000},
            {"id": "qwen3.8", "provider": "llamacpp", "name": "Qwen 3.8"},
            {"id": "glm-5-air", "provider": "zai", "name": "GLM 5 Air"}
        ]))
        .unwrap();
        s.models_filter = "glm".into();
        assert_eq!(s.filtered_models().len(), 2);
        s.models_filter = "zai glm-5".into();
        assert_eq!(s.filtered_models().len(), 1);
        s.models_sel = 99;
        s.models_clamp_selection();
        assert_eq!(s.models_sel, 0, "clamped to filtered list");
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
                state::ChatEntry::Tool {
                    running,
                    error,
                    output,
                    ..
                } => {
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

#[cfg(test)]
mod compaction_tests {
    use super::*;

    fn ev(json: serde_json::Value) -> serde_json::Value {
        json
    }

    #[test]
    fn threshold_compaction_fires_auto_continue_once() {
        let mut s = AppState::default();
        // Start arms the one-shot.
        assert!(!handle_event(
            &mut s,
            "compaction_start",
            &ev(serde_json::json!({ "reason": "threshold" }))
        ));
        assert!(s.compaction_auto_armed);
        assert!(s.is_compacting);
        // Threshold end with token numbers → fire + diag with numbers.
        let fired = handle_event(
            &mut s,
            "compaction_end",
            &ev(serde_json::json!({
                "reason": "threshold", "aborted": false, "willRetry": false,
                "result": { "tokensBefore": 81000, "estimatedTokensAfter": 32000 }
            })),
        );
        assert!(fired, "threshold compaction picks the work back up");
        assert!(!s.compaction_auto_armed, "consumed");
        assert!(!s.is_compacting);
        // A second end without a new start does NOT fire again.
        assert!(!handle_event(
            &mut s,
            "compaction_end",
            &ev(serde_json::json!({ "reason": "threshold", "willRetry": false }))
        ));
        // The transcript explains what happened.
        let diag: Vec<String> = s
            .entries
            .iter()
            .filter_map(|e| match e {
                state::ChatEntry::Diag(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            diag.iter().any(|d| d.contains("81000 → 32000 tok")),
            "{:?}",
            diag
        );
        assert!(diag.iter().any(|d| d.contains("picking the work back up")));
    }

    #[test]
    fn overflow_and_failure_compactions_do_not_fire() {
        let mut s = AppState::default();
        // Overflow: pi auto-retries on its own — we must not double-fire.
        handle_event(&mut s, "compaction_start", &ev(serde_json::json!({})));
        assert!(!handle_event(
            &mut s,
            "compaction_end",
            &ev(serde_json::json!({ "reason": "overflow", "willRetry": true }))
        ));
        // Failed compaction: errorMessage present, nothing to continue.
        handle_event(&mut s, "compaction_start", &ev(serde_json::json!({})));
        assert!(!handle_event(
            &mut s,
            "compaction_end",
            &ev(serde_json::json!({
                "reason": "overflow", "aborted": false, "willRetry": false,
                "errorMessage": "Context overflow recovery failed"
            }))
        ));
        // …but the failure is SURFACED now (it used to vanish).
        let diag: Vec<String> = s
            .entries
            .iter()
            .filter_map(|e| match e {
                state::ChatEntry::Diag(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(diag
            .iter()
            .any(|d| d.contains("compaction failed: Context overflow recovery failed")));
    }
}

// ── Phase-wiring tests (verification-rollout T0.1) ────────────────────
//
// The transition table is only useful if the real event stream never trips
// it. These tests replay the sequences pi actually emits and assert the
// wiring produces NO "state: unexpected …" diagnostics.
#[cfg(test)]
mod phase_wiring_tests {
    use super::*;

    fn state_diags(s: &AppState) -> Vec<String> {
        s.entries
            .iter()
            .filter_map(|e| match e {
                state::ChatEntry::Diag(t) if t.starts_with("state:") => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    fn fire(s: &mut AppState, ty: &str) -> bool {
        handle_event(s, ty, &serde_json::json!({}))
    }

    #[test]
    fn normal_turn_emits_no_state_diag() {
        let mut s = AppState::default();
        fire(&mut s, "agent_start");
        assert_eq!(s.phase(), state::UiPhase::Streaming);
        fire(&mut s, "agent_end");
        // pi emits agent_end AND agent_settled for one turn.
        fire(&mut s, "agent_settled");
        assert_eq!(s.phase(), state::UiPhase::Idle);
        assert!(
            state_diags(&s).is_empty(),
            "normal turn must not trip the table: {:?}",
            state_diags(&s)
        );
    }

    #[test]
    fn compaction_inside_turn_emits_no_state_diag() {
        let mut s = AppState::default();
        fire(&mut s, "agent_start");
        fire(&mut s, "compaction_start");
        assert_eq!(s.phase(), state::UiPhase::Compacting);
        fire(&mut s, "compaction_end");
        fire(&mut s, "agent_end");
        assert!(
            state_diags(&s).is_empty(),
            "compaction sequence tripped the table: {:?}",
            state_diags(&s)
        );
    }

    #[test]
    fn picker_opened_and_closed_mid_turn_emits_no_state_diag() {
        let mut s = AppState::default();
        fire(&mut s, "agent_start");
        s.note_phase(state::UiEvent::OpenModal);
        s.models_open = true;
        // Streaming outranks Modal — the turn stays the visible phase.
        assert_eq!(s.phase(), state::UiPhase::Streaming);
        s.note_phase(state::UiEvent::CloseModal);
        s.models_open = false;
        fire(&mut s, "agent_end");
        assert!(
            state_diags(&s).is_empty(),
            "picker mid-turn tripped the table: {:?}",
            state_diags(&s)
        );
    }

    #[test]
    fn dialog_answered_mid_turn_emits_no_state_diag() {
        let mut s = AppState::default();
        fire(&mut s, "agent_start");
        s.note_phase(state::UiEvent::OpenDialog);
        s.dialogs.push_back(state::Dialog::confirm(
            "id".to_string(),
            "t".to_string(),
            Some("m".to_string()),
        ));
        assert_eq!(s.phase(), state::UiPhase::Dialog);
        s.note_phase(state::UiEvent::CloseDialog);
        s.dialogs.pop_front();
        assert_eq!(s.phase(), state::UiPhase::Streaming);
        fire(&mut s, "agent_end");
        assert!(
            state_diags(&s).is_empty(),
            "mid-turn dialog tripped the table: {:?}",
            state_diags(&s)
        );
    }
}
