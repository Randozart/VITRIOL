//! Consolidation ("sleep") — port of `hermetis/consolidate.py`.
//!
//! Summarizes batches of raw episodes into dense knowledge nodes and decays /
//! prunes old data. The idle-triggered background loop is a tokio task that the
//! server starts; `mark_active()` resets the idle timer on each request.

use std::path::Path;

use rusqlite::params;

use crate::db::{EdgeSpec, Hermes};

/// Consolidation env knobs (Python defaults).
pub fn consolidate_every() -> usize {
    env_usize("MEMORY_CONSOLIDATE_EVERY", 50)
}
pub fn idle_seconds() -> u64 {
    env_usize("MEMORY_IDLE_SECONDS", 60) as u64
}
pub fn retention_days() -> i64 {
    env_usize("MEMORY_RETENTION_DAYS", 30) as i64
}
pub fn node_decay() -> f64 {
    std::env::var("MEMORY_NODE_DECAY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.95)
}

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Discover all project dirs in the memory root that hold a memory.db.
pub fn get_active_projects(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("memory.db").exists() {
            if let Some(name) = p.file_name() {
                out.push(name.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

/// Episodes not yet linked by a `consolidated_from` edge (oldest first).
fn unconsolidated_batch(
    conn: &rusqlite::Connection,
    batch_size: usize,
) -> rusqlite::Result<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT e.* FROM episodes e
         LEFT JOIN edges ed ON ed.to_type = 'episode'
                           AND ed.to_id = e.id
                           AND ed.relation = 'consolidated_from'
         WHERE ed.id IS NULL
         ORDER BY e.id ASC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([batch_size as i64], |r| {
        let mut map = serde_json::Map::new();
        let ncols = r.as_ref().column_count();
        for i in 0..ncols {
            let name = r.as_ref().column_name(i).unwrap_or("").to_string();
            match r.get_ref(i) {
                Ok(rusqlite::types::ValueRef::Integer(v)) => {
                    map.insert(name, serde_json::json!(v));
                }
                Ok(rusqlite::types::ValueRef::Real(v)) => {
                    map.insert(name, serde_json::json!(v));
                }
                Ok(rusqlite::types::ValueRef::Text(v)) => {
                    map.insert(name, serde_json::json!(String::from_utf8_lossy(v)));
                }
                Ok(rusqlite::types::ValueRef::Blob(v)) => {
                    map.insert(name, serde_json::json!(v.len()));
                }
                Ok(rusqlite::types::ValueRef::Null) => {
                    map.insert(name, serde_json::Value::Null);
                }
                Err(_) => {}
            }
        }
        Ok(serde_json::Value::Object(map))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Build the summary from a batch — Python `_generate_summary` (deterministic).
fn generate_summary(episodes: &[serde_json::Value]) -> String {
    if episodes.is_empty() {
        return String::new();
    }
    let mut roles: Vec<String> = Vec::new();
    let mut total_chars = 0usize;
    let mut topics: Vec<String> = Vec::new();
    for ep in episodes {
        let role = ep.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if !roles.iter().any(|r| r == role) {
            roles.push(role.to_string());
        }
        let content = ep
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        total_chars += content.len();
        let first_line: String = content
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(80)
            .collect();
        if !first_line.trim().is_empty() && !topics.iter().any(|t| t == &first_line) {
            topics.push(first_line);
        }
    }
    let mut parts = vec![
        format!("Consolidated {} episodes", episodes.len()),
        format!("Roles: {}", roles.join(", ")),
        format!("Total length: ~{total_chars} chars"),
        format!(
            "Topics: {}",
            topics
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ),
    ];
    let mut key_exchanges: Vec<String> = Vec::new();
    for ep in episodes.iter().rev().take(5) {
        let role = ep.get("role").and_then(|r| r.as_str()).unwrap_or("user");
        let content = ep
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();
        if !content.is_empty() {
            let snippet: String = content.chars().take(200).collect();
            key_exchanges.push(format!("[{role}]: {snippet}"));
        }
    }
    if !key_exchanges.is_empty() {
        parts.push("---".to_string());
        parts.extend(key_exchanges);
    }
    parts.join("\n")
}

/// Run one consolidation pass for a project. Returns the node id created, or
/// None when skipped (too few episodes / already exists).
pub fn consolidate_project(h: &Hermes, project_id: &str) -> Result<Option<i64>, String> {
    let every = consolidate_every();
    let conn = h.conn(project_id).map_err(|e| e.to_string())?;
    let batch = unconsolidated_batch(&conn, every).map_err(|e| e.to_string())?;
    if batch.len() < 10 {
        return Ok(None);
    }
    let summary = generate_summary(&batch);
    if summary.is_empty() {
        return Ok(None);
    }
    let first_id = batch[0].get("id").and_then(|i| i.as_i64()).unwrap_or(0);
    let last_id = batch[batch.len() - 1]
        .get("id")
        .and_then(|i| i.as_i64())
        .unwrap_or(0);
    let label = format!("consolidated_{first_id}_{last_id}");
    // INSERT OR IGNORE -> lastrowid 0 when the row already exists.
    conn.execute(
        "INSERT OR IGNORE INTO knowledge_nodes
         (label, summary, source_min, source_max, strength)
         VALUES (?1, ?2, ?3, ?4, 1.0)",
        params![label, summary, first_id, last_id],
    )
    .map_err(|e| e.to_string())?;
    let node_id = conn.last_insert_rowid();
    if node_id == 0 {
        return Ok(None);
    }
    for ep in &batch {
        let ep_id = ep.get("id").and_then(|i| i.as_i64()).unwrap_or(0);
        let spec = EdgeSpec::new("node", node_id, "episode", ep_id, "consolidated_from");
        h.get_or_create_edge(project_id, &spec)
            .map_err(|e| e.to_string())?;
    }
    conn.execute(
        "UPDATE knowledge_nodes
         SET strength = MAX(0.3, strength * ?1)
         WHERE created_at < datetime('now', '-7 days')",
        [node_decay()],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM episodes
         WHERE created_at < datetime('now', ?1 || ' days')
         AND id NOT IN (SELECT to_id FROM edges WHERE relation = 'consolidated_from')",
        [(-retention_days()).to_string()],
    )
    .map_err(|e| e.to_string())?;
    Ok(Some(node_id))
}

/// Idle-triggered consolidation worker (the Python `ConsolidationThread`).
pub struct ConsolidationWorker {
    last_request: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl Default for ConsolidationWorker {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsolidationWorker {
    pub fn new() -> Self {
        Self {
            last_request: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(now_secs())),
        }
    }

    /// Reset the idle timer (called on each request).
    pub fn mark_active(&self) {
        self.last_request
            .store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    }

    /// Shared ticker handle (so the server middleware can bump it too).
    pub fn ticker(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        self.last_request.clone()
    }

    /// Run the check loop until cancelled. Mirrors Python: every 30s, if idle
    /// >= IDLE_SECONDS, consolidate every active project.
    pub async fn run_loop(self, h: std::sync::Arc<Hermes>) {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            consolidate_tick(&h, &self.last_request).await;
        }
    }
}

/// One idle-tick: consolidate every active project when idle long enough.
async fn consolidate_tick(h: &std::sync::Arc<Hermes>, last_request: &std::sync::atomic::AtomicU64) {
    let last = last_request.load(std::sync::atomic::Ordering::Relaxed);
    let idle = now_secs().saturating_sub(last);
    if idle < idle_seconds() {
        return;
    }
    for project in get_active_projects(h.root()) {
        let _ = tokio::task::spawn_blocking({
            let h = h.clone();
            let project = project.clone();
            move || consolidate_project(&h, &project)
        })
        .await;
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EpisodeMeta, EpisodeWrite};

    fn tmp_store(name: &str) -> (std::path::PathBuf, Hermes) {
        let root = std::env::temp_dir().join(format!("hermes_consolidate_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        (root.clone(), Hermes::new(&root))
    }

    #[test]
    fn consolidates_batch_into_node() {
        let (root, h) = tmp_store("basic");
        h.get_or_create_session("p", "s").unwrap();
        for i in 0..12 {
            h.store_episode(
                "p",
                "s",
                &EpisodeWrite {
                    role: "user".into(),
                    content: format!("topic alpha note {i}"),
                    meta: EpisodeMeta::default(),
                },
            )
            .unwrap();
        }
        let node = consolidate_project(&h, "p").unwrap();
        assert!(node.is_some());
        // second pass: all episodes now linked -> skipped
        let node2 = consolidate_project(&h, "p").unwrap();
        assert!(node2.is_none());
        // the node links to all 12 episodes
        let targets = h.edge_targets("p", "node", node.unwrap()).unwrap();
        assert_eq!(targets.len(), 12);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn too_few_episodes_skips() {
        let (root, h) = tmp_store("few");
        h.get_or_create_session("p", "s").unwrap();
        h.store_episode(
            "p",
            "s",
            &EpisodeWrite {
                role: "user".into(),
                content: "only one".into(),
                meta: EpisodeMeta::default(),
            },
        )
        .unwrap();
        assert!(consolidate_project(&h, "p").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn summary_is_deterministic() {
        let eps = vec![
            serde_json::json!({"role": "user", "content": "alpha beta gamma"}),
            serde_json::json!({"role": "assistant", "content": "gamma delta"}),
        ];
        let a = generate_summary(&eps);
        let b = generate_summary(&eps);
        assert_eq!(a, b);
        assert!(a.contains("Consolidated 2 episodes"));
        assert!(a.contains("Topics: alpha beta gamma"));
    }
}
