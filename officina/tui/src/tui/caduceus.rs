// /caduceus modal data — read-only view of the gateway state file the
// caduceus extension writes (2026-11-12, QoL batch stage 2). Contract I4:
// the TUI never talks HTTP to the gateway; it only parses this snapshot
// (written at startup / session switch / every 60 s, atomic tmp+rename).
//
// PROVENANCE: original work, this repo.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Connections {
    pub active: Vec<String>,
    pub backoff: Vec<String>,
    pub hard_down: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeyRow {
    pub provider: String,
    pub env: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub daily_tokens: u64,
    pub daily_cap: u64,
    pub monthly_tokens: u64,
    pub monthly_cap: u64,
    pub allowed: bool,
    pub paid_allowed: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CaduceusState {
    /// ms epoch of the write (staleness rendering)
    pub ts: u64,
    pub base_url: String,
    pub gateway_up: bool,
    pub catalog_count: u64,
    pub catalog_fetched_at: String,
    pub connections: Connections,
    pub cooling: u64,
    pub inference_key: String,
    pub keys: Vec<KeyRow>,
    pub budget: Budget,
}

impl CaduceusState {
    /// Snapshot age in ms (0 when the stamp is from the future — clock skew
    /// must not render negative ages).
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.ts)
    }
}

pub fn state_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("caduceus-state.json");
    p
}

fn parse_state(json: &str) -> Option<CaduceusState> {
    serde_json::from_str(json).ok()
}

pub fn load() -> Option<CaduceusState> {
    load_at(&state_path())
}

pub fn load_at(path: &Path) -> Option<CaduceusState> {
    parse_state(&fs::read_to_string(path).ok()?)
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 999 → "999" · 1234 → "1.2k" · 1_234_000 → "1.2M"
pub fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        trim_dot_zero(format!("{:.1}", n as f64 / 1_000_000.0)) + "M"
    } else if n >= 1_000 {
        trim_dot_zero(format!("{:.1}", n as f64 / 1_000.0)) + "k"
    } else {
        n.to_string()
    }
}

fn trim_dot_zero(mut s: String) -> String {
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    s
}

/// 5_000 ms → "5s" · 120_000 → "2m" · 7_200_000 → "2h" · 172_800_000 → "2d"
pub fn fmt_age_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3_600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3_600)
    } else {
        format!("{}d", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "ts": 1731000000000,
        "base_url": "http://127.0.0.1:20128/v1",
        "gateway_up": true,
        "catalog_count": 123,
        "catalog_fetched_at": "2026-11-11T10:00:00.000Z",
        "connections": {"active": ["a","b"], "backoff": ["c"], "hard_down": ["d"]},
        "cooling": 2,
        "inference_key": "...a1b2",
        "keys": [{"provider":"zai","env":"ZAI_KEY","value":"...c3d4","source":"env-file"}],
        "budget": {"daily_tokens":1200000,"daily_cap":2000000,"monthly_tokens":8000000,
                   "monthly_cap":40000000,"allowed":true,"paid_allowed":false}
    }"#;

    #[test]
    fn parses_full_state() {
        let s = parse_state(SAMPLE).expect("sample parses");
        assert!(s.gateway_up);
        assert_eq!(s.catalog_count, 123);
        assert_eq!(s.connections.active, vec!["a", "b"]);
        assert_eq!(s.connections.hard_down, vec!["d"]);
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].provider, "zai");
        assert_eq!(s.budget.daily_cap, 2_000_000);
        assert!(s.budget.allowed);
        assert!(!s.budget.paid_allowed);
        assert_eq!(s.inference_key, "...a1b2");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s = parse_state("{}").expect("empty object parses");
        assert!(!s.gateway_up);
        assert_eq!(s.catalog_count, 0);
        assert!(s.connections.active.is_empty());
        assert_eq!(s.budget.daily_cap, 0);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_state("not json").is_none());
        assert!(parse_state(r#"{"ts":"oops"}"#).is_none());
    }

    #[test]
    fn load_at_missing_file_is_none() {
        assert!(load_at(Path::new("/nonexistent/caduceus-state.json")).is_none());
    }

    #[test]
    fn age_is_saturating_and_skew_safe() {
        let s = parse_state(SAMPLE).unwrap();
        assert_eq!(s.age_ms(1_731_000_010_000), 10_000);
        assert_eq!(s.age_ms(1_730_999_990_000), 0); // future stamp → 0
    }

    #[test]
    fn fmt_count_buckets() {
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_234), "1.2k");
        assert_eq!(fmt_count(20_000), "20k");
        assert_eq!(fmt_count(1_234_000), "1.2M");
        assert_eq!(fmt_count(40_000_000), "40M");
    }

    #[test]
    fn fmt_age_buckets() {
        assert_eq!(fmt_age_ms(5_000), "5s");
        assert_eq!(fmt_age_ms(59_000), "59s");
        assert_eq!(fmt_age_ms(120_000), "2m");
        assert_eq!(fmt_age_ms(7_200_000), "2h");
        assert_eq!(fmt_age_ms(172_800_000), "2d");
    }
}
