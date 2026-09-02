// RPC bridge — spawns pi --mode rpc as a subprocess, communicates via JSONL
// stdin/stdout. Handles request/response correlation and event dispatch.
//
// Architecture: background tasks read stdout and stderr; stdout JSONL is
// routed (responses / events / extension UI requests) to the TUI event loop,
// stderr drains into a DiagLog ring buffer (must drain — an unread pipe
// fills at ~64KiB and blocks the subprocess).

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::protocol::{ExtensionUiRequest, RpcCommand, RpcEvent, RpcResponse};

/// Parsed wire message — either a response, an event, or an extension UI request.
#[derive(Debug)]
pub enum WireMessage {
    Response(RpcResponse),
    Event(RpcEvent),
    ExtensionUiRequest(ExtensionUiRequest),
}

/// Shared stderr ring buffer — last N lines from the subprocess.
#[derive(Clone)]
pub struct DiagLog {
    lines: Arc<Mutex<Vec<String>>>,
    cap: usize,
}

impl DiagLog {
    pub fn new(cap: usize) -> Self {
        Self {
            lines: Arc::new(Mutex::new(Vec::new())),
            cap,
        }
    }

    pub fn push(&self, line: String) {
        let mut v = self.lines.lock().unwrap();
        if v.len() >= self.cap {
            v.remove(0);
        }
        v.push(line);
    }

    pub fn snapshot(&self) -> Vec<String> {
        self.lines.lock().unwrap().clone()
    }
}

/// Bi-directional RPC bridge to a pi --mode rpc subprocess.
pub struct RpcBridge {
    child: Child,
    stdin_tx: mpsc::UnboundedSender<String>,
    next_id: AtomicU32,
    pub diag: DiagLog,
}

impl RpcBridge {
    /// Spawn `node <cli_path> --mode rpc -a` and return a connected bridge.
    ///
    /// `-a` (--approve) overrides project trust: RPC mode has no UI for the
    /// trust dialog, and untrusted means pi silently skips project extensions
    /// AND project settings (verified 2026-09-02: without -a → 0 setWidgets,
    /// model "unknown"; with -a → 23 setWidgets, 15 extension commands).
    pub async fn spawn(cli_path: &Path, cwd: &Path) -> Result<Self> {
        let mut child = Command::new("node")
            .arg(cli_path)
            .arg("--mode")
            .arg("rpc")
            .arg("-a")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn pi --mode rpc")?;

        let stdin = child.stdin.take().context("no stdin")?;
        // Stdout left on child — taken by start_reader().
        let stderr = child.stderr.take().context("no stderr")?;

        let diag = DiagLog::new(500);

        // Stdin writer task
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = stdin_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // Stderr drain task
        let diag_sink = diag.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                diag_sink.push(line);
            }
        });

        Ok(Self {
            child,
            stdin_tx,
            next_id: AtomicU32::new(1),
            diag,
        })
    }

    /// Start the stdout reader task. Returns a channel that yields parsed wire messages.
    pub fn start_reader(&mut self) -> mpsc::UnboundedReceiver<WireMessage> {
        let stdout = self.child.stdout.take().expect("stdout already taken");
        let (tx, rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }

                let parsed: Result<Value, _> = serde_json::from_str(&line);
                let value = match parsed {
                    Ok(v) => v,
                    Err(_) => continue, // skip unparseable lines
                };

                let msg_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

                // Route by message type
                if msg_type == "response" {
                    if let Ok(resp) = serde_json::from_value::<RpcResponse>(value) {
                        let _ = tx.send(WireMessage::Response(resp));
                    }
                } else if msg_type == "extension_ui_request" {
                    if let Ok(req) = serde_json::from_value::<ExtensionUiRequest>(value) {
                        let _ = tx.send(WireMessage::ExtensionUiRequest(req));
                    }
                } else {
                    // All other types are events
                    let event = RpcEvent {
                        event_type: msg_type.to_string(),
                        fields: value,
                    };
                    let _ = tx.send(WireMessage::Event(event));
                }
            }
        });

        rx
    }

    /// Send a command with an auto-generated correlation ID.
    pub async fn request(&self, mut cmd: RpcCommand) -> Result<RpcResponse> {
        let id = format!("req_{}", self.next_id.fetch_add(1, Ordering::Relaxed));

        // Inject the correlation ID into the command
        match &mut cmd {
            RpcCommand::Prompt { id: ref mut i, .. }
            | RpcCommand::Steer { id: ref mut i, .. }
            | RpcCommand::FollowUp { id: ref mut i, .. }
            | RpcCommand::Abort { id: ref mut i }
            | RpcCommand::NewSession { id: ref mut i, .. }
            | RpcCommand::SwitchSession { id: ref mut i, .. }
            | RpcCommand::GetState { id: ref mut i }
            | RpcCommand::GetMessages { id: ref mut i }
            | RpcCommand::GetSessionStats { id: ref mut i }
            | RpcCommand::SetModel { id: ref mut i, .. }
            | RpcCommand::CycleModel { id: ref mut i }
            | RpcCommand::GetAvailableModels { id: ref mut i }
            | RpcCommand::GetCommands { id: ref mut i }
            | RpcCommand::SetThinkingLevel { id: ref mut i, .. }
            | RpcCommand::CycleThinkingLevel { id: ref mut i }
            | RpcCommand::Compact { id: ref mut i, .. }
            | RpcCommand::Bash { id: ref mut i, .. } => {
                *i = Some(id.clone());
            }
            RpcCommand::ExtensionUiResponse { .. } => {} // ID already set
        }

        let json = serde_json::to_string(&cmd)?;
        let line = format!("{}\n", json);
        self.stdin_tx
            .send(line)
            .context("failed to send command — subprocess may have exited")?;

        Ok(RpcResponse {
            id: Some(id),
            msg_type: "response".to_string(),
            command: None,
            success: None,
            data: None,
            error: None,
        })
    }

    /// Send a raw JSON line (for extension_ui_response).
    pub async fn send_raw(&self, json: &str) -> Result<()> {
        let line = format!("{}\n", json);
        self.stdin_tx
            .send(line)
            .context("failed to send raw line")?;
        Ok(())
    }

    /// Kill the subprocess.
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await.context("failed to kill subprocess")?;
        Ok(())
    }
}
