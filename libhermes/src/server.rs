//! axum HTTP server — port of `hermetis_server.py`'s routes.
//!
//! Same JSON contracts so `plugins/copula.ts` and `vitriol-tui` are untouched.
//! Semantic-off path is exact; `/hermetis/embed` reports 503 when the embed
//! provider is unavailable (mirrors Python). Routes that need un-ported modules
//! (`/hermetis/repo_map`, `/pymander/*`) return an honest "not built yet" error
//! until the P4/P5 phases land.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::middleware;
use axum::routing::{get, post};
use axum::{Json, Router};

use crate::compact;
use crate::db::Hermes;
use crate::retrieval::{self, ContextBlockOptions, RetrieveParams};
use crate::{EpisodeMeta, EpisodeWrite, NodeMeta};

/// Shared server state.
#[derive(Clone)]
pub struct ServerState {
    pub h: Arc<Hermes>,
    /// Optional idle ticker bumped on every request (memory consolidation).
    pub last_request: Option<Arc<std::sync::atomic::AtomicU64>>,
}

/// Build the route table.
pub fn router(state: ServerState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/hermetis/store", post(store))
        .route("/hermetis/embed", post(embed))
        .route("/hermetis/node", post(node))
        .route("/hermetis/search", post(search))
        .route("/hermetis/context", post(context))
        .route("/hermetis/recent", get(recent))
        .route("/hermetis/stats", get(stats))
        .route("/hermetis/repo_map", post(repo_map))
        .route("/pymander/list", get(pymander_list))
        .route("/pymander/search", post(pymander_search))
        .route("/pymander/select", post(pymander_select))
        .route("/pymander/context", post(pymander_context))
        .layer(middleware::from_fn_with_state(state.clone(), mark_active))
        .with_state(state)
}

/// Bump the consolidation idle ticker on every request.
async fn mark_active(
    axum::extract::State(st): axum::extract::State<ServerState>,
    req: axum::http::Request<axum::body::Body>,
    next: middleware::Next,
) -> Result<axum::http::Response<axum::body::Body>, axum::http::StatusCode> {
    if let Some(ticker) = &st.last_request {
        ticker.store(now_secs(), std::sync::atomic::Ordering::Relaxed);
    }
    Ok(next.run(req).await)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Resolve + sanitize a project_id (Python `_project_id`).
fn project_id(payload: &serde_json::Value) -> Option<String> {
    let pid = payload
        .get("project_id")
        .or_else(|| payload.get("project"))
        .and_then(|v| v.as_str())?;
    if pid.is_empty() {
        return None;
    }
    let sanitized: String = pid
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' => '_',
            _ => c,
        })
        .collect();
    Some(sanitized.chars().take(120).collect())
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok", "service": "hermetis"}))
}

async fn store(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let role = payload
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("assistant")
        .to_string();
    let content = payload
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = payload
        .get("session_id")
        .and_then(|s| s.as_str())
        .unwrap_or("default")
        .to_string();
    if content.is_empty() {
        return err(400, "content required");
    }
    let token_count = payload
        .get("token_count")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    let _ = st.h.get_or_create_session(&pid, &session_id);
    let write = EpisodeWrite {
        role,
        content,
        meta: EpisodeMeta {
            token_count,
            ..Default::default()
        },
    };
    match st.h.store_episode(&pid, &session_id, &write) {
        Ok(episode_id) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "episode_id": episode_id, "project_id": pid})),
        ),
        Err(e) => err(500, &format!("store failed: {e}")),
    }
}

async fn embed() -> (StatusCode, Json<serde_json::Value>) {
    // P3 semantic-off: no embed provider yet -> 503 (mirrors Python).
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"ok": false, "error": "embed server unavailable"})),
    )
}

async fn node(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let label = payload.get("label").and_then(|l| l.as_str()).unwrap_or("");
    let summary = payload
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    if label.is_empty() || summary.is_empty() {
        return err(400, "label and summary required");
    }
    let meta = NodeMeta {
        strength: payload
            .get("strength")
            .and_then(|s| s.as_f64())
            .unwrap_or(1.0),
        source_min: payload.get("source_min").and_then(|s| s.as_i64()),
        source_max: payload.get("source_max").and_then(|s| s.as_i64()),
        ..Default::default()
    };
    match st.h.store_node(&pid, label, summary, &meta) {
        Ok(node_id) => (
            StatusCode::OK,
            Json(serde_json::json!({"ok": true, "node_id": node_id, "project_id": pid})),
        ),
        Err(e) => err(500, &format!("node failed: {e}")),
    }
}

async fn search(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let query = payload.get("query").and_then(|q| q.as_str()).unwrap_or("");
    if query.is_empty() {
        return err(400, "query required");
    }
    let p = RetrieveParams {
        top_k: payload.get("top_k").and_then(|t| t.as_i64()).unwrap_or(5) as usize,
        cascade_depth: payload
            .get("cascade_depth")
            .and_then(|t| t.as_i64())
            .unwrap_or(1) as usize,
        include_history: payload
            .get("include_history")
            .and_then(|b| b.as_bool())
            .unwrap_or(false),
        candidate_multiplier: 10,
    };
    let candidates = retrieval::retrieve(&st.h, &pid, query, &p);
    let results: Vec<serde_json::Value> = candidates
        .iter()
        .map(|c| {
            let ctype = c.get("_type").and_then(|t| t.as_str()).unwrap_or("episode");
            let body = if ctype == "node" {
                compact::format_node(c)
            } else {
                compact::format_episode(c, None)
            };
            serde_json::json!({
                "type": ctype,
                "content": body,
                "score": (c.get("_score").and_then(|s| s.as_f64()).unwrap_or(0.0) * 10000.0).round() / 10000.0,
                "source": c.get("_source").and_then(|s| s.as_str()).unwrap_or(""),
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true, "query": query, "results": results,
            "count": results.len(), "project_id": pid
        })),
    )
}

async fn context(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let recent_text = payload
        .get("recent_text")
        .and_then(|r| r.as_str())
        .unwrap_or("");
    if recent_text.is_empty() {
        return err(400, "recent_text required");
    }
    let o = ContextBlockOptions {
        budget_tokens: payload
            .get("budget_tokens")
            .and_then(|b| b.as_i64())
            .unwrap_or(1500) as usize,
        top_k: payload.get("top_k").and_then(|t| t.as_i64()).unwrap_or(5) as usize,
        min_score: payload
            .get("min_score")
            .and_then(|m| m.as_f64())
            .unwrap_or(0.3),
        session_id: payload
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(str::to_string),
    };
    let (block, top_score, is_new) = retrieval::context_block(&st.h, &pid, recent_text, &o);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true, "project_id": pid,
            "tokens": crate::scorer::estimate_tokens(&block),
            "top_score": (top_score * 10000.0).round() / 10000.0,
            "is_new_topic": is_new,
            "context": block,
        })),
    )
}

async fn recent(
    State(st): State<ServerState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(
        &serde_json::json!({"project_id": params.get("project_id").or_else(|| params.get("project")).cloned()}),
    ) else {
        return err(400, "project_id required");
    };
    let limit = params
        .get("limit")
        .and_then(|l| l.parse::<i64>().ok())
        .unwrap_or(5)
        .clamp(1, 20);
    let conn = match st.h.conn(&pid) {
        Ok(c) => c,
        Err(e) => return err(500, &format!("open failed: {e}")),
    };
    let recent = recent_rows(&conn, limit);
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "project_id": pid, "recent": recent})),
    )
}

/// The most recent episodes (newest first), snippet-truncated — Python
/// `memory_recent` row shape.
fn recent_rows(conn: &rusqlite::Connection, limit: i64) -> Vec<serde_json::Value> {
    let mut stmt = conn
        .prepare("SELECT id, role, content, created_at FROM episodes ORDER BY id DESC LIMIT ?1")
        .unwrap();
    let rows = stmt
        .query_map([limit], |r| {
            let id: i64 = r.get(0)?;
            let role: String = r.get(1)?;
            let content: String = r.get(2)?;
            let created: String = r.get(3)?;
            let snippet: String = content.chars().take(180).collect();
            Ok(serde_json::json!({"id": id, "role": role, "snippet": snippet, "created_at": created}))
        })
        .unwrap();
    rows.flatten().collect()
}

async fn stats(
    State(st): State<ServerState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(
        &serde_json::json!({"project_id": params.get("project_id").or_else(|| params.get("project")).cloned()}),
    ) else {
        return err(400, "project_id required");
    };
    let conn = match st.h.conn(&pid) {
        Ok(c) => c,
        Err(e) => return err(500, &format!("open failed: {e}")),
    };
    let count = |table: &str| -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap_or(0)
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "project_id": pid,
            "episodes": count("episodes"),
            "nodes": count("knowledge_nodes"),
            "sessions": count("sessions"),
        })),
    )
}

async fn repo_map(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let root_str = payload.get("root").and_then(|r| r.as_str()).unwrap_or("");
    let root = std::path::Path::new(root_str);
    if root_str.is_empty() || !root.is_dir() {
        return err(400, "root must be a directory");
    }
    let budget = payload
        .get("budget_tokens")
        .and_then(|b| b.as_i64())
        .unwrap_or(1000) as usize;
    let do_store = payload
        .get("store")
        .and_then(|s| s.as_bool())
        .unwrap_or(true);
    let max_files = payload
        .get("max_files")
        .and_then(|m| m.as_i64())
        .map(|v| v as usize);
    let single_file = payload.get("file").and_then(|f| f.as_str());
    let (map_text, stored) = if let Some(file) = single_file {
        let n = crate::repomap::store_file_nodes(&st.h, &pid, root, &[file.to_string()]);
        (crate::repomap::build_repo_map(root, budget, max_files), n)
    } else if do_store {
        crate::repomap::store_repo_map(&st.h, &pid, root, budget, max_files)
    } else {
        (crate::repomap::build_repo_map(root, budget, max_files), 0)
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true, "project_id": pid, "nodes_stored": stored,
            "map_tokens": crate::scorer::estimate_tokens(&map_text), "map": map_text,
        })),
    )
}

async fn pymander_list(State(st): State<ServerState>) -> (StatusCode, Json<serde_json::Value>) {
    let domains = crate::pymander::list_domains(st.h.root());
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "domains": domains})),
    )
}

async fn pymander_search(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let domain = payload.get("domain").and_then(|d| d.as_str()).unwrap_or("");
    let query = payload.get("query").and_then(|q| q.as_str()).unwrap_or("");
    if domain.is_empty() || query.is_empty() {
        return err(400, "domain and query required");
    }
    let top_k = payload.get("top_k").and_then(|t| t.as_i64()).unwrap_or(5) as usize;
    let hits = match crate::pymander::search(&st.h, domain, query, top_k) {
        Ok(h) => h,
        Err(e) => return err(400, &e),
    };
    let results: Vec<serde_json::Value> = hits
        .iter()
        .map(|h| {
            serde_json::json!({
                "label": h.get("label").cloned().unwrap_or(serde_json::json!("")),
                "summary": h.get("summary").and_then(|s| s.as_str()).unwrap_or(""),
                "score": (h.get("_score").and_then(|s| s.as_f64()).unwrap_or(0.0) * 10000.0).round() / 10000.0,
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(serde_json::json!({"ok": true, "domain": domain, "results": results})),
    )
}

async fn pymander_select(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let domains: Vec<String> = payload
        .get("domains")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    match crate::pymander::set_selection(st.h.root(), &pid, &domains) {
        Ok(res) => (
            StatusCode::OK,
            Json(
                serde_json::json!({"ok": true, "project_id": pid, "domains": res.get("domains").cloned().unwrap_or(serde_json::json!([]))}),
            ),
        ),
        Err(e) => err(400, &e),
    }
}

async fn pymander_context(
    State(st): State<ServerState>,
    Json(payload): Json<serde_json::Value>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Some(pid) = project_id(&payload) else {
        return err(400, "project_id required");
    };
    let query = payload.get("query").and_then(|q| q.as_str()).unwrap_or("");
    let budget = payload
        .get("budget_tokens")
        .and_then(|b| b.as_i64())
        .unwrap_or(3000) as usize;
    let top_k = payload.get("top_k").and_then(|t| t.as_i64()).unwrap_or(3) as usize;
    let block = crate::pymander::build_doctrine(
        &st.h,
        st.h.root(),
        &pid,
        &crate::pymander::DoctrineOpts {
            query: query.to_string(),
            budget_tokens: budget,
            top_k,
        },
    );
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok": true, "project_id": pid,
            "tokens": crate::scorer::estimate_tokens(&block),
            "context": block,
        })),
    )
}

fn err(code: u16, msg: &str) -> (StatusCode, Json<serde_json::Value>) {
    let status = StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(serde_json::json!({"error": msg})))
}

/// Start the axum server on `host:port`.
pub async fn serve(host: &str, port: u16, state: ServerState) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind((host, port)).await?;
    axum::serve(listener, router(state)).await
}
