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
}

/// The last decisions, newest first (ledger order is chronological).
#[derive(Debug, Clone, Default)]
pub struct RouteReport {
    pub day: String,
    pub entries: Vec<RouteEntry>,
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
