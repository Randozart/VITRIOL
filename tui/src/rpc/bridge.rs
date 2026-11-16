// RPC bridge — spawns pi --mode rpc as a subprocess, communicates via JSONL
// stdin/stdout. Handles request/response correlation and event dispatch.
//
// Architecture: background tasks read stdout and stderr; stdout JSONL is
// routed (responses / events / extension UI requests) to the TUI event loop,
// stderr drains into a DiagLog ring buffer (must drain — an unread pipe
// fills at ~64KiB and blocks the subprocess).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
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

/// Extension directories carried into foreign projects (see spawn doc).
/// Root is derived from the pi binary location —
/// `<officina>/node_modules/.bin/pi` → `<officina>/.pi/extensions` — so a
/// relocated checkout keeps working. Every officina extension with an index.ts
/// is carried unless the project shadows it by name; `_shared` is a support
/// module, not an extension.
fn officina_extensions_to_carry(cli_path: &Path, cwd: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();

    // `.bin/pi` → node_modules → <officina>
    let officina = match cli_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        Some(p) => p.to_path_buf(),
        None => return out,
    };
    let exts = officina.join(".pi").join("extensions");
    if !exts.is_dir() {
        return out;
    }

    // Full harness surface (operator decision 2026-11-10): load every
    // extension directory with an index.ts. The old hardcoded 7-item subset
    // left permissions-guard, snapshot, rewind, subagent, memory, repo-map,
    // plan-mode and the quality gates out of the canonical frontend.
    //
    // Project-owned extensions win by name so a repo can shadow a portable
    // extension without double-registering tools/commands.
    let mut shadowed: HashSet<String> = HashSet::new();
    for root in [
        cwd.join(".pi").join("extensions"),
        cwd.join(".officina").join("extensions"),
    ] {
        if let Ok(rd) = std::fs::read_dir(&root) {
            for ent in rd.flatten() {
                if ent.path().is_dir() {
                    shadowed.insert(ent.file_name().to_string_lossy().to_string());
                }
            }
        }
    }

    let Ok(rd) = std::fs::read_dir(&exts) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("index.ts").is_file())
        .filter(|p| {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string());
            name.as_deref() != Some("_shared") && name.is_none_or(|n| !shadowed.contains(&n))
        })
        .collect();
    dirs.sort();
    out.extend(dirs);
    out
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

/// Everything needed to spawn the same pi invocation again after a
/// recovery (2026-11-15 self-heal): the CLI, the project cwd and the
/// session/extension arguments chosen at startup.
#[derive(Debug, Clone)]
pub struct PiLaunch {
    pub cli_path: PathBuf,
    pub cwd: PathBuf,
    pub extra_args: Vec<String>,
}

/// Arguments for a respawn: drop the startup-only session selectors and
/// pin the recovery to the file pi was already writing.
pub fn restart_args(extra: &[String], session: Option<&Path>) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in extra {
        if skip_next {
            skip_next = false;
            continue;
        }
        match arg.as_str() {
            "--session" | "--fork" | "--session-id" => {
                skip_next = true;
                continue;
            }
            _ if arg.starts_with("--session=")
                || arg.starts_with("--fork=")
                || arg.starts_with("--session-id=") =>
            {
                continue;
            }
            _ => out.push(arg.clone()),
        }
    }
    if let Some(path) = session {
        out.push("--session".to_string());
        out.push(path.to_string_lossy().to_string());
    }
    out
}

/// Bi-directional RPC bridge to a pi --mode rpc subprocess.
pub struct RpcBridge {
    child: Child,
    stdin_tx: mpsc::UnboundedSender<String>,
    next_id: AtomicU32,
    pi_path: PathBuf,
    launch: PiLaunch,
    pub diag: DiagLog,
}

impl RpcBridge {
    /// pi's OS pid — the root of the subprocess tree a running bash tool
    /// lives in (liveness probing reads /proc under it).
    pub fn pi_pid(&self) -> Option<u32> {
        self.child.id()
    }

    /// Spawn `node <cli_path> --mode rpc -a` and return a connected bridge.
    ///
    /// Project trust is no longer blanket-approved: the dialog subsystem in
    /// the TUI answers pi's trust `select`. OFFICINA_TRUST_PROJECT=1 restores
    /// the old `-a` behaviour for headless automation.
    ///
    /// Officina extensions travel with the TUI (owner request 2026-09-02:
    /// "shouldn't officina be reachable from any project folder, so long as
    /// VITRIOL is running?"). A bare project has no `.pi/extensions` of its
    /// own, so nothing would register the `llamacpp` provider (owner bug
    /// report 2026-09-02: prompt failed abroad with "No API key found" —
    /// pi fell back to stock google) and the mode chip / composer fire /
    /// sidebar panel would never arrive. Those are loaded explicitly via
    /// `-e` whenever the session dir doesn't do its own extension
    /// discovery. llama-cpp-provider resolves its models.json via
    /// import.meta.url, so it is cwd-independent. session-panel rides the
    /// RPC setWidget fallback (its setSidebar path doesn't exist on pi's
    /// RPC surface), giving foreign projects the full sidebar: coupling,
    /// ctx, eng, scratchpad notes, tasks, files, hints. Deliberately NOT
    /// loaded abroad: knowledge-inject / task-state / skill-inject /
    /// phase-model et al. — they inject or display VITRIOL-workflow
    /// context and would poison a foreign project (session-panel only
    /// imports their pure data getters, which stay empty without them).
    pub async fn spawn(cli_path: &Path, cwd: &Path, extra_args: &[String]) -> Result<Self> {
        let launch = PiLaunch {
            cli_path: cli_path.to_path_buf(),
            cwd: cwd.to_path_buf(),
            extra_args: extra_args.to_vec(),
        };
        Self::spawn_launch(launch, None).await
    }

    async fn spawn_launch(launch: PiLaunch, diag: Option<DiagLog>) -> Result<Self> {
        let mut command = Command::new("node");
        command
            .arg(&launch.cli_path)
            .arg("--mode")
            .arg("rpc")
            // Structured navigation tools (2026-11-10): read/bash/edit/write
            // alone makes the model burn tokens on find/grep/ls via bash.
            .arg("--tools")
            .arg("+grep,+find,+ls");
        // Trust: the Rust TUI now implements pi's project-trust `select`
        // dialog, so do not blanket-approve. Opt back into the old behaviour
        // with OFFICINA_TRUST_PROJECT=1 (automation headless runs).
        if std::env::var("OFFICINA_TRUST_PROJECT").as_deref() == Ok("1") {
            command.arg("-a");
        }
        for arg in &launch.extra_args {
            command.arg(arg);
        }
        for ext in officina_extensions_to_carry(&launch.cli_path, &launch.cwd) {
            command.arg("-e").arg(ext);
        }
        // OpenCode Go models are built into pi but gated on its API key.
        // Reuse the operator's existing OpenCode auth store at spawn time —
        // the secret is never copied into Officina config or logs. This is
        // the model catalog bridge, not a settings import.
        if std::env::var_os("OPENCODE_API_KEY").is_none() {
            if let Some(key) = opencode_go_key() {
                command.env("OPENCODE_API_KEY", key);
            }
        }
        // Own process group (2026-11-15 self-heal): a stuck tool subtree is
        // terminated as one unit instead of leaving orphans behind.
        command.process_group(0);
        let mut child = command
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("failed to spawn pi --mode rpc")?;

        let stdin = child.stdin.take().context("no stdin")?;
        // Stdout left on child — taken by start_reader().
        let stderr = child.stderr.take().context("no stderr")?;

        let diag = diag.unwrap_or_else(|| DiagLog::new(500));
        // Crash black-box (2026-11-13): the panic hook reads this ring via
        // the process-global (crashlog.rs) — registered here because the
        // diag log lives exactly once per bridge.
        crate::tui::crashlog::set_diag(diag.clone());

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
            pi_path: launch.cli_path.clone(),
            launch,
            diag,
        })
    }

    /// `<officina>/scripts/<name>` derived from the pi CLI path
    /// (`<officina>/node_modules/.bin/pi`). None when the franken-layout
    /// doesn't match.
    pub fn officina_script(&self, name: &str) -> Option<PathBuf> {
        let officina = self
            .pi_path
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())?;
        let script = officina.join("scripts").join(name);
        script.is_file().then_some(script)
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
        if let Some(slot) = cmd.id_mut() {
            *slot = Some(id.clone());
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

    /// Kill the subprocess (direct child only; recovery uses terminate_tree).
    pub async fn kill(&mut self) -> Result<()> {
        self.child
            .kill()
            .await
            .context("failed to kill subprocess")?;
        Ok(())
    }

    /// Terminate pi AND every tool it spawned. The child runs in its own
    /// process group (see spawn_launch), so one signal reaches the whole
    /// subtree. SIGTERM first for a clean session flush; SIGKILL after 2 s.
    pub async fn terminate_tree(&mut self) -> Result<()> {
        let Some(pid) = self.child.id() else {
            let _ = self.child.wait().await;
            return Ok(());
        };
        signal_group(pid, "TERM");
        if tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait())
            .await
            .is_err()
        {
            signal_group(pid, "KILL");
            let _ = self.child.kill().await;
            let _ =
                tokio::time::timeout(std::time::Duration::from_secs(2), self.child.wait()).await;
        }
        Ok(())
    }

    /// Kill the old tree, append the session pin and spawn a fresh child.
    /// Returns the new stdout reader channel; the caller must install it in
    /// its select loop (the old one is dead with the old child).
    pub async fn respawn(
        &mut self,
        session: Option<&Path>,
    ) -> Result<mpsc::UnboundedReceiver<WireMessage>> {
        self.terminate_tree().await?;
        let mut launch = self.launch.clone();
        launch.extra_args = restart_args(&launch.extra_args, session);
        let mut fresh = Self::spawn_launch(launch.clone(), Some(self.diag.clone())).await?;
        std::mem::swap(&mut self.child, &mut fresh.child);
        self.stdin_tx = fresh.stdin_tx;
        self.next_id = fresh.next_id;
        self.pi_path = fresh.pi_path;
        self.launch = fresh.launch;
        // `fresh` now owns the reaped old child; kill_on_drop is a no-op.
        Ok(self.start_reader())
    }
}

/// Read the OpenCode Go API key from OpenCode's own auth store. Env first;
/// XDG_DATA_HOME then ~/.local/share. Never logged or persisted.
fn opencode_go_key() -> Option<String> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    let text = std::fs::read_to_string(data.join("opencode").join("auth.json")).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&text).ok()?;
    let key = doc
        .get("opencode-go")
        .and_then(|v| v.get("key"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|k| !k.is_empty())?;
    Some(key.to_string())
}

/// Signal a whole process group via the system `kill` (negative pgid).
/// Best-effort: a vanished group is the normal success case.
fn signal_group(pid: u32, signal: &str) -> bool {
    std::process::Command::new("kill")
        .args(["-s", signal, "--", &format!("-{pid}")])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn carries_every_officina_extension_into_bare_projects() {
        // Fake officina checkout: <root>/node_modules/.bin/pi + .pi/extensions.
        let root = std::env::temp_dir().join(format!("officina-carry-{}", std::process::id()));
        let bin = root.join("node_modules").join(".bin");
        let exts = root.join(".pi").join("extensions");
        for name in [
            "llama-cpp-provider",
            "agent-mode",
            "session-panel",
            "_shared",
        ] {
            let dir = exts.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("index.ts"), "// ext\n").unwrap();
        }
        std::fs::create_dir_all(&bin).unwrap();
        let pi = bin.join("pi");
        std::fs::write(&pi, "#!/bin/sh\n").unwrap();

        // Bare project → all real extensions carried, support dir skipped.
        let bare = std::env::temp_dir().join(format!("officina-bare-{}", std::process::id()));
        std::fs::create_dir_all(&bare).unwrap();
        let carried = officina_extensions_to_carry(&pi, &bare);
        assert_eq!(carried.len(), 3);
        assert!(!carried.iter().any(|p| p.ends_with("_shared")));

        // Project shadowing one name → the shadowed extension is not carried.
        let own = std::env::temp_dir().join(format!("officina-own-{}", std::process::id()));
        let own_ext = own.join(".pi").join("extensions").join("agent-mode");
        std::fs::create_dir_all(&own_ext).unwrap();
        std::fs::write(own_ext.join("index.ts"), "// project ext\n").unwrap();
        let carried = officina_extensions_to_carry(&pi, &own);
        assert_eq!(carried.len(), 2);
        assert!(!carried.iter().any(|p| p.ends_with("agent-mode")));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bare);
        let _ = std::fs::remove_dir_all(&own);
    }

    #[tokio::test]
    async fn respawn_restarts_the_child_and_hands_back_a_live_reader() {
        let dir = std::env::temp_dir().join(format!("officina-respawn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-pi.js");
        std::fs::write(
            &script,
            "process.stdin.resume();\n\
             process.stdout.write(JSON.stringify({type:'respawn_probe'})+'\\n');\n\
             setInterval(()=>{}, 1000);\n",
        )
        .unwrap();

        let mut bridge = RpcBridge::spawn(&script, &dir, &[]).await.unwrap();
        let mut rx = bridge.start_reader();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("first reader message timed out")
            .expect("reader closed");
        match first {
            WireMessage::Event(e) => assert_eq!(e.event_type, "respawn_probe"),
            other => panic!("unexpected first message: {other:?}"),
        }

        let mut fresh = bridge.respawn(None).await.unwrap();
        let second = tokio::time::timeout(std::time::Duration::from_secs(5), fresh.recv())
            .await
            .expect("respawned reader message timed out")
            .expect("respawned reader closed");
        match second {
            WireMessage::Event(e) => assert_eq!(e.event_type, "respawn_probe"),
            other => panic!("unexpected respawn message: {other:?}"),
        }
        bridge.terminate_tree().await.unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn terminate_tree_kills_the_tool_subtree_not_just_pi() {
        let dir = std::env::temp_dir().join(format!("officina-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-pi-tree.js");
        std::fs::write(
            &script,
            "const {spawn}=require('child_process');\n\
             const c=spawn('sleep',['30'],{stdio:'ignore'});\n\
             process.stdin.resume();\n\
             process.stdout.write(JSON.stringify({type:'grandchild', pid:c.pid})+'\\n');\n\
             setInterval(()=>{},1000);\n",
        )
        .unwrap();
        let mut bridge = RpcBridge::spawn(&script, &dir, &[]).await.unwrap();
        let mut rx = bridge.start_reader();
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("grandchild message timed out")
            .expect("reader closed");
        let pid = match msg {
            WireMessage::Event(e) => e
                .fields
                .get("pid")
                .and_then(|v| v.as_u64())
                .expect("pid field") as u32,
            other => panic!("unexpected message: {other:?}"),
        };
        bridge.terminate_tree().await.unwrap();
        let mut gone = false;
        for _ in 0..20 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                gone = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(gone, "tool subtree pid {pid} survived terminate_tree");
    }

    #[test]
    fn restart_args_pin_the_live_session_and_drop_startup_selectors() {
        let extra = vec![
            "--session".to_string(),
            "/old/s.jsonl".to_string(),
            "--session-id".to_string(),
            "officina-1".to_string(),
            "--fork=/old/fork.jsonl".to_string(),
            "--keep".to_string(),
            "--session=/old/other.jsonl".to_string(),
        ];
        let out = restart_args(&extra, Some(Path::new("/live/s.jsonl")));
        assert_eq!(
            out,
            vec![
                "--keep".to_string(),
                "--session".to_string(),
                "/live/s.jsonl".to_string()
            ]
        );
        // No live file: startup selectors are still gone, never duplicated.
        let out = restart_args(&extra, None);
        assert_eq!(out, vec!["--keep".to_string()]);
    }

    #[test]
    fn carries_nothing_without_officina_tree() {
        let bare = std::env::temp_dir().join(format!("officina-noroot-{}", std::process::id()));
        std::fs::create_dir_all(&bare).unwrap();
        let fake_pi = bare.join("pi");
        std::fs::write(&fake_pi, "#!/bin/sh\n").unwrap();
        assert!(officina_extensions_to_carry(&fake_pi, &bare).is_empty());
        let _ = std::fs::remove_dir_all(&bare);
    }
}
