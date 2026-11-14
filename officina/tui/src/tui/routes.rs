//! /routes page data — the Caduceus decision trail.
//!
//! The route ledger (_shared/route-ledger.ts) is the FACTS file: one JSON
//! object per request, day-stamped, carrying the decision (tier, provider,
//! model, reason, local state, latency, outcome). The TUI only READS it.
//
// PROVENANCE: original work, this repo.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RouteEntry {
    pub ts: String,
    /// Why the router chose this lane (user/retry/continuation/overflow).
    pub reason: String,
    /// "local" | "cloud"
    pub tier: String,
    pub provider: String,
    pub model: String,
    pub profile: String,
    /// Engine state at decision time: up/down/busy/unknown.
    #[serde(rename = "localState")]
    pub local_state: String,
    #[serde(rename = "latencyMs")]
    pub latency_ms: u64,
    #[serde(rename = "tokensIn")]
    pub tokens_in: u64,
    #[serde(rename = "tokensOut")]
    pub tokens_out: u64,
    #[serde(rename = "costEstimate")]
    pub cost_estimate: f64,
    pub outcome: String,
    /// Lanes skipped by admission, with reasons (2026-11-14).
    #[serde(rename = "candidatesSkipped")]
    pub candidates_skipped: Vec<String>,
}

/// Background probe fact (cloud-probe, 2026-11-14).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProbeFact {
    pub kind: String,
    #[serde(rename = "okAt")]
    pub ok_at: Option<u64>,
    #[serde(rename = "latencyMs")]
    pub latency_ms: Option<u64>,
    pub until: Option<u64>,
    #[serde(rename = "failCount")]
    pub fail_count: Option<u64>,
    #[serde(rename = "nextProbeAt")]
    pub next_probe_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProbeLease {
    pub pid: u64,
    pub until: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct PauseFact {
    pub until: u64,
    #[serde(rename = "safeBytes")]
    pub safe_bytes: Option<u64>,
    pub reason: String,
}

/// Gateway admission facts (gateway-health.json): probes, lease, pauses.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GatewayHealthDoc {
    pub ts: u64,
    pub probes: std::collections::HashMap<String, ProbeFact>,
    #[serde(rename = "probeLease")]
    pub probe_lease: Option<ProbeLease>,
    pub pauses: std::collections::HashMap<String, PauseFact>,
    pub invalid: std::collections::HashMap<String, serde_json::Value>,
}

pub fn gateway_health_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("gateway-health.json");
    p
}

pub fn load_gateway_health() -> GatewayHealthDoc {
    load_gateway_health_at(&gateway_health_path())
}

pub fn load_gateway_health_at(path: &Path) -> GatewayHealthDoc {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Gateway liveness from the caduceus facts file (None = unknown).
pub fn caduceus_gateway_up() -> Option<bool> {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("caduceus-state.json");
    let raw = fs::read_to_string(p).ok()?;
    let doc: serde_json::Value = serde_json::from_str(&raw).ok()?;
    doc.get("gateway_up").and_then(|v| v.as_bool())
}

/// One-word probe status for a lane (None = never probed).
pub fn probe_label(fact: Option<&ProbeFact>, now_ms: u64) -> String {
    let Some(fact) = fact else {
        return "unverified".to_string();
    };
    match fact.kind.as_str() {
        "ok" => match fact.latency_ms {
            Some(ms) => format!("✓ {ms}ms"),
            None => "✓ ok".to_string(),
        },
        "cooling" => {
            let until = fact.until.unwrap_or(0);
            if until > now_ms {
                format!("cooling {}m", (until - now_ms).div_ceil(60_000))
            } else {
                "cooling · expired".to_string()
            }
        }
        "paused" => {
            let until = fact.until.unwrap_or(0);
            if until > now_ms {
                format!("paused {}m", (until - now_ms).div_ceil(60_000))
            } else {
                "paused · expired".to_string()
            }
        }
        "invalid" => "invalid".to_string(),
        "no_fit" => "no fit".to_string(),
        other => other.to_string(),
    }
}

/// The last decisions, newest first (ledger order is chronological).
#[derive(Debug, Clone, Default)]
pub struct RouteReport {
    pub day: String,
    pub entries: Vec<RouteEntry>,
    /// Current probe facts, keyed by lane id (owner/model).
    pub probes: std::collections::HashMap<String, ProbeFact>,
}

impl RouteReport {
    pub fn latest(&self) -> Option<&RouteEntry> {
        self.entries.first()
    }
}

/// Local day stamp "YYYYMMDD" (UTC-independent enough for a day file —
/// the ledger writer stamps with the same local day).
pub fn today() -> String {
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
    let months = [
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
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!("{y:04}{:02}{:02}", m + 1, d + 1)
}

pub fn ledger_path(day: &str) -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push(format!("route-ledger-{day}.jsonl"));
    p
}

pub fn load_recent(day: &str, limit: usize) -> RouteReport {
    load_recent_at(&ledger_path(day), day, limit)
}

pub fn load_recent_at(path: &Path, day: &str, limit: usize) -> RouteReport {
    let mut entries: Vec<RouteEntry> = fs::read_to_string(path)
        .ok()
        .map(|text| {
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<RouteEntry>(l).ok())
                .collect()
        })
        .unwrap_or_default();
    entries.reverse();
    entries.truncate(limit);
    RouteReport {
        day: day.to_string(),
        entries,
        probes: load_gateway_health().probes,
    }
}

/// Short local time from the ledger's ISO stamp ("2026-11-14T09:28:51.223Z").
pub fn short_time(ts: &str) -> String {
    ts.split('T')
        .nth(1)
        .map(|t| t.chars().take(8).collect::<String>())
        .unwrap_or_else(|| ts.chars().take(8).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_ledger_line_and_orders_newest_first() {
        let dir = std::env::temp_dir().join(format!("routes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger.jsonl");
        let l1 = r#"{"ts":"2026-11-14T09:21:51.807Z","reason":"user","tier":"cloud","provider":"caduceus","model":"opencode-go/deepseek-v4.1-flash","profile":"auto","localState":"down","latencyMs":30088,"tokensIn":4604,"tokensOut":1869,"costEstimate":0,"outcome":"ok"}"#;
        let l2 = r#"{"ts":"2026-11-14T09:24:35.060Z","reason":"retry","tier":"local","provider":"llamacpp","model":"Lapis Occultus","profile":"auto","localState":"up","latencyMs":1200,"tokensIn":10,"tokensOut":20,"costEstimate":0,"outcome":"ok"}"#;
        std::fs::write(&path, format!("{l1}\n{l2}\n")).unwrap();
        let r = load_recent_at(&path, "20261114", 10);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].reason, "retry", "newest first");
        assert_eq!(r.entries[0].tier, "local");
        assert_eq!(r.entries[1].latency_ms, 30_088);
        assert_eq!(r.latest().unwrap().model, "Lapis Occultus");
        assert_eq!(short_time(&r.entries[1].ts), "09:21:51");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_corrupt_lines_are_skipped() {
        let dir = std::env::temp_dir().join(format!("routes-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ledger.jsonl");
        std::fs::write(&path, "not json\n{}\n").unwrap();
        let r = load_recent_at(&path, "20261114", 10);
        assert_eq!(r.entries.len(), 1, "empty object still parses (defaults)");
        assert!(r.entries[0].ts.is_empty());
        let missing = load_recent_at(&dir.join("nope.jsonl"), "20261114", 10);
        assert!(missing.entries.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
