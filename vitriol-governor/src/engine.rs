//! Side effects: engine HTTP, systemctl, scripts/vitriol, oom_score_adj.

use std::collections::HashSet;
use std::io::Write;
use std::time::Duration;

fn sh(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("spawn {cmd}: {e}"))?;
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if out.status.success() {
        Ok(combined)
    } else {
        Err(format!("{cmd} {:?} failed: {}", args, combined.trim()))
    }
}

pub fn log(msg: &str) {
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S");
    eprintln!("governor: [{now}] {msg}");
}

pub fn engine_pids(unit: &str) -> (HashSet<u32>, bool) {
    let mut pids = pids_from_unit(unit);
    pids.extend(crate::observe::pids_by_comm("llama-server"));
    let up = !pids.is_empty();
    (pids, up)
}

fn pids_from_unit(unit: &str) -> HashSet<u32> {
    let mut out = HashSet::new();
    if let Ok(v) = sh("systemctl", &["show", unit, "-p", "MainPID", "--value"]) {
        if let Ok(pid) = v.trim().parse::<u32>() {
            if pid > 0 {
                out.insert(pid);
            }
        }
    }
    out
}

pub fn health_ok(port: u16) -> bool {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build()
        .get(&format!("http://127.0.0.1:{port}/health"))
        .call()
        .map(|r| r.status() == 200)
        .unwrap_or(false)
}

pub fn props(port: u16) -> Option<serde_json::Value> {
    let body = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build()
        .get(&format!("http://127.0.0.1:{port}/props"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    serde_json::from_str(&body).ok()
}

pub fn wait_health(port: u16, timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if health_ok(port) {
            return true;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    false
}

/// `scripts/vitriol config load <profile>` (journals + fingerprints in the
/// launcher; backs up the previous config).
pub fn config_load(project_dir: &str, profile: &str) -> Result<(), String> {
    let script = format!("{project_dir}/scripts/vitriol");
    let out = sh(&script, &["config", "load", profile])?;
    log(&format!("config load {profile}: {}", last_line(&out)));
    Ok(())
}

fn last_line(s: &str) -> String {
    s.lines().last().unwrap_or("").to_string()
}

pub fn unit_restart(unit: &str) -> Result<(), String> {
    sh("systemctl", &["restart", unit]).map(|_| ())
}

/// Engine stops being the last OOM victim while yielded.
pub fn set_oom_score_adj(pid: u32, val: i32) -> Result<(), String> {
    let mut f = std::fs::File::options()
        .write(true)
        .open(format!("/proc/{pid}/oom_score_adj"))
        .map_err(|e| format!("oom_score_adj open pid {pid}: {e}"))?;
    f.write_all(val.to_string().as_bytes())
        .map_err(|e| format!("oom_score_adj write pid {pid}: {e}"))?;
    Ok(())
}

pub fn apply_yield_oom(cfg: &crate::config::Config) {
    let (pids, _) = engine_pids(&cfg.engine_unit);
    for pid in pids {
        if let Err(e) = set_oom_score_adj(pid, 0) {
            log(&format!("WARN {e}"));
        } else {
            log(&format!("oom_score_adj=0 pid {pid} (engine yields OOM priority)"));
        }
    }
}
