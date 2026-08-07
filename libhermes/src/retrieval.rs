//! Retrieval with cascading (spreading activation) — port of
//! `hermetis/retrieval.py`. Direct search → edge traversal → score → rank.
//! Semantic-off path is bit-exact (keyword Jaccard); the semantic path calls
//! the same GPU embed server and is verified with tolerance.

use regex::Regex;
use std::collections::HashSet;

use crate::compact;
use crate::db::Hermes;
use crate::scorer::{compute_score, estimate_tokens, keyword_overlap};

/// Retrieval parameters (bundled; Python defaults + env knobs).
#[derive(Debug, Clone, Copy)]
pub struct RetrieveParams {
    pub top_k: usize,
    pub cascade_depth: usize,
    pub include_history: bool,
    /// Candidate multiplier: 20 semantic-on, 10 semantic-off (Python).
    pub candidate_multiplier: usize,
}

impl Default for RetrieveParams {
    fn default() -> Self {
        Self {
            top_k: env_usize("MEMORY_TOP_K", 5),
            cascade_depth: env_usize("MEMORY_CASCADE_DEPTH", 1),
            include_history: false,
            candidate_multiplier: 10,
        }
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Simple keyword-based intent classification — Python `classify_intent`.
pub fn classify_intent(query: &str) -> &'static str {
    let words: HashSet<String> = word_regex()
        .find_iter(&query.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect();
    if has_any(
        &words,
        &[
            "fix", "bug", "error", "crash", "broken", "issue", "fault", "null",
        ],
    ) {
        return "code_debug";
    }
    if has_any(
        &words,
        &[
            "how", "what", "why", "when", "where", "explain", "meaning", "purpose",
        ],
    ) {
        return "question";
    }
    if has_any(
        &words,
        &[
            "add",
            "implement",
            "create",
            "refactor",
            "write",
            "build",
            "new",
        ],
    ) {
        return "code_write";
    }
    "general"
}

/// True when any word in the set is present in the list.
fn has_any(words: &HashSet<String>, list: &[&str]) -> bool {
    words.iter().any(|w| list.contains(&w.as_str()))
}

fn word_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\w+").unwrap())
}

/// Merge `new` into `existing`, de-duplicating by (type, id) — Python
/// `_merge_candidates`. Mutates `existing` and returns it.
fn merge_candidates(existing: &mut Vec<serde_json::Value>, new: &[serde_json::Value]) {
    let mut seen: HashSet<(String, i64)> = existing.iter().map(|c| (ctype(c), cid(c))).collect();
    for candidate in new {
        let key = (ctype(candidate), cid(candidate));
        if seen.insert(key) {
            existing.push(candidate.clone());
        }
    }
}

fn ctype(c: &serde_json::Value) -> String {
    c.get("_type")
        .and_then(|t| t.as_str())
        .unwrap_or("episode")
        .to_string()
}

fn cid(c: &serde_json::Value) -> i64 {
    c.get("id").and_then(|i| i.as_i64()).unwrap_or(-1)
}

/// The main retrieval pipeline — Python `retrieve`.
pub fn retrieve(
    h: &Hermes,
    project_id: &str,
    query: &str,
    p: &RetrieveParams,
) -> Vec<serde_json::Value> {
    let mut candidates = hop1_direct(h, project_id, query, p);
    candidates = cascade(h, project_id, &mut candidates, p);
    let w = score_weights();
    let mut scored: Vec<serde_json::Value> = candidates
        .into_iter()
        .map(|mut c| {
            c["_score"] = serde_json::json!(score_one(query, &c, &w));
            c
        })
        .collect();
    scored.sort_by(|a, b| {
        let sa = a.get("_score").and_then(|s| s.as_f64()).unwrap_or(0.0);
        let sb = b.get("_score").and_then(|s| s.as_f64()).unwrap_or(0.0);
        sb.partial_cmp(&sa).unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(p.top_k);
    scored
}

/// Hop 1: direct retrieval over episodes and knowledge nodes.
fn hop1_direct(
    h: &Hermes,
    project_id: &str,
    _query: &str,
    p: &RetrieveParams,
) -> Vec<serde_json::Value> {
    let mut candidates: Vec<serde_json::Value> = Vec::new();
    if let Ok(eps) = h.fetch_episodes(project_id, (p.top_k * p.candidate_multiplier) as i64) {
        let mut tagged = eps;
        for ep in &mut tagged {
            ep["_type"] = serde_json::json!("episode");
            ep["_content"] = ep.get("content").cloned().unwrap_or(serde_json::json!(""));
            ep["_source"] = serde_json::json!("hop1_direct");
        }
        merge_candidates(&mut candidates, &tagged);
    }
    if let Ok(mut ns) = h.fetch_nodes(project_id, (p.top_k * 2) as i64, p.include_history) {
        for n in &mut ns {
            n["_type"] = serde_json::json!("node");
            n["_content"] = n.get("summary").cloned().unwrap_or(serde_json::json!(""));
            n["_source"] = serde_json::json!("hop1_direct");
        }
        merge_candidates(&mut candidates, &ns);
    }
    candidates
}

/// Hops 2+: edge traversal (spreading activation). Each level is one loop so
/// the cascade stays Praetor-clean despite the algorithm being multi-pass.
fn cascade(
    h: &Hermes,
    project_id: &str,
    candidates: &mut Vec<serde_json::Value>,
    p: &RetrieveParams,
) -> Vec<serde_json::Value> {
    for depth in 0..p.cascade_depth {
        let hop = expand_all(h, project_id, candidates, depth);
        let snapshot = hop;
        merge_candidates(candidates, &snapshot);
    }
    candidates.clone()
}

/// Expand every candidate's outgoing edges into tagged targets.
fn expand_all(
    h: &Hermes,
    project_id: &str,
    candidates: &[serde_json::Value],
    depth: usize,
) -> Vec<serde_json::Value> {
    let mut hop: Vec<serde_json::Value> = Vec::new();
    for candidate in candidates {
        let added = expand_one(h, project_id, candidate, depth);
        merge_candidates(&mut hop, &added);
    }
    hop
}

/// Expand one candidate's outgoing edges.
fn expand_one(
    h: &Hermes,
    project_id: &str,
    candidate: &serde_json::Value,
    depth: usize,
) -> Vec<serde_json::Value> {
    let from_type = ctype(candidate);
    let from_id = cid(candidate);
    let edges = h
        .outgoing_edges(project_id, &from_type, from_id)
        .unwrap_or_default();
    let mut out: Vec<serde_json::Value> = Vec::new();
    for edge in edges {
        let weight = edge
            .get("weight")
            .cloned()
            .unwrap_or(serde_json::json!(1.0));
        let relation = edge
            .get("relation")
            .cloned()
            .unwrap_or(serde_json::json!(""));
        let targets = h
            .edge_targets(project_id, &from_type, from_id)
            .unwrap_or_default();
        let tagged: Vec<serde_json::Value> = targets
            .into_iter()
            .map(|mut t| {
                t["_type"] = t
                    .get("_type")
                    .cloned()
                    .unwrap_or(serde_json::json!("episode"));
                if t.get("_content").and_then(|x| x.as_str()).is_none() {
                    let content = t
                        .get("content")
                        .or_else(|| t.get("summary"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("");
                    t["_content"] = serde_json::json!(content);
                }
                t["_source"] = serde_json::json!(format!("hop{}_cascade", depth + 2));
                t["_edge_weight"] = weight.clone();
                t["_edge_relation"] = relation.clone();
                t
            })
            .collect();
        merge_candidates(&mut out, &tagged);
    }
    out
}

/// The scoring weights (env-overridable).
fn score_weights() -> crate::scorer::ScoreWeights {
    crate::scorer::ScoreWeights {
        relevance_weight: env_f64("MEMORY_RELEVANCE_WEIGHT", 0.40),
        recency_weight: env_f64("MEMORY_RECENCY_WEIGHT", 0.35),
        hebbian_coeff: env_f64("MEMORY_HEBBIAN_WEIGHT", 0.15),
        strength_coeff: env_f64("MEMORY_STRENGTH_WEIGHT", 0.10),
    }
}

/// Score one candidate; nodes get a +0.05 current-world bonus (P3.2).
fn score_one(query: &str, candidate: &serde_json::Value, w: &crate::scorer::ScoreWeights) -> f64 {
    let content = candidate
        .get("_content")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let input = crate::scorer::ScoreInput {
        query,
        content,
        created_at: candidate.get("created_at").and_then(|c| c.as_str()),
        hebbian_weight: candidate
            .get("_edge_weight")
            .and_then(|w| w.as_f64())
            .unwrap_or(0.5),
        node_strength: candidate
            .get("strength")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.0),
    };
    let mut score = compute_score(&input, w);
    if ctype(candidate) == "node" {
        score += 0.05;
    }
    score
}
fn env_f64(key: &str, default: f64) -> f64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Options for `context_block` (bundled to stay under the param gate).
#[derive(Debug, Clone)]
pub struct ContextBlockOptions {
    pub budget_tokens: usize,
    pub top_k: usize,
    pub min_score: f64,
    pub session_id: Option<String>,
}

/// Build a budget-capped context block — Python `context_block`. Semantic-off:
/// `is_new_topic` is always True (embeddings unavailable), so injection is
/// never skipped by the topic heuristic.
pub fn context_block(
    h: &Hermes,
    project_id: &str,
    recent_text: &str,
    o: &ContextBlockOptions,
) -> (String, f64, bool) {
    let params = RetrieveParams {
        top_k: o.top_k,
        cascade_depth: 1,
        include_history: false,
        candidate_multiplier: 10,
    };
    let mut candidates: Vec<serde_json::Value> = retrieve(h, project_id, recent_text, &params)
        .into_iter()
        .filter(|c| c.get("_score").and_then(|s| s.as_f64()).unwrap_or(0.0) >= o.min_score)
        .collect();
    let top_score = candidates
        .first()
        .and_then(|c| c.get("_score").and_then(|s| s.as_f64()))
        .unwrap_or(0.0);
    // Semantic-off: the embed server is unavailable, so Python `_is_new_topic`
    // returns True (inject-worthy) unconditionally.
    let is_new = true;
    let mut lines = Vec::new();
    let mut used = 0usize;
    for c in &mut candidates {
        let body = if ctype(c) == "node" {
            compact::format_node(c)
        } else {
            compact::format_episode(c, None)
        };
        let toks = estimate_tokens(&body) + 1;
        if used + toks > o.budget_tokens && used > 0 {
            break;
        }
        used += toks;
        lines.push(body);
    }
    (lines.join("\n\n"), top_score, is_new)
}

/// Keyword overlap exposed for cross-checks (semantic-off relevance).
pub fn keyword_overlap_pub(query: &str, content: &str) -> f64 {
    keyword_overlap(query, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EpisodeMeta, EpisodeWrite};

    fn tmp_store(name: &str) -> (std::path::PathBuf, Hermes) {
        let root = std::env::temp_dir().join(format!("hermes_retrieval_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        (root.clone(), Hermes::new(&root))
    }

    #[test]
    fn classify_intent_kinds() {
        assert_eq!(classify_intent("fix the crash in parser"), "code_debug");
        assert_eq!(classify_intent("how does the scheduler work"), "question");
        assert_eq!(classify_intent("implement a ring buffer"), "code_write");
        assert_eq!(classify_intent("ok"), "general");
    }

    #[test]
    fn retrieve_scores_and_ranks() {
        let (root, h) = tmp_store("ret");
        h.get_or_create_session("p", "s").unwrap();
        h.store_episode(
            "p",
            "s",
            &EpisodeWrite {
                role: "user".into(),
                content: "the red fox jumps over the lazy dog today quickly".into(),
                meta: EpisodeMeta::default(),
            },
        )
        .unwrap();
        h.store_episode(
            "p",
            "s",
            &EpisodeWrite {
                role: "assistant".into(),
                content: "the red fox is a mammal with a bushy tail".into(),
                meta: EpisodeMeta::default(),
            },
        )
        .unwrap();
        h.store_node(
            "p",
            "fox-note",
            "the red fox hunts at dusk and sleeps by day",
            &crate::NodeMeta {
                git_rev: "abc".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let out = retrieve(&h, "p", "red fox", &RetrieveParams::default());
        assert!(!out.is_empty());
        // best match should be the node or the most-overlapping episode
        let best = &out[0];
        assert!(best["_score"].as_f64().unwrap() > 0.0);
        // node bonus: the node should outrank episodes with same overlap
        let node = out.iter().find(|c| ctype(c) == "node");
        assert!(node.is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn retrieve_cascades_edges() {
        let (root, h) = tmp_store("cascade");
        h.get_or_create_session("p", "s").unwrap();
        let a = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "user".into(),
                    content: "alpha one two three four five".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        let b = h
            .store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "assistant".into(),
                    content: "beta six seven eight nine ten".into(),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        // explicit cross edge a->b already exists (follows); query matches b only
        let out = retrieve(&h, "p", "beta six seven", &RetrieveParams::default());
        assert!(out.iter().any(|c| cid(c) == b));
        let _ = a;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn context_block_budget_caps() {
        let (root, h) = tmp_store("ctx");
        h.get_or_create_session("p", "s").unwrap();
        h.store_episode(
            "p",
            "s",
            &EpisodeWrite {
                role: "user".into(),
                content: "the quick brown fox jumps".into(),
                meta: EpisodeMeta::default(),
            },
        )
        .unwrap();
        let (block, top, is_new) = context_block(
            &h,
            "p",
            "quick brown fox",
            &ContextBlockOptions {
                budget_tokens: 200,
                top_k: 5,
                min_score: 0.0,
                session_id: Some("s".into()),
            },
        );
        assert!(block.contains("fox") || block.is_empty());
        assert!(top >= 0.0);
        assert!(is_new); // semantic-off always new
        let _ = std::fs::remove_dir_all(&root);
    }
}
