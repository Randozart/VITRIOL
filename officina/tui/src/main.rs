mod envfile;
mod fire;
mod markdown;
mod rpc;
mod session_guard;
mod theme;
mod tui;
mod watermark;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use rpc::bridge::RpcBridge;

#[derive(Parser)]
#[command(
    name = "officina",
    about = "Ratatui TUI for Officina — pi-coding-agent frontend"
)]
struct Cli {
    /// Working directory (where pi-coding-agent will run). Defaults to the
    /// directory officina was LAUNCHED from — the old compile-time default
    /// silently pointed every bare launch at the VITRIOL officina dir
    /// (owner bug report 2026-09-02: launched in Projects/ontic, session
    /// landed in VITRIOL).
    #[arg(short, long)]
    cwd: Option<PathBuf>,

    /// Path to pi CLI entry point
    #[arg(
        short,
        long,
        default_value = "/home/randozart/Desktop/Projects/VITRIOL/officina/node_modules/.bin/pi"
    )]
    pi: PathBuf,

    /// Open this pi session file (or id) at startup — used by /import and
    /// `vitriol opencode`.
    #[arg(long)]
    session: Option<String>,

    /// Fork this pi session file (or id) at startup.
    #[arg(long)]
    fork: Option<String>,

    /// Start a fresh session instead of resuming the latest one.
    #[arg(long, conflicts_with_all = ["session", "fork"])]
    new: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Officina's launch env (~/.vitriol/officina/env) must apply to the pi
    // child and the TUI itself; shell exports still win (2026-11-10).
    envfile::load_officina_env();

    // Per-turn git snapshots are a harness default (operator decision
    // 2026-11-10): rollback is not optional safety. The extension still
    // no-ops outside a git repo; TRIS_SNAPSHOT=0 is the kill switch.
    if std::env::var_os("TRIS_SNAPSHOT").is_none() {
        std::env::set_var("TRIS_SNAPSHOT", "1");
    }

    // Resolve the working directory: explicit flag > launch directory.
    let cwd = cli.cwd.unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
    });

    // Resolve the pi CLI path relative to cwd
    let pi_path = if cli.pi.is_absolute() {
        cli.pi.clone()
    } else {
        cwd.join(&cli.pi)
    };

    if !pi_path.exists() {
        anyhow::bail!(
            "pi CLI not found at {:?} — are you running from the officina directory?",
            pi_path
        );
    }

    let mut extra = Vec::new();
    // Cross-process guard FIRST (kernel-held flock on the session store):
    // prepare_session_args consults it to refuse resuming or mutating a
    // store another live TUI owns (corruption guard I2/I3, 2026-11-11).
    let lock = session_guard::acquire_session_lock(&cwd);
    let (session_args, session_notice) = session_guard::prepare_session_args(
        &cwd,
        cli.session.as_deref(),
        cli.new,
        cli.fork.as_deref(),
        &lock,
    );
    extra.extend(session_args);
    if let Some(note) = &session_notice {
        eprintln!("[officina] {note}");
    }
    if let Some(fork) = cli.fork {
        extra.push("--fork".to_string());
        extra.push(fork);
    }
    if cli.new {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        extra.push("--session-id".to_string());
        extra.push(format!("officina-{stamp}"));
    }

    // The lock must outlive the bridge: `lock` stays bound until the end
    // of main, so pi's session store stays fenced for the whole run.

    // Spawn pi --mode rpc
    let mut bridge = RpcBridge::spawn(&pi_path, &cwd, &extra)
        .await
        .context("failed to spawn pi-coding-agent RPC subprocess")?;

    // Run the TUI event loop
    let result = tui::run(&mut bridge, cwd).await;

    // Clean up
    let _ = bridge.kill().await;

    result
}
