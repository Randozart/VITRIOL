//! HTTP contract tests for the axum server — verify the JSON shapes the plugin
//! and TUI consume. Semantic-off: `/hermetis/embed` returns 503 (Python parity).

use std::sync::Arc;

use libhermes::server::{router, ServerState};
use libhermes::Hermes;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

fn state_at(root: std::path::PathBuf) -> ServerState {
    ServerState {
        h: Arc::new(Hermes::new(&root)),
        last_request: None,
    }
}

async fn call(
    state: &ServerState,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let resp = router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn health_and_store_and_search_flow() {
    let root = std::env::temp_dir().join("hermes_server_test");
    let _ = std::fs::remove_dir_all(&root);
    let st = state_at(root.clone());

    let (s, j) = call(&st, "GET", "/health", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["service"], "hermetis");

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/store",
        Some(serde_json::json!({"project_id": "p", "session_id": "s", "role": "user", "content": "the red fox design works well", "token_count": 8})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["ok"], true);
    let ep_id = j["episode_id"].as_i64().unwrap();

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/node",
        Some(serde_json::json!({"project_id": "p", "label": "fox", "summary": "the red fox is a night hunter", "strength": 0.9})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["ok"], true);

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/search",
        Some(serde_json::json!({"project_id": "p", "query": "red fox", "top_k": 5})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["ok"], true);
    assert_eq!(j["count"].as_i64().unwrap(), 2); // episode + node
    let first = &j["results"][0];
    assert!(first["score"].as_f64().unwrap() > 0.0);
    assert!(first["type"].is_string());
    assert!(first["content"].as_str().unwrap().contains("fox"));
    let _ = ep_id;

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/context",
        Some(
            serde_json::json!({"project_id": "p", "recent_text": "red fox", "budget_tokens": 200}),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["is_new_topic"], true); // semantic-off always new
    assert!(j["context"].as_str().is_some());

    let (s, j) = call(&st, "GET", "/hermetis/recent?project_id=p&limit=5", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(j["recent"].as_array().unwrap().len(), 1);
    assert!(j["recent"][0]["snippet"]
        .as_str()
        .unwrap()
        .contains("red fox"));

    let (s, j) = call(&st, "GET", "/hermetis/stats?project_id=p", None).await;
    assert_eq!(s, StatusCode::OK);
    eprintln!("stats json: {}", j);
    assert_eq!(j["episodes"], 1);
    assert_eq!(j["nodes"], 1);
    assert_eq!(j["sessions"], 1);
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn validation_and_pending_routes() {
    let root = std::env::temp_dir().join("hermes_server_val");
    let _ = std::fs::remove_dir_all(&root);
    let st = state_at(root.clone());

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/store",
        Some(serde_json::json!({"content": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(j["error"].as_str().unwrap().contains("project_id"));

    let (s, _) = call(
        &st,
        "POST",
        "/hermetis/embed",
        Some(serde_json::json!({"text": "x"})),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);

    let (s, j) = call(
        &st,
        "POST",
        "/hermetis/repo_map",
        Some(serde_json::json!({"project_id": "p"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST); // root missing
    assert!(j["error"].as_str().unwrap().contains("root"));

    let (s, j) = call(&st, "GET", "/pymander/list", None).await;
    assert_eq!(s, StatusCode::NOT_IMPLEMENTED);
    assert!(j["error"].as_str().unwrap().contains("P5"));
    let _ = std::fs::remove_dir_all(&root);
}
