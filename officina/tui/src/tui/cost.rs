// /cost data — the routing ledger is the FACTS file (exec-infra stage 5).
// Written by _shared/route-ledger.ts (camelCase fields, one JSON object per
// line, day-stamped). The TUI only READS it; euro amounts come exclusively
// from the ledger's own costEstimate field (I5: no invented pricing).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LedgerEntry {
    #[serde(rename = "tier")]
    pub _tier: String,
    pub provider: Option<String>,
    #[serde(rename = "model")]
    pub _model: Option<String>,
    #[serde(rename = "tokensIn")]
    pub tokens_in: u64,
    #[serde(rename = "tokensOut")]
    pub tokens_out: u64,
    #[serde(rename = "costEstimate")]
    pub cost_estimate: f64,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ModelRow {
    pub key: String,
    pub requests: u64,
    pub failures: u64,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost: f64,
}

#[derive(Debug, Clone, Default)]
pub struct CostReport {
    pub day: String,
    pub rows: Vec<ModelRow>,
    pub total_in: u64,
    pub total_out: u64,
    pub total_cost: f64,
    /// From caduceus-state.json — may be absent (then the bars are skipped).
    pub budget_daily_tokens: u64,
    pub budget_daily_cap: u64,
    pub budget_monthly_tokens: u64,
    pub budget_monthly_cap: u64,
    pub budget_allowed: bool,
    pub budget_present: bool,
}

impl CostReport {
    /// Daily usage as a 0..=1 fraction of the cap (cap 0 → 0).
    pub fn daily_fraction(&self) -> f32 {
        if self.budget_daily_cap == 0 {
            0.0
        } else {
            (self.budget_daily_tokens as f32 / self.budget_daily_cap as f32).clamp(0.0, 1.0)
        }
    }

    pub fn monthly_fraction(&self) -> f32 {
        if self.budget_monthly_cap == 0 {
            0.0
        } else {
            (self.budget_monthly_tokens as f32 / self.budget_monthly_cap as f32).clamp(0.0, 1.0)
        }
    }
}

pub fn ledger_path(day: &str) -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push(format!("route-ledger-{day}.jsonl"));
    p
}

pub fn today_stamp(now: &str) -> String {
    // now = chrono-free: pass e.g. "2026-11-12" from the caller (Local date).
    now.to_string()
}

pub fn parse_line(line: &str) -> Option<LedgerEntry> {
    serde_json::from_str(line).ok()
}

pub fn load_at(path: &Path, day: &str) -> CostReport {
    let mut report = CostReport {
        day: day.to_string(),
        ..Default::default()
    };
    // Budget facts ride along from the caduceus snapshot (absent → skip bars).
    if let Some(state) = crate::tui::caduceus::load() {
        report.budget_daily_tokens = state.budget.daily_tokens;
        report.budget_daily_cap = state.budget.daily_cap;
        report.budget_monthly_tokens = state.budget.monthly_tokens;
        report.budget_monthly_cap = state.budget.monthly_cap;
        report.budget_allowed = state.budget.allowed;
        report.budget_present = true;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return report;
    };
    let mut by_model: BTreeMap<String, ModelRow> = BTreeMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Some(e) = parse_line(line) else {
            continue;
        };
        let key = match (&e.provider, &e._model) {
            (Some(p), Some(m)) => format!("{p}/{m}"),
            (Some(p), None) => p.clone(),
            _ => "unknown".to_string(),
        };
        let row = by_model.entry(key).or_insert_with(|| ModelRow {
            key: String::new(),
            ..Default::default()
        });
        row.requests += 1;
        if e.outcome.as_deref() == Some("fail") || e.outcome.as_deref() == Some("error") {
            row.failures += 1;
        }
        row.tokens_in += e.tokens_in;
        row.tokens_out += e.tokens_out;
        row.cost += e.cost_estimate;
        report.total_in += e.tokens_in;
        report.total_out += e.tokens_out;
        report.total_cost += e.cost_estimate;
    }
    let mut rows: Vec<ModelRow> = by_model
        .into_iter()
        .map(|(key, mut r)| {
            r.key = key;
            r
        })
        .collect();
    rows.sort_by(|a, b| {
        b.tokens_in
            .cmp(&a.tokens_in)
            .then_with(|| a.key.cmp(&b.key))
    });
    report.rows = rows;
    report
}

pub fn load(day: &str) -> CostReport {
    load_at(&ledger_path(day), day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_ledger_fields() {
        let line = r#"{"ts":"2026-11-12T14:59:21.716Z","reason":"user","tier":"cloud","provider":"caduceus","model":"opencode-go/deepseek-v4.1-flash","profile":"auto","localState":"down","latencyMs":4891,"tokensIn":15580,"tokensOut":10,"cacheRead":0,"cacheWrite":0,"costEstimate":0,"outcome":"ok"}"#;
        let e = parse_line(line).expect("parses");
        assert_eq!(e.tokens_in, 15_580);
        assert_eq!(e.tokens_out, 10);
        assert_eq!(e.cost_estimate, 0.0);
        assert_eq!(e.outcome.as_deref(), Some("ok"));
    }

    #[test]
    fn aggregates_by_model_and_counts_failures() {
        let dir = std::env::temp_dir().join(format!("cost-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("route-ledger-test.jsonl");
        fs::write(
            &path,
            concat!(
                r#"{"tier":"cloud","provider":"caduceus","model":"a/b","tokensIn":100,"tokensOut":5,"costEstimate":0.0,"outcome":"ok"}"#, "\n",
                r#"{"tier":"cloud","provider":"caduceus","model":"a/b","tokensIn":50,"tokensOut":3,"costEstimate":0.01,"outcome":"fail"}"#, "\n",
                r#"{"tier":"local","provider":"llamacpp","model":"qwen","tokensIn":1000,"tokensOut":100,"costEstimate":0.0,"outcome":"ok"}"#, "\n",
                "not json\n",
            ),
        )
        .unwrap();
        let report = load_at(&path, "test");
        assert_eq!(report.rows.len(), 2);
        assert_eq!(report.rows[0].key, "llamacpp/qwen"); // 1000 in > 150 in
        let ab = report
            .rows
            .iter()
            .find(|r| r.key == "caduceus/a/b")
            .unwrap();
        assert_eq!(ab.requests, 2);
        assert_eq!(ab.failures, 1);
        assert_eq!(ab.tokens_in, 150);
        assert_eq!(report.total_cost, 0.01);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn budget_fractions_clamp() {
        let mut r = CostReport {
            budget_daily_tokens: 15_801_870,
            budget_daily_cap: 2_000_000,
            budget_monthly_tokens: 45_672_814,
            budget_monthly_cap: 40_000_000,
            ..Default::default()
        };
        assert_eq!(r.daily_fraction(), 1.0);
        assert_eq!(r.monthly_fraction(), 1.0);
        r.budget_daily_tokens = 1_000_000;
        r.budget_daily_cap = 2_000_000;
        assert!((r.daily_fraction() - 0.5).abs() < 1e-6);
        r.budget_daily_cap = 0;
        assert_eq!(r.daily_fraction(), 0.0);
    }
}
