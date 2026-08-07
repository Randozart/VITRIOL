//! Hebbian weight updates — port of `hermetis/hebbian.py`.

use crate::db::{EdgeSpec, Hermes};
use regex::Regex;

/// Lazy `\w+` matcher (matches Python `re`).
fn word_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\w+").unwrap())
}

/// True when a candidate memory is referenced in the LLM response (3+ words of
/// the candidate's first-50-char signature overlap, len > 3) — Python
/// `is_referenced`.
pub fn is_referenced(candidate: &serde_json::Value, response_text: &str) -> bool {
    let content = candidate
        .get("_content")
        .or_else(|| candidate.get("content"))
        .or_else(|| candidate.get("summary"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let signature = content
        .chars()
        .take(50)
        .collect::<String>()
        .trim()
        .to_lowercase();
    if signature.is_empty() {
        return false;
    }
    let re = word_regex();
    let words: std::collections::HashSet<String> = re
        .find_iter(&signature)
        .map(|m| m.as_str().to_string())
        .collect();
    let response_words: std::collections::HashSet<String> = re
        .find_iter(&response_text.to_lowercase())
        .map(|m| m.as_str().to_string())
        .collect();
    let significant = words
        .intersection(&response_words)
        .filter(|w| w.len() > 3)
        .count();
    significant >= 3
}

/// Update Hebbian weights for retrieved candidates — Python `update_weights`:
/// used → +0.1 (cap 3.0), unused → −0.05 (floor 0.0); both-used co-retrieved
/// pairs → +0.05 (cap 3.0).
pub fn update_weights(
    h: &Hermes,
    project_id: &str,
    response_text: &str,
    retrieved: &[serde_json::Value],
) {
    if response_text.is_empty() || retrieved.is_empty() {
        return;
    }
    update_candidate_edges(h, project_id, response_text, retrieved);
    strengthen_co_retrieved(h, project_id, response_text, retrieved);
}

/// Phase 1: strengthen used candidates' outgoing edges, weaken unused ones.
fn update_candidate_edges(
    h: &Hermes,
    project_id: &str,
    response_text: &str,
    retrieved: &[serde_json::Value],
) {
    for candidate in retrieved {
        update_candidate(h, project_id, candidate, response_text);
    }
}

/// Update one candidate's outgoing edges based on whether it was referenced.
fn update_candidate(
    h: &Hermes,
    project_id: &str,
    candidate: &serde_json::Value,
    response_text: &str,
) {
    let used = is_referenced(candidate, response_text);
    let ctype = candidate
        .get("_type")
        .and_then(|t| t.as_str())
        .unwrap_or("episode");
    let Some(cid) = candidate.get("id").and_then(|i| i.as_i64()) else {
        return;
    };
    let Ok(edges) = h.outgoing_edges(project_id, ctype, cid) else {
        return;
    };
    for edge in edges {
        let Some(eid) = edge.get("id").and_then(|i| i.as_i64()) else {
            continue;
        };
        let w = edge.get("weight").and_then(|w| w.as_f64()).unwrap_or(1.0);
        let new_w = if used {
            (w + 0.1).min(3.0)
        } else {
            (w - 0.05).max(0.0)
        };
        let _ = h.update_edge_weight(project_id, eid, new_w);
    }
}

/// Phase 2: strengthen co-retrieved pairs that were both used in the response.
fn strengthen_co_retrieved(
    h: &Hermes,
    project_id: &str,
    response_text: &str,
    retrieved: &[serde_json::Value],
) {
    for (i, j) in pair_indices(retrieved.len()) {
        strengthen_pair(h, project_id, &retrieved[i], &retrieved[j], response_text);
    }
}

/// All (i, j) index pairs with i < j, as a flat iterator.
fn pair_indices(n: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..n).flat_map(move |i| ((i + 1)..n).map(move |j| (i, j)))
}

/// Strengthen one co-retrieved pair when both were used in the response.
fn strengthen_pair(
    h: &Hermes,
    project_id: &str,
    a: &serde_json::Value,
    b: &serde_json::Value,
    response_text: &str,
) {
    if !is_referenced(a, response_text) || !is_referenced(b, response_text) {
        return;
    }
    let a_type = a.get("_type").and_then(|t| t.as_str()).unwrap_or("episode");
    let b_type = b.get("_type").and_then(|t| t.as_str()).unwrap_or("episode");
    let (Some(a_id), Some(b_id)) = (
        a.get("id").and_then(|x| x.as_i64()),
        b.get("id").and_then(|x| x.as_i64()),
    ) else {
        return;
    };
    let spec = EdgeSpec::new(a_type, a_id, b_type, b_id, "co_retrieved");
    if let Ok(edge) = h.get_or_create_edge(project_id, &spec) {
        if let (Some(eid), Some(w)) = (
            edge.get("id").and_then(|x| x.as_i64()),
            edge.get("weight").and_then(|x| x.as_f64()),
        ) {
            let _ = h.update_edge_weight(project_id, eid, (w + 0.05).min(3.0));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_referenced_matches() {
        // Python parity: significant = overlap words with len > 3, need >= 3.
        let cand = json!({"_content": "the quick brown fox jumps over the lazy dog today"});
        assert!(!is_referenced(&cand, "I used the quick brown fox approach")); // only quick,brown
        assert!(is_referenced(
            &cand,
            "the quick brown design today works well"
        )); // quick,brown,design,today,works,well
        assert!(!is_referenced(&cand, "completely unrelated response"));
        assert!(!is_referenced(&json!({"_content": ""}), "anything"));
    }

    #[test]
    fn update_weights_reinforces_used() {
        let root = std::env::temp_dir().join("hermes_hebbian");
        let _ = std::fs::remove_dir_all(&root);
        let h = Hermes::new(&root);
        h.get_or_create_session("p", "s").unwrap();
        let id0 = h
            .store_episode(
                "p",
                "s",
                &crate::EpisodeWrite {
                    role: "user".into(),
                    content: "the quick brown fox design works well".into(),
                    meta: Default::default(),
                },
            )
            .unwrap();
        let _id1 = h
            .store_episode(
                "p",
                "s",
                &crate::EpisodeWrite {
                    role: "assistant".into(),
                    content: "answer".into(),
                    meta: Default::default(),
                },
            )
            .unwrap();
        let cand = json!({"_type": "episode", "id": id0, "_content": "the quick brown fox design works well"});
        let resp = "the quick brown fox design works well approach";
        update_weights(&h, "p", resp, &[cand]);
        let edges = h.outgoing_edges("p", "episode", id0).unwrap();
        assert_eq!(edges.len(), 1); // follows edge
        assert!((edges[0]["weight"].as_f64().unwrap() - 1.1).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&root);
    }
}
