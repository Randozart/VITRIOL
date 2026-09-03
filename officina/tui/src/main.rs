mod fire;
mod markdown;
mod rpc;
mod theme;
mod tui;
mod watermark;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use rpc::bridge::RpcBridge;

#[derive(Parser)]
#[command(name = "officina", about = "Ratatui TUI for Officina — pi-coding-agent frontend")]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

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

    // Spawn pi --mode rpc
    let mut bridge = RpcBridge::spawn(&pi_path, &cwd)
        .await
        .context("failed to spawn pi-coding-agent RPC subprocess")?;

    // Run the TUI event loop
    let result = tui::run(&mut bridge, cwd).await;

    // Clean up
    let _ = bridge.kill().await;

    result
}
