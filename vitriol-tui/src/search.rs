//! One-shot Hermetis search.
//!
//! Search is user-initiated (Enter in the HERMETIS tab), so it runs in a
//! short-lived thread posting to `/hermetis/search` and sending the formatted
//! hits back to the UI. The poller never calls this — the query is human.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use serde_json::json;
use ureq::AgentBuilder;

use crate::config::Config;

/// One retrieval hit rendered for display.
#[derive(Debug, Clone)]
pub struct SearchHit {
    /// Hit type: `episode` or `node`.
    pub kind: String,
    /// Formatted content snippet.
    pub content: String,
    /// Retrieval score.
    pub score: f64,
    /// Source label.
    pub source: String,
}

/// Run a search in the background, sending hits (or an empty vec) on `tx`.
pub fn spawn(cfg: Config, project_id: String, query: String, tx: Sender<Vec<SearchHit>>) {
    thread::Builder::new()
        .name("vitriol-tui-search".into())
        .spawn(move || {
            let _ = tx.send(search(&cfg, &project_id, &query));
        })
        .expect("spawn vitriol-tui search thread");
}

/// Execute the search synchronously and return the formatted hits.
fn search(cfg: &Config, project_id: &str, query: &str) -> Vec<SearchHit> {
    // Retrieval embeds the query on the CPU bge server and takes ~15 s, so the
    // timeout must be generous; the work runs on a background thread.
    let agent = AgentBuilder::new().timeout(Duration::from_secs(30)).build();
    let url = format!("{}/hermetis/search", cfg.hermetis_base());
    let body = json!({"project_id": project_id, "query": query, "top_k": 5});
    let Ok(resp) = agent.post(&url).send_json(body) else {
        return Vec::new();
    };
    let Ok(payload) = resp.into_json::<serde_json::Value>() else {
        return Vec::new();
    };
    let Some(results) = payload.get("results").and_then(|r| r.as_array()) else {
        return Vec::new();
    };
    results
        .iter()
        .filter_map(|r| {
            Some(SearchHit {
                kind: r.get("type")?.as_str()?.to_string(),
                content: r.get("content")?.as_str()?.to_string(),
                score: r.get("score")?.as_f64()?,
                source: r.get("source")?.as_str()?.to_string(),
            })
        })
        .collect()
}
