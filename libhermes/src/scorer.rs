//! Scoring — byte-parity port of `hermetis/scorer.py` (semantic-off path).
//!
//! Composite scoring = relevance × rel_w + recency × rec_w + hebbian × heb_w +
//! strength × str_w, all normalized to [0,1]. Relevance is keyword Jaccard when
//! semantic mode is off (the P2 exact-parity path); the semantic path calls the
//! same GPU embed server and is verified with tolerance.

use regex::Regex;
use std::collections::HashSet;

/// Lazy `\w+` matcher (unicode-aware, matches Python `re`).
fn word_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\w+").unwrap())
}

/// Rough token estimation (4 chars ≈ 1 token) — Python `estimate_tokens`.
pub fn estimate_tokens(text: &str) -> usize {
    (text.len() / 4).max(1)
}

/// Jaccard similarity of word sets — Python `keyword_overlap`.
pub fn keyword_overlap(query: &str, content: &str) -> f64 {
    let re = word_regex();
    let qw: HashSet<String> = re
        .find_iter(&query.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect();
    let cw: HashSet<String> = re
        .find_iter(&content.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect();
    if qw.is_empty() || cw.is_empty() {
        return 0.0;
    }
    let inter = qw.intersection(&cw).count();
    let union = qw.union(&cw).count();
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

/// Semantic similarity; semantic-off falls back to keyword overlap (exact).
pub fn semantic_similarity(query: &str, content: &str) -> f64 {
    keyword_overlap(query, content)
}

/// Linear recency decay over `max_days`, clamped to [0,1]. Missing dates give a
/// neutral 0.5. Naive timestamps are treated as UTC (Python behavior).
pub fn recency_score(created_at: Option<&str>, max_days: f64) -> f64 {
    use chrono::{DateTime, Utc};
    let Some(created) = created_at else {
        return 0.5;
    };
    let parsed = DateTime::parse_from_rfc3339(created)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // "YYYY-MM-DD HH:MM:SS" (SQLite datetime) or "YYYY-MM-DD"
            chrono::NaiveDateTime::parse_from_str(created, "%Y-%m-%d %H:%M:%S")
                .map(|n| n.and_utc())
                .or_else(|_| {
                    chrono::NaiveDate::parse_from_str(created, "%Y-%m-%d")
                        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
                })
        });
    let Ok(created) = parsed else {
        return 0.5;
    };
    let days_old = (Utc::now() - created).num_milliseconds() as f64 / 86_400_000.0;
    (1.0 - (days_old / max_days)).clamp(0.0, 1.0)
}

/// The per-candidate score inputs.
#[derive(Debug, Clone)]
pub struct ScoreInput<'a> {
    pub query: &'a str,
    pub content: &'a str,
    pub created_at: Option<&'a str>,
    pub hebbian_weight: f64,
    pub node_strength: f64,
}

/// The composite scoring weights (Python `compute_score` coefficients).
#[derive(Debug, Clone, Copy)]
pub struct ScoreWeights {
    pub relevance_weight: f64,
    pub recency_weight: f64,
    pub hebbian_coeff: f64,
    pub strength_coeff: f64,
}

/// The composite score — Python `compute_score`.
pub fn compute_score(input: &ScoreInput<'_>, w: &ScoreWeights) -> f64 {
    let rel = semantic_similarity(input.query, input.content);
    let max_days: f64 = std::env::var("MEMORY_RECENCY_MAX_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30.0);
    let rec = recency_score(input.created_at, max_days);
    let heb = input.hebbian_weight.clamp(0.0, 1.0);
    let strn = input.node_strength.clamp(0.0, 1.0);
    rel * w.relevance_weight
        + rec * w.recency_weight
        + heb * w.hebbian_coeff
        + strn * w.strength_coeff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_tokens_floor_one() {
        assert_eq!(estimate_tokens(""), 1);
        assert_eq!(estimate_tokens("hello"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
    }

    #[test]
    fn keyword_overlap_jaccard() {
        assert_eq!(keyword_overlap("the quick fox", "the fox"), 2.0 / 3.0);
        assert_eq!(keyword_overlap("aaa", "bbb"), 0.0);
        assert_eq!(keyword_overlap("", "bbb"), 0.0);
        // case-insensitive + unicode-aware words
        assert_eq!(keyword_overlap("The Fox", "the fox"), 1.0);
    }

    #[test]
    fn recency_neutral_and_clamped() {
        assert_eq!(recency_score(None, 30.0), 0.5);
        assert_eq!(recency_score(Some("garbage-date"), 30.0), 0.5);
        // a far-future date clamps to 0; long-ago clamps to 1... wait:
        // a very old date -> days_old huge -> 1 - big -> clamp 0.
        assert_eq!(recency_score(Some("2000-01-01 00:00:00"), 30.0), 0.0);
    }

    #[test]
    fn compute_score_is_bounded() {
        let input = ScoreInput {
            query: "q",
            content: "c",
            created_at: None,
            hebbian_weight: 0.5,
            node_strength: 1.0,
        };
        let w = ScoreWeights {
            relevance_weight: 0.4,
            recency_weight: 0.35,
            hebbian_coeff: 0.15,
            strength_coeff: 0.10,
        };
        let s = compute_score(&input, &w);
        assert!((0.0..=1.0).contains(&s));
    }
}
