pub mod ansi;
pub mod cache;
pub mod caduceus;
pub mod cost;
pub mod crashlog;
pub mod diff;
pub mod engine_config;
pub mod heal;
pub mod janitor;
pub mod layout;
pub mod paste;
pub mod question;
pub mod review;
pub mod routes;
pub mod session_check;
pub mod settings;
pub mod stall;
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
    // Crash black-box: the hook reads the diag ring via the shared global
    // (registered inside DiagLog::new), so install order does not matter.
    crate::tui::crashlog::install(bridge.diag.clone());
    if let Some(line) = crate::tui::crashlog::crash_since_clean_exit() {
        state.notice = Some((
            format!("previous run crashed — {line} · /diag last"),
            "warning".into(),
        ));
    }
    if let Some((count, bytes)) = crate::tui::janitor::auto_run() {
        state.notice = Some((
            format!(
                "janitor: removed {count} stale item(s) · {}",
                crate::tui::cache::fmt_bytes(bytes)
            ),
            "info".into(),
        ));
    }
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
        return Err(io::Error::other("not a TTY"));
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
    let heal_policy = heal::Policy::from_env();
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
        state.apply_liveliness_gate();
        state.question_poll();
        state.step_fire(dt);
        terminal.draw(|frame| render(frame, state))?;

        // Dynamic redraw cadence: fresh screens (glimmer) and live fires
        // animate at ~11 fps; busy screens ride the 500 ms heartbeat.
        // Animate only when there is something to animate: real activity,
        // the decorative glimmer on a fresh screen, or a burning fire.
        let delay =
            if state.liveliness_active() || state.entries.is_empty() || state.fire_level > 0.0 {
                Duration::from_millis(90)
            } else {
                Duration::from_millis(500)
            };

        tokio::select! {
            maybe_input = input_rx.recv() => {
                match maybe_input {
                    Some(Event::Key(key)) => {
                        handle_key(key, state, bridge).await;
                        if state.heal_requested {
                            state.heal_requested = false;
                            state.recovery.degraded = false;
                            state.recovery.attempts.clear();
                            recover(
                                state,
                                bridge,
                                rx,
                                "operator requested recovery",
                                heal::Failure::Unknown,
                                &heal_policy,
                            )
                            .await;
                        }
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
                            let note: Option<(String, String)>;
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
                        state.note_wire_activity();
                        if handle_response(state, &resp) {
                            let _ = bridge.request(RpcCommand::GetState { id: None }).await;
                            let _ = bridge.request(RpcCommand::GetMessages { id: None }).await;
                        }
                        if state.recovery.phase == heal::Phase::Rehydrating
                            && resp.command.as_deref() == Some("get_messages")
                        {
                            finish_rehydrate(state);
                        }
                    }
                    Some(WireMessage::Event(event)) => {
                        state.note_wire_activity();
                        let auto_continue = handle_event(state, &event.event_type, &event.fields);
                        if let Some((failure, detail)) = state.recovery.pending_failure.take() {
                            handle_provider_failure(state, bridge, failure, &detail, &heal_policy)
                                .await;
                        }
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
                            state.held_resolve_queued();
                            state.note_prompt_sent(&message, None);
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
                        state.note_wire_activity();
                        handle_ui_request(state, &req, bridge).await;
                    }
                    None => {
                        // A closed reader stays closed until a respawn installs
                        // a new channel. Sleep through backoff instead of
                        // spinning hot on the dead channel.
                        let now = std::time::Instant::now();
                        if state.recovery.phase == heal::Phase::Degraded
                            || state
                                .recovery
                                .next_retry_at
                                .is_some_and(|at| now < at)
                        {
                            tokio::time::sleep(Duration::from_millis(300)).await;
                            continue;
                        }
                        state.note_prompt_failed();
                        if heal::self_heal_enabled() {
                            recover(
                                state,
                                bridge,
                                rx,
                                "agent process exited",
                                heal::Failure::ChildExit,
                                &heal_policy,
                            )
                            .await;
                        } else {
                            state.entries.push(state::ChatEntry::Diag(
                                "agent process exited".to_string(),
                            ));
                            terminal.draw(|frame| render(frame, state))?;
                            tokio::time::sleep(Duration::from_millis(1200)).await;
                            break;
                        }
                    }
                }
            }
            _ = tokio::time::sleep(delay) => {
                // heartbeat redraw — keeps streaming indicator/gauges alive
                // (90 ms on fresh screens so the watermark glimmer animates)
                state.probe_liveness(bridge.pi_pid());
                maybe_heal(state, bridge, rx, &heal_policy).await;
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

    if state.setting_menu.is_some() {
        if let Some(message) = settings::key(state, key.code) {
            if let Some(Some(request)) = settings::command(state, &message) {
                let _ = bridge.request(request).await;
            }
        }
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

    // /fx — fire/glimmer controls; every change persists immediately.
    if state.fx_open {
        let mut changed = false;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                state.note_phase(state::UiEvent::CloseModal);
                state.fx_open = false;
            }
            KeyCode::Up | KeyCode::Char('k') => state.fx_sel = state.fx_sel.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => state.fx_sel = (state.fx_sel + 1).min(3),
            KeyCode::Char(' ') | KeyCode::Right | KeyCode::Char('l') => {
                match state.fx_sel {
                    0 => {
                        state.set_fire("");
                    }
                    1 => {
                        state.set_fire("style");
                    }
                    2 => {
                        state.set_fire("tint");
                    }
                    _ => {
                        state.cycle_glimmer();
                    }
                }
                changed = true;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                // Left cycles the same rows (fire/style are one-way cycles;
                // glimmer cycles too — direction is cosmetic here).
                match state.fx_sel {
                    0 => {
                        state.set_fire("");
                    }
                    1 => {
                        state.set_fire("style");
                    }
                    2 => {
                        state.set_fire("tint");
                    }
                    _ => {
                        state.cycle_glimmer();
                    }
                }
                changed = true;
            }
            _ => {}
        }
        if changed {
            persist_fire(state);
            persist_glimmer(state);
        }
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

    // /routes — decision trail; esc/q closes, arrows/pg scroll.
    if state.routes_open {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                state.note_phase(state::UiEvent::CloseModal);
                state.routes_open = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.routes_sel = state.routes_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.routes_sel = state.routes_sel.saturating_add(1)
            }
            KeyCode::PageUp => state.routes_sel = state.routes_sel.saturating_sub(10),
            KeyCode::PageDown => state.routes_sel = state.routes_sel.saturating_add(10),
            KeyCode::Home | KeyCode::Char('g') => state.routes_sel = 0,
            KeyCode::End | KeyCode::Char('G') => state.routes_sel = usize::MAX,
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

    // Question panel (2026-11-13): top priority when visible — the
    // operator's answer goes to the staged question, nothing else.
    if state.question.is_some() {
        match key.code {
            KeyCode::Esc => state.question_cancel(),
            KeyCode::Up | KeyCode::Char('k') => {
                state.question_sel = state.question_sel.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => state.question_sel += 1,
            KeyCode::Enter => state.question_answer(),
            KeyCode::Char(c @ '1'..='9') => state.question_pick((c as u8 - b'0') as usize),
            _ => {}
        }
        let n = state
            .question
            .as_ref()
            .map(|q| q.options.len())
            .unwrap_or(0);
        if state.question_sel >= n {
            state.question_sel = n.saturating_sub(1);
        }
        return;
    }
    match key.code {
        // ^esc — quit. The only physical quit chord (owner-approved; some
        // desktops reserve it, /quit always works). MUST precede the plain
        // Esc arm or the guard never fires.
        KeyCode::Esc if key.modifiers.contains(KeyModifiers::CONTROL) => {
            if state.quit_may_proceed() {
                graceful_quit(state, bridge).await;
            }
        }
        KeyCode::Esc => {
            // Order (K4): the command menu closes first and leaves the composer
            // untouched; only then does Esc mean "stop the work" (2026-11-16).
            if state.cand_sel_logic_open() {
                state.dismiss_command_menu();
                state.notice = Some(("menu closed".to_string(), "info".to_string()));
            } else {
                // Operator stop: Esc ends the work NOW — abort the turn, drop
                // every queued/held message, and never let the settle deadline
                // restart pi. Idle Esc stays the deliberate no-op it has been
                // since the accidental-quit report.
                let busy = state.is_streaming || state.has_running_tool();
                if busy || !state.queued_prompts.is_empty() {
                    cancel_pending_dialogs(state, bridge).await;
                    let _ = bridge.request(RpcCommand::Abort { id: None }).await;
                    let dropped = state.operator_stop();
                    let now = std::time::Instant::now();
                    state.arm_abort(now, state::AbortSource::Operator);
                    if dropped > 0 {
                        state.entries.push(state::ChatEntry::Diag(format!(
                            "harness: stopped — dropped {dropped} queued message(s)"
                        )));
                    }
                    state.notice = Some((
                        if dropped > 0 {
                            format!("stopped · {dropped} queued dropped")
                        } else {
                            "stopped".to_string()
                        },
                        "warning".to_string(),
                    ));
                    stall::trail(
                        "abort.operator",
                        serde_json::json!({ "dropped": dropped, "busy": busy }),
                    );
                }
            }
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
        // Tab cycles the agent mode (owner request 2026-09-01). Two ways it
        // used to become a dead key (2026-11-16 operator report: "tabbing
        // through modes is blocked so often"):
        //   1. the guard tested `!input.is_empty()`, so ANY typed text blocked
        //      it — only a command line may keep the key (the popup owns those);
        //   2. the widget hint lags one round-trip, so a second Tab re-sent the
        //      same target and nothing visibly happened.
        // A missing hint is now reported instead of silently doing nothing.
        KeyCode::Tab if state.input.starts_with('/') => {}
        KeyCode::Tab => match state.next_agent_mode() {
            Some(next) if state.mode_switch_in_flight(&next) => {
                state.notice = Some((format!("agent: switching to {next}…"), "info".to_string()));
            }
            Some(next) => {
                state.note_mode_switch_sent(&next);
                let m = format!("/agent {next}");
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: m,
                        images: None,
                        streaming_behavior: None,
                    })
                    .await;
            }
            None => {
                state.notice = Some((
                    "agent: no mode hint yet — try /agent <name>".to_string(),
                    "warning".to_string(),
                ));
            }
        },
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
                state.held_push(&msg, state::HeldKind::FollowUp);
                let _ = bridge
                    .request(RpcCommand::FollowUp {
                        id: None,
                        message: msg,
                        images: images_for_send(state),
                    })
                    .await;
                state.notice = Some(("queued follow-up".to_string(), "info".to_string()));
            } else {
                state.note_prompt_sent(&msg, None);
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
            // D8 (2026-11-16): a UNIQUE partial command runs in one press —
            // `/q` → `/quit` — instead of completing and waiting for a second
            // Enter. Ambiguous prefixes still complete further below.
            let expanded = state.expand_unique_command(&msg);
            if expanded != msg {
                state.notice = Some((format!("{msg} → {expanded}"), "info".to_string()));
            }
            let msg = expanded;
            if let Some(request) = settings::command(state, &msg) {
                if let Some(request) = request {
                    let _ = bridge.request(request).await;
                }
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
            if msg == "/thinking-view" || msg.starts_with("/thinking-view ") {
                let args = msg.strip_prefix("/thinking-view").unwrap_or("").trim();
                state.thinking_view_command(args);
                return;
            }
            if msg == "/janitor" || msg.starts_with("/janitor ") {
                let dry = msg.starts_with("/janitor dry") || msg.starts_with("/janitor --dry");
                state.janitor_command(dry);
                return;
            }
            if msg == "/cache" || msg.starts_with("/cache ") {
                let args = msg.strip_prefix("/cache").unwrap_or("").trim();
                state.cache_command(args);
                return;
            }
            // /fx — fire + glimmer panel (local UI state only).
            if msg == "/fx" {
                state.note_phase(state::UiEvent::OpenModal);
                state.fx_sel = 0;
                state.fx_open = true;
                return;
            }
            // /answers — last INQUIRE answer panel (facts file, read-only).
            if msg == "/answers" {
                state.answers_open_modal();
                return;
            }
            // /probe — background cloud-probe status (read-only facts file).
            if msg == "/probe" {
                let doc = crate::tui::routes::load_gateway_health();
                let now = crate::tui::caduceus::now_ms();
                let total = doc.probes.len();
                let verified = doc
                    .probes
                    .values()
                    .filter(|p| {
                        p.kind == "ok"
                            && p.ok_at
                                .map(|t| now.saturating_sub(t) < 10 * 60_000)
                                .unwrap_or(false)
                    })
                    .count();
                let paused = doc.pauses.iter().filter(|(_, p)| p.until > now).count();
                let leader = doc
                    .probe_lease
                    .as_ref()
                    .map(|l| {
                        format!(
                            "pid {} · {}s left",
                            l.pid,
                            l.until.saturating_sub(now).div_ceil(1000)
                        )
                    })
                    .unwrap_or_else(|| "none".to_string());
                let next = doc
                    .probes
                    .values()
                    .filter_map(|p| p.next_probe_at)
                    .filter(|t| *t > now)
                    .min()
                    .map(|t| format!("{}s", (t - now) / 1000))
                    .unwrap_or_else(|| "—".to_string());
                state.entries.push(state::ChatEntry::Diag(format!(
                    "probe: leader {leader} · verified {verified}/{total} · invalid {} · paused owners {paused} · next in {next}",
                    doc.invalid.len()
                )));
                let mut lanes: Vec<(&String, &crate::tui::routes::ProbeFact)> =
                    doc.probes.iter().collect();
                lanes.sort_by_key(|(lane, _)| lane.to_string());
                for (lane, fact) in lanes.into_iter().take(12) {
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "  {} — {}",
                        lane,
                        crate::tui::routes::probe_label(Some(fact), now)
                    )));
                }
                return;
            }
            // /doctor — admission self-check (gateway, probes, pauses, budget).
            if msg == "/doctor" {
                let doc = crate::tui::routes::load_gateway_health();
                let now = crate::tui::caduceus::now_ms();
                let gateway = match crate::tui::routes::caduceus_gateway_up() {
                    Some(true) => "up".to_string(),
                    Some(false) => "DOWN".to_string(),
                    None => "unknown (no facts file)".to_string(),
                };
                let verified = doc.probes.values().filter(|p| p.kind == "ok").count();
                let cooling = doc
                    .probes
                    .values()
                    .filter(|p| p.kind == "cooling" && p.until.map(|u| u > now).unwrap_or(false))
                    .count();
                let paused = doc.pauses.iter().filter(|(_, p)| p.until > now).count();
                let leader = doc
                    .probe_lease
                    .as_ref()
                    .map(|l| format!("pid {}", l.pid))
                    .unwrap_or_else(|| "none".to_string());
                state.entries.push(state::ChatEntry::Diag(format!(
                    "doctor: gateway {gateway} · probe leader {leader} · verified {verified}/{} · cooling {cooling} · paused owners {paused} · invalid lanes {}",
                    doc.probes.len(),
                    doc.invalid.len()
                )));
                // Session-format self-check (2026-11-14).
                let session = crate::tui::session_check::check_current_session(&state.cwd);
                if let Some(path) = &session.path {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let verdict = if session.ok() {
                        "format ok".to_string()
                    } else {
                        format!(
                            "{} issue(s): {}",
                            session.issues.len(),
                            session.issues.join("; ")
                        )
                    };
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "  session {name} — {} entries · {} messages · {}/{} tool pairs · {verdict}",
                        session.entries,
                        session.messages,
                        session.tool_calls,
                        session.tool_results
                    )));
                } else {
                    state.entries.push(state::ChatEntry::Diag(
                        "  session — none found for this cwd yet".to_string(),
                    ));
                }
                // Rust core sidecar presence (fail-open by design).
                let core_mode = std::env::var("OFFICINA_ROUTER_CORE").unwrap_or_default();
                let core_bin = std::env::var("OFFICINA_CORE_BIN").ok().unwrap_or_else(|| {
                    let mut p =
                        std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
                    p.push(".local");
                    p.push("bin");
                    p.push("vitriol-core-rpc");
                    p.to_string_lossy().to_string()
                });
                state.entries.push(state::ChatEntry::Diag(format!(
                    "  core: mode {} · binary {}",
                    if core_mode.eq_ignore_ascii_case("rust") {
                        "rust"
                    } else {
                        "ts (set OFFICINA_ROUTER_CORE=rust to try the sidecar)"
                    },
                    if std::path::Path::new(&core_bin).exists() {
                        format!("present ({core_bin})")
                    } else {
                        "missing (fallback active)".to_string()
                    }
                )));
                for (owner, pause) in doc.pauses.iter().filter(|(_, p)| p.until > now).take(6) {
                    let safe = pause
                        .safe_bytes
                        .map(|b| format!(" · safe {} KB", b / 1024))
                        .unwrap_or_default();
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "  paused {owner} — {}m left{safe}",
                        pause.until.saturating_sub(now).div_ceil(60_000)
                    )));
                }
                return;
            }
            // /routes — Caduceus decision trail (read-only facts file).
            if msg == "/routes" {
                state.routes = Some(crate::tui::routes::load_recent(&chrono_day(), 400));
                state.routes_sel = 0;
                state.note_phase(state::UiEvent::OpenModal);
                state.routes_open = true;
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
                        .push(state::ChatEntry::Error(format!("session scan failed: {e}"))),
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
                    if state.quit_may_proceed() {
                        graceful_quit(state, bridge).await;
                    }
                    return;
                }
                "/recover" | "/heal" => {
                    state.heal_requested = true;
                    state.notice = Some((
                        "harness: manual recovery requested".to_string(),
                        "warning".to_string(),
                    ));
                    return;
                }
                "/diag last" => {
                    let path = crate::tui::crashlog::crash_log_path();
                    match std::fs::read_to_string(&path) {
                        Ok(text) => {
                            let lines: Vec<&str> = text.lines().collect();
                            let tail = lines.len().saturating_sub(20);
                            for line in &lines[tail..] {
                                state
                                    .entries
                                    .push(state::ChatEntry::Diag((*line).to_string()));
                            }
                        }
                        Err(_) => state
                            .entries
                            .push(state::ChatEntry::Diag("no crash log".into())),
                    }
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
                                    .push(state::ChatEntry::Error("session scan failed".into()));
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
                    state.held_push(&msg, state::HeldKind::Queued);
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
                    state.held_push(&msg, state::HeldKind::Steer);
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
                state.note_prompt_sent(&msg, images.clone());
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
                state.note_input_typed();
            }
        }
        KeyCode::Delete => {
            if state.cursor < state.input.chars().count() {
                remove_char_at(&mut state.input, state.cursor);
                state.note_input_typed();
            }
        }
        KeyCode::Left => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Right
            if state.cand_sel_logic_open() && state.cursor == state.input.chars().count() =>
        {
            state.complete_command();
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
                    state.note_input_typed();
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
            state.entries.push(state::ChatEntry::Error(format!(
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
            state.entries.push(state::ChatEntry::Error(format!(
                "import failed to run node: {e}"
            )));
            return;
        }
        Err(e) => {
            state
                .entries
                .push(state::ChatEntry::Error(format!("import task failed: {e}")));
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
    // Deliberate quit: the black-box knows this was not a crash.
    crate::tui::crashlog::stamp_clean_exit();
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

/// Recovery + stall watchdog (2026-11-15). Aborts a stalled turn, detects a
/// wire-idle hang, and escalates to a bounded pi respawn when abort cannot
/// settle. With OFFICINA_SELF_HEAL=0 the pre-self-heal behavior is kept.
async fn maybe_heal(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<WireMessage>,
    policy: &heal::Policy,
) {
    let now = std::time::Instant::now();

    // Abort-settle deadline runs even with the tool watchdog disabled: an
    // unresponsive abort must never leave 'working…' spinning forever
    // (2026-11-13, operator report). Only a WATCHDOG abort may escalate to a
    // child restart; an operator Esc releases the UI and says what is still
    // running (E1/E3, 2026-11-16).
    if let Some(source) = state.abort_due(now) {
        state.abort_deadline = None;
        state.note_phase(state::UiEvent::AgentEnd);
        state.is_streaming = false;
        state.is_compacting = false;
        state.expected_tool_calls.clear();
        let abandoned = state.abandon_running_tools("abort did not settle — released");
        if source == state::AbortSource::Operator {
            let still = state
                .oldest_running_tool()
                .map(|tool| {
                    format!(
                        "pi is still finishing {} ({}s)",
                        tool.name,
                        tool.age.as_secs()
                    )
                })
                .unwrap_or_else(|| "pi has not confirmed the stop".to_string());
            stall::trail(
                "abort.operator_unsettled",
                serde_json::json!({ "abandoned": abandoned, "still": still }),
            );
            state.entries.push(state::ChatEntry::Warn(format!(
                "harness: stopped — {still}; no restart was attempted (use /recover if it stays stuck)"
            )));
            return;
        } else if heal::self_heal_enabled() {
            state.entries.push(state::ChatEntry::Warn(format!(
                "harness watchdog: abort did not settle after 10s — restarting pi{}; session preserved, transcript restored",
                if abandoned > 0 {
                    format!(", {abandoned} tool call(s) marked cancelled")
                } else {
                    String::new()
                }
            )));
            let failure = match state.recovery.reason.as_str() {
                "wire idle" => heal::Failure::WireIdle,
                "tool stall" => heal::Failure::ToolStall,
                _ => heal::Failure::AbortUnsettled,
            };
            recover(state, bridge, rx, "abort did not settle", failure, policy).await;
            return;
        }
        state.entries.push(state::ChatEntry::Warn(format!(
            "harness watchdog: abort did not settle after 10s — UI released{}; \
             the pi child may still be finishing in the background. If it stays \
             unresponsive, /quit then relaunch Officina.",
            if abandoned > 0 {
                format!(", {abandoned} tool call(s) marked cancelled")
            } else {
                String::new()
            }
        )));
    }

    // A failed respawn is retried after its backoff slot.
    if state.recovery.next_retry_at.is_some_and(|at| now >= at)
        && !matches!(
            state.recovery.phase,
            heal::Phase::Respawning | heal::Phase::Rehydrating
        )
    {
        state.recovery.next_retry_at = None;
        let reason = state.recovery.reason.clone();
        recover(
            state,
            bridge,
            rx,
            &reason,
            heal::Failure::AbortUnsettled,
            policy,
        )
        .await;
    }

    // Rehydration that never completes is a failed recovery, not a hung UI.
    if state.recovery.phase == heal::Phase::Rehydrating
        && state
            .recovery
            .rehydrate_deadline
            .is_some_and(|deadline| now >= deadline)
    {
        state
            .recovery
            .degrade("session restore timed out after respawn");
        state.entries.push(state::ChatEntry::Warn(
            "harness: pi restarted but the session transcript did not restore — /recover retries, /quit exits"
                .to_string(),
        ));
    }

    // Engine-start readiness probe (throttled; only while a start is pending).
    if state.recovery.engine_deadline.is_some()
        && state.recovery.engine_next_probe.is_none_or(|at| now >= at)
    {
        state.recovery.engine_next_probe = Some(now + Duration::from_secs(3));
        match heal::engine_activity().await {
            heal::EngineActivity::UpBusy | heal::EngineActivity::UpIdle => {
                state.recovery.engine_deadline = None;
                state.recovery.engine_next_probe = None;
                state.notice = Some((
                    "harness: engine is ready — resuming".to_string(),
                    "info".to_string(),
                ));
            }
            heal::EngineActivity::Down => {
                if state
                    .recovery
                    .engine_deadline
                    .is_some_and(|deadline| now >= deadline)
                {
                    state.recovery.engine_deadline = None;
                    state.recovery.engine_next_probe = None;
                    state.entries.push(state::ChatEntry::Warn(
                        "harness: engine did not become ready — /engine status for details"
                            .to_string(),
                    ));
                }
            }
        }
    }

    // One-shot replay after a recovered model/engine failure.
    if state.recovery.pending_replay.is_some() {
        let due = state
            .recovery
            .pending_replay
            .as_ref()
            .is_some_and(|p| now >= p.at);
        let too_old = state
            .recovery
            .pending_replay
            .as_ref()
            .is_some_and(|p| now.duration_since(p.at) > Duration::from_secs(90));
        let needs_lapis = state
            .recovery
            .pending_replay
            .as_ref()
            .is_some_and(|p| p.requires_lapis);
        if due
            && !state.is_streaming
            && state.recovery.phase == heal::Phase::Idle
            && (!needs_lapis || state.model_is_lapis())
            && !state.replay_has_side_effects()
        {
            state.recovery.pending_replay = None;
            if let Some((text, images)) = state.take_replayable_prompt() {
                state.entries.push(state::ChatEntry::Diag(
                    "harness: replaying the interrupted prompt".to_string(),
                ));
                let _ = bridge
                    .request(RpcCommand::Prompt {
                        id: None,
                        message: text,
                        images,
                        streaming_behavior: None,
                    })
                    .await;
            }
        } else if too_old {
            state.recovery.pending_replay = None;
        }
    }

    // Wire-idle hang: pi is streaming but no wire message arrived and no
    // tool watch is armed. Probe the engine first so a long prefill is never
    // mistaken for a hang.
    if state.wire_idle_due(now, policy.wire_idle)
        && !state.abort_pending()
        && state.recovery.engine_deadline.is_none()
    {
        let activity = heal::engine_activity().await;
        if activity == heal::EngineActivity::UpBusy {
            state.grant_wire_grace(Duration::from_secs(120));
            state.notice = Some((
                "harness: engine is still prefilling — wire-idle watchdog extended".to_string(),
                "info".to_string(),
            ));
        } else if activity == heal::EngineActivity::Down
            && state.model_is_lapis()
            && policy.engine_start
            && state.recovery.engine_deadline.is_none()
        {
            // A hung request with lapis selected is the one case where the
            // wire-idle detector may bring the engine up. Every other model
            // stays manual (operator rule 2026-11-15).
            let active = engine_unit_active().await;
            let verb = if active {
                if policy.engine_restart {
                    "restart"
                } else {
                    ""
                }
            } else {
                "start"
            };
            if verb.is_empty() {
                state.entries.push(state::ChatEntry::Diag(
                    "harness: engine unit is active but unreachable — not restarting automatically (OFFICINA_HEAL_ENGINE_RESTART=0)"
                        .to_string(),
                ));
            } else {
                match engine_unit(verb).await {
                    Ok(()) => {
                        state.recovery.engine_deadline = Some(now + Duration::from_secs(90));
                        state.recovery.engine_next_probe = Some(now + Duration::from_secs(3));
                        state.grant_wire_grace(Duration::from_secs(120));
                        state.notice = Some((
                            format!("harness: engine unreachable — {verb}ing it (lapis selected)"),
                            "warning".to_string(),
                        ));
                    }
                    Err(e) => state.entries.push(state::ChatEntry::Error(format!(
                        "harness: engine {verb} failed: {e}"
                    ))),
                }
            }
        } else {
            state.entries.push(state::ChatEntry::Warn(format!(
                "harness watchdog: no harness output for {}s and pi is idle — aborting (auto-recovery armed)",
                now.duration_since(state.last_wire_at).as_secs()
            )));
            cancel_pending_dialogs(state, bridge).await;
            let _ = bridge.request(RpcCommand::Abort { id: None }).await;
            state.note_prompt_failed();
            state.recovery.phase = heal::Phase::Aborting;
            state.recovery.reason = "wire idle".to_string();
            state.arm_abort(now, state::AbortSource::Watchdog);
            return;
        }
    }

    if state.tool_watchdog_secs == 0 {
        return;
    }
    let mut stalled = false;

    // Stage 0: an assistant turn announced a tool call, but pi never emitted
    // `tool_execution_start`. That is the exact "persisted tool call, nothing
    // happened" failure; it has no running tool entry to age out.
    let start_limit = std::time::Duration::from_secs(state.tool_watchdog_secs.clamp(15, 60));
    if let Some((id, age)) = state.oldest_expected_tool_call() {
        if age >= start_limit && state.watchdog_fired.insert(format!("expected:{id}")) {
            state.entries.push(state::ChatEntry::Warn(format!(
                "harness watchdog: tool call {} never started after {}s — cancelling and aborting the turn",
                id.chars().take(8).collect::<String>(),
                age.as_secs()
            )));
            stalled = true;
        }
    }

    if let Some(tool) = state.oldest_running_tool() {
        let base = std::time::Duration::from_secs(state.tool_watchdog_secs);
        let base_due = tool.age >= tool.limit(base);
        // Decisive fast path: a silent, lone tool needs a couple of samples, not
        // the full budget, to prove it spawned nothing (2026-11-15). Only the
        // NeverStarted verdict may act before `base_due` — anything else waits.
        let decisive_due = tool.silent
            && state.running_tool_count() == 1
            && tool.age >= std::time::Duration::from_secs(stall::UNSTARTED_SECS);
        if base_due || decisive_due {
            let case = StallCase {
                tool: &tool,
                policy,
                now,
                decisive_only: !base_due,
            };
            maybe_grace_or_abort(state, bridge, &case).await;
            // An abort was just armed: the settle deadline owns the next step.
            if state.abort_pending() {
                return;
            }
        }
    } else if state.expected_tool_calls.is_empty() {
        state.abort_deadline = None;
    }

    if !stalled {
        return;
    }
    abort_for_stall(state, bridge, "tool stall", now).await;
}

/// Abort a turn that cannot make progress: close every dialog, ask pi to stop,
/// and arm the settle deadline that escalates to recovery.
async fn abort_for_stall(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    reason: &'static str,
    now: std::time::Instant,
) {
    cancel_pending_dialogs(state, bridge).await;
    let _ = bridge.request(RpcCommand::Abort { id: None }).await;
    state.note_prompt_failed();
    state.recovery.phase = heal::Phase::Aborting;
    state.recovery.reason = reason.to_string();
    state.arm_abort(now, state::AbortSource::Watchdog);
    stall::trail(
        "abort.sent",
        serde_json::json!({ "reason": reason, "recovery": state.recovery.reason }),
    );
}

/// One stall evaluation's inputs, bundled so the ladder stays under the
/// parameter/cyclomatic limits (Praetor Datalog 4; 2026-11-15).
struct StallCase<'a> {
    tool: &'a state::RunningTool,
    policy: &'a heal::Policy,
    now: std::time::Instant,
    /// Only the decisive `NeverStarted` verdict may act (fast path).
    decisive_only: bool,
}

/// Evidence-gated stall ladder (2026-11-15).
///
/// A tool that reports running but has spawned no process can never start, so
/// it is aborted immediately. Anything demonstrably working (pi computing, a
/// busy engine) keeps its time. A flat, silent tool gets exactly one bounded
/// grace before the turn is aborted; the trail records every step.
async fn maybe_grace_or_abort(state: &mut AppState, bridge: &mut RpcBridge, case: &StallCase<'_>) {
    let key = &case.tool.key;
    if state.watchdog_fired.contains(key) || state.stall_parked(key, case.now) {
        return; // already decided, or the ladder is parked
    }
    let verdict = evaluate_stall(state, bridge, case).await;
    apply_stall_verdict(state, bridge, case, verdict).await;
}

/// Gather liveness evidence and record the decision breadcrumb.
async fn evaluate_stall(
    state: &AppState,
    bridge: &RpcBridge,
    case: &StallCase<'_>,
) -> stall::StallVerdict {
    let pi_flat = state.pi_is_flat();
    let engine_busy = pi_flat && heal::engine_activity().await == heal::EngineActivity::UpBusy;
    let evidence = stall::Evidence {
        pi_flat,
        engine_busy,
        descendants: bridge.pi_pid().and_then(state::subtree_process_count),
    };
    let verdict = stall::stall_verdict(
        case.tool.silent,
        state.running_tool_count() == 1,
        state.stall_graced.contains(&case.tool.key),
        evidence,
    );
    stall::trail(
        "stall.evaluate",
        serde_json::json!({
            "key": case.tool.key,
            "name": case.tool.name,
            "age_s": case.tool.age.as_secs(),
            "silent": case.tool.silent,
            "verdict": format!("{verdict:?}"),
            "pi_flat": evidence.pi_flat,
            "engine_busy": evidence.engine_busy,
            "descendants": evidence.descendants,
        }),
    );
    verdict
}

/// Human-readable consequence of one verdict. Pure: no side effects, so the
/// ladder stays readable and each arm is testable by inspection.
fn stall_notice(verdict: stall::StallVerdict, name: &str, age_s: u64, grace_s: u64) -> String {
    match verdict {
        stall::StallVerdict::Working => format!(
            "harness: {name} still looks alive ({age_s}s) — watchdog waiting; a busy engine or active pi tree is real work"
        ),
        stall::StallVerdict::Grace => format!(
            "harness: {name} silent for {age_s}s with no process evidence — holding {grace_s}s before any abort (pass `timeout` for long builds)"
        ),
        stall::StallVerdict::NeverStarted => format!(
            "harness watchdog: {name} never spawned a process ({age_s}s, no output) — cancelling; the prompt will be replayed once"
        ),
        stall::StallVerdict::Abort => format!(
            "harness watchdog: {name} idle {age_s}s with no result — cancelling and aborting the turn"
        ),
    }
}

/// Announce and act on one verdict: park, hold, or abort the turn.
async fn apply_stall_verdict(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    case: &StallCase<'_>,
    verdict: stall::StallVerdict,
) {
    let tool = case.tool;
    let key = tool.key.clone();
    // Not decisive yet: hold the full budget, never spend the grace, say nothing.
    if case.decisive_only && verdict != stall::StallVerdict::NeverStarted {
        state.park_stall(&key, Duration::from_secs(30));
        return;
    }
    let text = stall_notice(
        verdict,
        &tool.name,
        tool.age.as_secs(),
        case.policy.stall_grace.as_secs(),
    );
    match verdict {
        stall::StallVerdict::Working => {
            state.park_stall(&key, Duration::from_secs(60));
            state.entries.push(state::ChatEntry::Diag(text));
        }
        stall::StallVerdict::Grace => {
            state.stall_graced.insert(key.clone());
            state.park_stall(&key, case.policy.stall_grace);
            state.entries.push(state::ChatEntry::Warn(text));
        }
        stall::StallVerdict::NeverStarted => {
            state.watchdog_fired.insert(key);
            state.note_stall_unexecuted();
            state.entries.push(state::ChatEntry::Warn(text));
            abort_for_stall(state, bridge, "tool never started", case.now).await;
        }
        stall::StallVerdict::Abort => {
            state.watchdog_fired.insert(key);
            if tool.silent {
                state.note_stall_unexecuted();
            }
            state.entries.push(state::ChatEntry::Warn(text));
            abort_for_stall(state, bridge, "tool stall", case.now).await;
        }
    }
}

/// Kill the tree, reopen the session pi was writing and rehydrate the view.
/// Bounded by the recovery policy; exhaustion leaves the TUI usable.
async fn recover(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<WireMessage>,
    reason: &str,
    _failure: heal::Failure,
    policy: &heal::Policy,
) {
    if !heal::self_heal_enabled()
        || matches!(
            state.recovery.phase,
            heal::Phase::Respawning | heal::Phase::Rehydrating
        )
    {
        return;
    }
    let now = std::time::Instant::now();
    if !state.recovery.can_attempt(now, policy) {
        state
            .recovery
            .degrade(&format!("recovery budget exhausted after {reason}"));
        state.entries.push(state::ChatEntry::Warn(format!(
            "harness degraded: recovery failed after {} attempts — input is preserved; /recover retries, /quit exits",
            policy.max_attempts
        )));
        return;
    }
    state.recovery.record_attempt(now, reason);
    state.recovery.pending_failure = None;
    let attempt = state.recovery.attempts.len();
    state.entries.push(state::ChatEntry::Warn(format!(
        "harness: {reason} — recovering (attempt {attempt}/{})",
        policy.max_attempts
    )));
    cancel_pending_dialogs(state, bridge).await;

    let session = state.session_file.clone().or_else(|| {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .and_then(|home| crate::session_guard::latest_session_in(&home, &state.cwd))
    });
    let resume = session.and_then(|path| {
        let (resume, note) = crate::session_guard::repair_for_resume(&path);
        if let Some(note) = note {
            state.entries.push(state::ChatEntry::Diag(note));
        }
        resume
    });

    match bridge.respawn(resume.as_deref()).await {
        Ok(new_rx) => {
            *rx = new_rx;
            state.session_file = resume;
            state.reset_for_rehydrate();
            state.recovery.phase = heal::Phase::Rehydrating;
            state.recovery.rehydrate_deadline = Some(now + Duration::from_secs(30));
            state.recovery.engine_deadline = None;
            state.recovery.engine_next_probe = None;
            let _ = bridge.request(RpcCommand::GetState { id: None }).await;
            let _ = bridge.request(RpcCommand::GetMessages { id: None }).await;
            let _ = bridge.request(RpcCommand::GetCommands { id: None }).await;
            let _ = bridge
                .request(RpcCommand::GetSessionStats { id: None })
                .await;
            state.entries.push(state::ChatEntry::Diag(
                "harness: pi restarted — restoring the session transcript".to_string(),
            ));
            if policy.replay && state.last_prompt_replayable() {
                state.recovery.pending_replay = Some(heal::PendingReplay {
                    at: now,
                    requires_lapis: false,
                });
            }
        }
        Err(e) => {
            let msg = format!("harness recover failed: {e}");
            stall::trail(
                "respawn.fail",
                serde_json::json!({ "attempt": attempt, "error": msg }),
            );
            state.recovery.last_error = Some(msg.clone());
            state.recovery.phase = heal::Phase::Idle;
            state.recovery.next_retry_at = Some(now + policy.backoff_for(attempt));
            state.entries.push(state::ChatEntry::Error(msg));
        }
    }
}

fn finish_rehydrate(state: &mut AppState) {
    stall::trail(
        "rehydrate.done",
        serde_json::json!({ "entries": state.entries.len() }),
    );
    state.recovery.recovered();
    state.entries.push(state::ChatEntry::Diag(
        "harness: session restored".to_string(),
    ));
    if let Some(p) = state.recovery.pending_replay.as_mut() {
        p.at = std::time::Instant::now();
    }
}

/// Classified provider failure → one bounded environment action.
async fn handle_provider_failure(
    state: &mut AppState,
    bridge: &mut RpcBridge,
    failure: heal::Failure,
    detail: &str,
    policy: &heal::Policy,
) {
    if !heal::self_heal_enabled() {
        return;
    }
    let now = std::time::Instant::now();
    if state.recovery.failure_seen(failure, now) {
        return;
    }
    let replayable = state.last_prompt_replayable();
    match failure {
        heal::Failure::EngineDown | heal::Failure::NoPhysicalModel => {
            // HARD RULE (operator 2026-11-15): only officina/lapis may
            // auto-start the engine. Every other selection stays manual.
            if !state.model_is_lapis() {
                state.entries.push(state::ChatEntry::Diag(
                    "harness: engine unavailable — auto-start skipped (select officina/lapis to enable it)"
                        .to_string(),
                ));
                return;
            }
            if !policy.engine_start {
                state.entries.push(state::ChatEntry::Diag(
                    "harness: engine unavailable — auto-start disabled (OFFICINA_HEAL_ENGINE=0)"
                        .to_string(),
                ));
                return;
            }
            let active = engine_unit_active().await;
            let verb = if active {
                if failure == heal::Failure::NoPhysicalModel {
                    state.entries.push(state::ChatEntry::Diag(format!(
                        "harness: no physical model available — {detail}"
                    )));
                    return;
                }
                if policy.engine_restart {
                    "restart"
                } else {
                    state.entries.push(state::ChatEntry::Diag(
                        "harness: engine unit is active — not restarting automatically (set OFFICINA_HEAL_ENGINE_RESTART=1 to allow)"
                            .to_string(),
                    ));
                    return;
                }
            } else {
                "start"
            };
            match engine_unit(verb).await {
                Ok(()) => {
                    state.recovery.engine_deadline = Some(now + Duration::from_secs(90));
                    state.recovery.engine_next_probe = Some(now);
                    if replayable && policy.replay {
                        state.recovery.pending_replay = Some(heal::PendingReplay {
                            at: now,
                            requires_lapis: true,
                        });
                    }
                    state.notice = Some((
                        format!("harness: {verb}ing the engine — waiting for readiness"),
                        "warning".to_string(),
                    ));
                }
                Err(e) => {
                    state.entries.push(state::ChatEntry::Error(format!(
                        "harness: engine {verb} failed: {e}"
                    )));
                }
            }
        }
        heal::Failure::ModelNotFound => {
            if state.model_is_lapis() {
                state.entries.push(state::ChatEntry::Diag(format!(
                    "harness: model unavailable — {detail}"
                )));
                return;
            }
            let _ = bridge
                .request(RpcCommand::SetModel {
                    id: None,
                    provider: "officina".to_string(),
                    model_id: "lapis".to_string(),
                })
                .await;
            if replayable && policy.replay {
                state.recovery.pending_replay = Some(heal::PendingReplay {
                    at: now + Duration::from_secs(1),
                    requires_lapis: true,
                });
            }
            state.notice = Some((
                "harness: model unavailable — switching to officina/lapis".to_string(),
                "warning".to_string(),
            ));
        }
        heal::Failure::RateLimit => {
            // A quota/rate error is switchable, not terminal (2026-11-16:
            // "I want Caduceus to just pick me a model and run with it so I
            // don't have to resume sessions"). Same shape as ModelNotFound:
            // go local, replay. The hint the gateway already sends decides how
            // long to wait; the budget stops an endless loop.
            if !state.recovery.take_rate_limit_replay(policy) {
                state.recovery.degrade("rate limited — replay budget spent");
                state.entries.push(state::ChatEntry::Error(format!(
                    "harness: still rate limited after {} replays — {detail}",
                    state.recovery.rate_limit_replays
                )));
                return;
            }
            let wait =
                heal::quota_hint(detail).unwrap_or_else(|| std::time::Duration::from_secs(90));
            if !state.model_is_lapis() {
                let _ = bridge
                    .request(RpcCommand::SetModel {
                        id: None,
                        provider: "officina".to_string(),
                        model_id: "lapis".to_string(),
                    })
                    .await;
                if replayable && policy.replay {
                    state.recovery.pending_replay = Some(heal::PendingReplay {
                        at: now + std::time::Duration::from_secs(1),
                        requires_lapis: true,
                    });
                }
                state.notice = Some((
                    format!(
                        "harness: rate limited — switching to officina/lapis (was {})",
                        detail.chars().take(60).collect::<String>()
                    ),
                    "warning".to_string(),
                ));
            } else {
                if replayable && policy.replay {
                    state.recovery.pending_replay = Some(heal::PendingReplay {
                        at: now + wait,
                        requires_lapis: false,
                    });
                }
                state.notice = Some((
                    format!(
                        "harness: rate limited — retrying in {}s",
                        wait.as_secs().max(1)
                    ),
                    "warning".to_string(),
                ));
            }
        }
        _ => {
            state.notice = Some((format!("harness: {detail}"), "warning".to_string()));
        }
    }
}

async fn engine_unit_active() -> bool {
    tokio::process::Command::new("systemctl")
        .args(["is-active", "vitriol-server.service"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "active")
        .unwrap_or(false)
}

async fn engine_unit(action: &str) -> Result<()> {
    let out = tokio::process::Command::new("systemctl")
        .args([action, "vitriol-server.service"])
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("systemctl {action}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "systemctl {action}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
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
            // Thinking blocks (2026-11-13): a click inside a block closes
            // or reopens the trace instead of starting a selection.
            if !over_sidebar(state, m.column, m.row) && state.thinking_toggle_at(m.row) {
                return;
            }
            // Tool blocks (2026-11-13): click opens the full args/output,
            // click again collapses to the one-line form.
            if !over_sidebar(state, m.column, m.row) && state.tool_toggle_at(m.row) {
                return;
            }
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
            state.entries.push(state::ChatEntry::Error(format!(
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
            // A fresh session has no status band yet; keeping the previous
            // session's settled geometry would render its rows for a frame.
            state.reset_band();
            state.input.clear();
            state.cursor = 0;
            state.resume_entries.clear();
            state.tree_lines.clear();
            state.tree_open = false;
            state.tree_sel = 0;
            return true;
        }
        Some("get_state") => {
            state.apply_state(crate::rpc::protocol::RpcSessionState {
                model: data
                    .get("model")
                    .and_then(|m| serde_json::from_value(m.clone()).ok()),
                thinking_level: data
                    .get("thinkingLevel")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                is_streaming: data
                    .get("isStreaming")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                is_compacting: data
                    .get("isCompacting")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                session_id: data
                    .get("sessionId")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                session_file: data
                    .get("sessionFile")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                session_name: data
                    .get("sessionName")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                message_count: data.get("messageCount").and_then(|v| v.as_u64()),
            });
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
                state.entries.push(state::ChatEntry::Error(
                    "no models available — check provider/auth".to_string(),
                ));
                state.models_open = false;
                return false;
            }
            // Start on the currently selected row so Enter is a no-op unless
            // the user moves. Position is in the GROUPED display order.
            let current = state
                .model
                .as_ref()
                .map(|m| (m.provider.clone(), m.id.clone()));
            state.models = models;
            state.models_filter.clear();
            state.models_sel = current
                .and_then(|(provider, id)| {
                    state
                        .filtered_models()
                        .iter()
                        .position(|m| m.provider == provider && m.id == id)
                })
                .unwrap_or(0);
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
/// Optional wire-event trace (2026-11-14, conformance harness): with
/// OFFICINA_TRACE_EVENTS=1 every pi event is appended as one JSON line to
/// ~/.vitriol/officina/traces/events-<pid>.jsonl (bounded to 20 MB). These
/// traces become replay fixtures for `handle_event` conformance tests.
fn capture_event(event_type: &str, fields: &serde_json::Value) {
    if std::env::var_os("OFFICINA_TRACE_EVENTS").is_none() {
        return;
    }
    let mut dir = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    dir.push(".vitriol");
    dir.push("officina");
    dir.push("traces");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join(format!("events-{}.jsonl", std::process::id()));
    if std::fs::metadata(&path)
        .map(|m| m.len() > 20 * 1024 * 1024)
        .unwrap_or(false)
    {
        return;
    }
    let line = serde_json::json!({"event": event_type, "fields": fields});
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
    }
}

fn handle_event(state: &mut AppState, event_type: &str, fields: &serde_json::Value) -> bool {
    capture_event(event_type, fields);
    match event_type {
        "agent_start" => {
            state.note_phase(state::UiEvent::AgentStart);
            state.is_streaming = true;
            state.held_clear(Some(state::HeldKind::FollowUp));
            state.watchdog_fired.clear();
            state.clear_stall_state();
            state.abort_deadline = None;
            state.expected_tool_calls.clear();
        }
        "agent_end" | "agent_settled" => {
            state.mark_activity();
            state.note_phase(state::UiEvent::AgentEnd);
            state.is_streaming = false;
            state.held_clear(None);
            state.refresh_ghost();
            state.refresh_answers_notice();
            state.abort_deadline = None;
            if state.recovery.phase == heal::Phase::Aborting {
                stall::trail(
                    "abort.settled",
                    serde_json::json!({ "reason": state.recovery.reason.clone() }),
                );
                state.recovery.recovered();
                state.entries.push(state::ChatEntry::Diag(
                    "harness: recovered by abort".to_string(),
                ));
            }
            state.settle_last_prompt();
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
        "message_start" => {
            // A steered message is inserted at the next step; the wire
            // opening a message means it landed (2026-11-13).
            state.held_clear(Some(state::HeldKind::Steer));
        }
        "message_update" => {
            state.mark_activity();
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
            // Model-call failures used to end the turn SILENTLY (owner
            // report 2026-11-14: a bad model selection "immediately halts"
            // with no explanation). Surface pi's errorMessage as a real
            // error entry; retries repeat the same string, so collapse
            // consecutive duplicates.
            if role == "assistant"
                && msg
                    .and_then(|m| m.get("stopReason"))
                    .and_then(|v| v.as_str())
                    == Some("error")
            {
                let detail = msg
                    .and_then(|m| m.get("errorMessage"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("model call failed (no errorMessage)");
                let entry = format!("model error: {detail}");
                let dup =
                    matches!(state.entries.last(), Some(state::ChatEntry::Error(t)) if *t == entry);
                if !dup {
                    state.entries.push(state::ChatEntry::Error(entry));
                }
                state.note_prompt_failed();
                state.recovery.pending_failure =
                    Some((heal::classify_error(detail), detail.to_string()));
            }
            if role == "assistant" {
                if let Some(content) = msg.and_then(|m| m.get("content")) {
                    state.note_expected_tool_calls(content);
                    let text = content_text(content);
                    let has_text = !text.is_empty();
                    let have_streamed = matches!(&state.entries.last(), Some(e)
                        if matches!(e, state::ChatEntry::Assistant(t) if !t.is_empty())
                            || matches!(e, state::ChatEntry::Thinking { text: t, .. } if !t.is_empty()));
                    if has_text && !have_streamed {
                        state.entries.push(state::ChatEntry::Assistant(text));
                    }
                    if has_text {
                        state.note_prompt_assistant_output();
                    }
                }
            }
        }
        "tool_execution_start" => {
            state.mark_activity();
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
            stall::trail(
                "tool.start",
                serde_json::json!({
                    "id": tool_call_id,
                    "name": name,
                    "timeout_s": args.get("timeout"),
                }),
            );
        }
        "tool_execution_update" => {
            state.mark_activity();
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
            stall::trail(
                "tool.end",
                serde_json::json!({
                    "id": tool_call_id,
                    "name": name,
                    "is_error": is_err,
                }),
            );
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
            // pi's edit tool returns the canonical diff; keep it for the
            // block (2026-11-14).
            let diff = fields
                .get("result")
                .and_then(|r| r.get("details"))
                .and_then(|d| d.get("diff"))
                .and_then(|v| v.as_str())
                .map(String::from);
            state.note_tool_finished(tool_call_id);
            state.set_tool_diff(tool_call_id, diff);
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
            state.compaction_started_at = Some(std::time::Instant::now());
            // Re-arm the one-shot auto-continue for THIS compaction.
            state.compaction_auto_armed = true;
        }
        "compaction_end" => {
            state.note_phase(state::UiEvent::CompactionEnd);
            state.is_compacting = false;
            state.compaction_started_at = None;
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
                    .push(state::ChatEntry::Error(format!("compaction failed: {}", e)));
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
            // GRIND handoff (2026-11-14): the grind extension owns the
            // post-compaction continuation while that mode is active — two
            // continuation prompts would race.
            let grind_active = state
                .agent_mode_label
                .as_deref()
                .is_some_and(|l| l.eq_ignore_ascii_case("GRIND"));
            let fire = !aborted
                && !will_retry
                && err.is_none()
                && reason == "threshold"
                && state.compaction_auto_armed
                && !grind_active;
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
            // Recovery already cleared the view and is replaying get_messages;
            // a late session_start must not wipe the restored transcript.
            let rehydrating = state.recovery.phase == heal::Phase::Rehydrating
                || state
                    .recovery
                    .rehydrate_grace_until
                    .is_some_and(|until| std::time::Instant::now() < until);
            if !rehydrating {
                state.entries.clear();
                state.md_cache_clear();
            }
        }
        _ => {}
    }
    false
}

async fn handle_ui_request(
    state: &mut AppState,
    req: &crate::rpc::protocol::ExtensionUiRequest,
    bridge: &mut RpcBridge,
) {
    // pi puts the dialog id at the TOP LEVEL of extension_ui_request
    // (rpc-mode.js: `output({type, id, ...request})`); the flattened `fields`
    // only carries title/message/options. Reading the id from fields was the
    // 2026-11-16 session-reset bug: every extension dialog was dropped, pi
    // waited forever, the tool never started and the watchdog respawned.
    let method = req.method.as_str();
    let fields = &req.fields;
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
            // Id comes from the top-level field first; the flattened legacy
            // location is kept only for old fixtures (2026-11-16).
            let id = if req.id.is_empty() {
                fields
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            } else {
                req.id.clone()
            };
            if id.is_empty() {
                // A request without an id can never be answered: silence would
                // block the actor forever (2026-11-15).
                stall::trail("ui.unknown_id", serde_json::json!({ "method": method }));
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
        other => {
            // Unknown extension-UI methods used to be dropped, leaving pi
            // awaiting an answer that can never arrive (2026-11-15). Fail fast
            // instead of hanging the actor, and leave a breadcrumb.
            let id = fields
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            stall::trail(
                "ui.unknown",
                serde_json::json!({ "method": other, "id": id }),
            );
            if !id.is_empty() {
                let _ = bridge.send_raw(&dialog_cancelled(&id).to_string()).await;
            }
        }
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

    #[tokio::test]
    async fn real_wire_confirm_with_top_level_id_is_answered_not_dropped() {
        // pi sends `{type, id, ...request}` — the TUI previously read the id
        // from the flattened fields, dropped every dialog, and the actor hung
        // at tool preflight (2026-11-16 session-reset bug).
        use crate::rpc::protocol::ExtensionUiRequest;
        let req: ExtensionUiRequest = serde_json::from_value(serde_json::json!({
            "type": "extension_ui_request",
            "id": "dlg-1",
            "method": "confirm",
            "title": "permissions",
            "message": "Allow bash?",
        }))
        .unwrap();
        let dir = std::env::temp_dir().join(format!("officina-dlg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stub.cjs"), "process.stdin.resume();").unwrap();
        let mut bridge = RpcBridge::spawn(&dir.join("stub.cjs"), &dir, &[])
            .await
            .unwrap();
        let mut s = AppState::default();
        handle_ui_request(&mut s, &req, &mut bridge).await;
        let Some(dlg) = s.dialogs.pop_front() else {
            panic!("confirm was dropped instead of queued");
        };
        assert_eq!(dlg.id, "dlg-1", "the dialog keeps the real top-level id");
        assert!(matches!(dlg.kind, state::DialogKind::Confirm));
        drop(bridge);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn id_less_request_stays_a_trail_noop_never_a_queued_dialog() {
        use crate::rpc::protocol::ExtensionUiRequest;
        let req: ExtensionUiRequest = serde_json::from_value(serde_json::json!({
            "type": "extension_ui_request",
            "id": "",
            "method": "confirm",
            "title": "t",
            "message": "m",
        }))
        .unwrap();
        // Real pi always sends an id; a genuinely id-less request is a wire
        // error and must be a no-op, never a queued dialog we cannot answer.
        let dir = std::env::temp_dir().join(format!("officina-dlg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stub.cjs"), "process.stdin.resume();").unwrap();
        let mut bridge = RpcBridge::spawn(&dir.join("stub.cjs"), &dir, &[])
            .await
            .unwrap();
        let mut s = AppState::default();
        handle_ui_request(&mut s, &req, &mut bridge).await;
        assert!(
            s.dialogs.is_empty(),
            "genuinely id-less request stays a no-op"
        );
        drop(bridge);
        let _ = std::fs::remove_dir_all(&dir);
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
    fn grind_mode_owns_the_post_compaction_continuation() {
        // 2026-11-14: the grind extension re-prompts on agent_end; the TUI
        // must not fire its own continuation under GRIND (double turns).
        let mut s = AppState::default();
        s.agent_mode_label = Some("GRIND".into());
        handle_event(
            &mut s,
            "compaction_start",
            &ev(serde_json::json!({ "reason": "threshold" })),
        );
        let fired = handle_event(
            &mut s,
            "compaction_end",
            &ev(serde_json::json!({
                "reason": "threshold", "aborted": false, "willRetry": false
            })),
        );
        assert!(!fired, "TUI continues nothing while GRIND is active");
        assert!(!s.compaction_auto_armed, "one-shot still consumed");
        // A non-GRIND label keeps the old behaviour (parity, I5).
        let mut b = AppState::default();
        b.agent_mode_label = Some("BUILD".into());
        handle_event(
            &mut b,
            "compaction_start",
            &ev(serde_json::json!({ "reason": "threshold" })),
        );
        assert!(handle_event(
            &mut b,
            "compaction_end",
            &ev(serde_json::json!({ "reason": "threshold", "willRetry": false })),
        ));
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
                state::ChatEntry::Diag(t)
                | state::ChatEntry::Warn(t)
                | state::ChatEntry::Error(t) => Some(t.clone()),
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
                state::ChatEntry::Diag(t)
                | state::ChatEntry::Warn(t)
                | state::ChatEntry::Error(t) => Some(t.clone()),
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

    fn replay_fixture(name: &str, text: &str) -> AppState {
        let mut s = AppState::default();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let value: serde_json::Value = serde_json::from_str(line).expect("fixture line json");
            let event = value["event"].as_str().expect("fixture event");
            handle_event(&mut s, event, &value["fields"]);
        }
        assert!(!s.is_streaming, "{name}: settles streaming");
        assert!(!s.has_running_tool(), "{name}: no running tools at settle");
        assert!(s.held.is_empty(), "{name}: held messages cleared");
        assert!(!s.is_compacting, "{name}: compaction cleared");
        assert!(
            s.compaction_started_at.is_none(),
            "{name}: compaction timer cleared"
        );
        s
    }

    #[test]
    fn recorded_fixtures_replay_to_a_clean_settle() {
        replay_fixture("full_turn", include_str!("fixtures/full_turn.jsonl"));
        replay_fixture("abort_turn", include_str!("fixtures/abort_turn.jsonl"));
        replay_fixture(
            "compaction_turn",
            include_str!("fixtures/compaction_turn.jsonl"),
        );
        let error = replay_fixture("error_turn", include_str!("fixtures/error_turn.jsonl"));
        let errors = error
            .entries
            .iter()
            .filter(|e| matches!(e, state::ChatEntry::Error(_)))
            .count();
        assert_eq!(errors, 1, "model error surfaced exactly once");
    }

    #[test]
    fn full_turn_lifecycle_is_conformant() {
        let mk = |v: serde_json::Value| v;
        let mut s = AppState::default();
        s.push_user("do the work".into());
        handle_event(&mut s, "agent_start", &mk(serde_json::json!({})));
        assert!(s.is_streaming, "agent_start streams");
        handle_event(&mut s, "message_start", &mk(serde_json::json!({})));
        handle_event(
            &mut s,
            "message_update",
            &mk(serde_json::json!({"delta": "working"})),
        );
        handle_event(
            &mut s,
            "tool_execution_start",
            &mk(serde_json::json!({
                "toolCallId": "t1", "toolName": "bash", "args": {"command": "ls"}
            })),
        );
        assert_eq!(
            s.entries
                .iter()
                .filter(|e| matches!(e, state::ChatEntry::Tool { running: true, .. }))
                .count(),
            1,
            "tool running"
        );
        handle_event(
            &mut s,
            "tool_execution_end",
            &mk(serde_json::json!({
                "toolCallId": "t1", "toolName": "bash", "isError": false,
                "result": {"content": [{"type": "text", "text": "ok"}]}
            })),
        );
        assert_eq!(
            s.entries
                .iter()
                .filter(|e| matches!(e, state::ChatEntry::Tool { running: true, .. }))
                .count(),
            0,
            "tool settled"
        );
        handle_event(
            &mut s,
            "message_end",
            &mk(serde_json::json!({
                "message": {"role": "assistant", "stopReason": "end_turn",
                            "content": [{"type": "text", "text": "done"}]}
            })),
        );
        handle_event(&mut s, "agent_end", &mk(serde_json::json!({})));
        assert!(!s.is_streaming, "agent_end settles the turn");
        handle_event(
            &mut s,
            "agent_settled",
            &mk(serde_json::json!({"aborted": false})),
        );
        assert!(s.held.is_empty(), "no held message survives a settle");
        assert_eq!(
            s.entries
                .iter()
                .filter(|e| matches!(e, state::ChatEntry::Tool { running: true, .. }))
                .count(),
            0
        );
    }

    #[test]
    fn aborted_settle_never_leaves_streaming_or_held() {
        let mk = |v: serde_json::Value| v;
        let mut s = AppState::default();
        s.held_push("steer me", state::HeldKind::Steer);
        handle_event(&mut s, "agent_start", &mk(serde_json::json!({})));
        assert!(s.is_streaming);
        handle_event(
            &mut s,
            "agent_settled",
            &mk(serde_json::json!({"aborted": true})),
        );
        assert!(!s.is_streaming, "abort settles streaming");
        assert!(s.held.is_empty(), "abort clears held messages");
    }

    #[test]
    fn model_error_is_surfaced_and_deduplicated() {
        let ev = |v: serde_json::Value| v;
        let mut s = AppState::default();
        let event = ev(serde_json::json!({
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": ""}],
                "stopReason": "error",
                "errorMessage": "Lapis Occultus: no physical model available — engine down"
            }
        }));
        handle_event(&mut s, "message_end", &event);
        let errors: Vec<String> = s
            .entries
            .iter()
            .filter_map(|e| match e {
                state::ChatEntry::Error(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("model error: Lapis Occultus"),
            "{errors:?}"
        );
        // Retries repeat the identical string — no duplicate entries.
        handle_event(&mut s, "message_end", &event);
        let count = s
            .entries
            .iter()
            .filter(|e| matches!(e, state::ChatEntry::Error(_)))
            .count();
        assert_eq!(count, 1, "identical consecutive errors collapse");
        // A healthy message adds no error.
        handle_event(
            &mut s,
            "message_end",
            &ev(serde_json::json!({
                "message": {"role": "assistant", "content": [{"type": "text", "text": "hi"}], "stopReason": "end_turn"}
            })),
        );
        assert_eq!(
            s.entries
                .iter()
                .filter(|e| matches!(e, state::ChatEntry::Error(_)))
                .count(),
            1
        );
    }

    #[test]
    fn compaction_timer_tracks_chat_progress() {
        let mut s = AppState::default();
        fire(&mut s, "agent_start");
        assert!(s.compaction_started_at.is_none());
        fire(&mut s, "compaction_start");
        assert!(s.is_compacting);
        assert!(
            s.compaction_started_at.is_some(),
            "chat progress timer armed"
        );
        fire(&mut s, "compaction_end");
        assert!(!s.is_compacting);
        assert!(
            s.compaction_started_at.is_none(),
            "chat progress timer cleared"
        );
        fire(&mut s, "agent_end");
    }

    #[test]
    fn held_messages_are_tracked_until_they_land() {
        let mut s = AppState::default();
        s.held_push("steer me", state::HeldKind::Steer);
        s.held_push("later please", state::HeldKind::FollowUp);
        s.held_push("behind the tool", state::HeldKind::Queued);
        let label = s.held_label().expect("held label");
        assert!(label.contains("steering: “steer me”"), "{label}");
        assert!(label.contains("(+2 more)"), "{label}");
        // A message_start means the steer landed.
        fire(&mut s, "message_start");
        assert!(!s.held.iter().any(|h| h.kind == state::HeldKind::Steer));
        // Delivery of the queued prompt resolves exactly one queue entry.
        s.held_resolve_queued();
        assert!(!s.held.iter().any(|h| h.kind == state::HeldKind::Queued));
        assert!(s.held.iter().any(|h| h.kind == state::HeldKind::FollowUp));
        // A new turn consumes the follow-up; turn end clears everything.
        fire(&mut s, "agent_start");
        assert!(s.held.is_empty());
        s.held_push("late", state::HeldKind::Steer);
        fire(&mut s, "agent_end");
        assert!(s.held.is_empty());
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

#[cfg(test)]
mod input_settings_tests {
    use super::*;

    async fn press(code: KeyCode, state: &mut AppState, bridge: &mut RpcBridge) {
        handle_key(KeyEvent::new(code, KeyModifiers::NONE), state, bridge).await;
    }

    #[tokio::test]
    async fn tab_cycles_agent_even_when_a_ghost_suggestion_is_visible() {
        let dir = std::env::temp_dir().join(format!("officina-tab-ghost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("stdin.log");
        let script = dir.join("stub.cjs");
        std::fs::write(
            &script,
            format!(
                "const fs=require('fs');process.stdin.on('data',d=>fs.appendFileSync({:?},d));process.stdin.resume();",
                log
            ),
        )
        .unwrap();
        let mut bridge = RpcBridge::spawn(&script, &dir, &[]).await.unwrap();
        let mut state = AppState::default();
        state.agent_mode = Some("plan".into());
        state.prompt_ghost = Some(state::GhostSuggestion {
            id: "g".into(),
            text: "suggested next step".into(),
            source: "test".into(),
        });
        press(KeyCode::Tab, &mut state, &mut bridge).await;
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let sent = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            sent.contains("\"type\":\"prompt\"") && sent.contains("/agent plan"),
            "Tab must send the agent cycle despite a ghost: {sent:?}"
        );
        assert!(state.prompt_ghost.is_some(), "ghost is not accepted by Tab");
        drop(bridge);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn right_tab_cursor_and_menu_keyboard_contract() {
        let dir = std::env::temp_dir().join(format!("officina-input-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("stub.cjs");
        std::fs::write(&script, "process.stdin.resume();").unwrap();
        let mut bridge = RpcBridge::spawn(&script, &dir, &[]).await.unwrap();
        let mut state = AppState::default();
        state.input = "/side".into();
        state.cursor = 5;
        press(KeyCode::Tab, &mut state, &mut bridge).await;
        assert_eq!(state.input, "/side");
        state.cursor = 2;
        press(KeyCode::Right, &mut state, &mut bridge).await;
        assert_eq!(state.cursor, 3);
        assert_eq!(state.input, "/side");
        state.cursor = 5;
        press(KeyCode::Right, &mut state, &mut bridge).await;
        assert_eq!(state.input, "/sidebar");
        press(KeyCode::Enter, &mut state, &mut bridge).await;
        assert!(state.setting_menu.is_some());
        assert_eq!(state.sidebar_force, None);
        press(KeyCode::Down, &mut state, &mut bridge).await;
        press(KeyCode::Enter, &mut state, &mut bridge).await;
        assert_eq!(state.sidebar_force, Some(true));
        assert!(state.setting_menu.is_none());
        state.ghost_events_file = Some(dir.join("ghost.jsonl"));
        state.prompt_ghost = Some(state::GhostSuggestion {
            id: "test".into(),
            text: "héllo".into(),
            source: "test".into(),
        });
        press(KeyCode::Tab, &mut state, &mut bridge).await;
        assert_eq!(state.input, "");
        assert!(state.prompt_ghost.is_some());
        press(KeyCode::Right, &mut state, &mut bridge).await;
        assert_eq!(state.input, "héllo");
        assert_eq!(state.cursor, 5);
        state.cursor = 1;
        press(KeyCode::Right, &mut state, &mut bridge).await;
        assert_eq!(state.cursor, 2);
        assert_eq!(state.input, "héllo");
        drop(bridge);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
