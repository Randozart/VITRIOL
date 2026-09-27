// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). Integration tests: M1 proxy
// round-trip against an in-process stub upstream - body forwarding,
// streaming (SSE) passthrough, x-demiurge-lane audit header, hop-by-hop
// stripping, /demiurge/* routes.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use demiurge::config::Registry;
use demiurge::pressure::PressureBoard;
use demiurge::proxy::Proxy;

const LANES_JSON: &str = r#"{
    "lanes": [
        { "id": "stub", "kind": "local", "endpoint": "PLACEHOLDER", "always_preferred": true },
        { "id": "boxb", "kind": "node", "endpoint": "http://127.0.0.1:1", "kv": true }
    ]
}"#;

async fn spawn_stub() -> String {
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(|headers: axum::http::HeaderMap, body: String| async move {
                let passthrough = headers
                    .get("x-custom-thing")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();
                // echo proves the body arrived intact and custom headers pass
                format!("echo:{passthrough}:{body}")
            }),
        )
        .route(
            "/v1/events",
            get(|| async {
                axum::response::Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from("data: one\n\ndata: two\n\n"))
                    .unwrap()
            }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

fn proxy_for(base: &str) -> Proxy {
    let json = LANES_JSON.replace("PLACEHOLDER", base);
    let registry = Arc::new(Registry::parse(&json).expect("fixture registry"));
    Proxy {
        registry,
        client: reqwest::Client::new(),
        board: PressureBoard::new(),
    }
}

#[tokio::test]
async fn forwards_body_and_custom_header() {
    let base = spawn_stub().await;
    let state = proxy_for(&base);
    let app = Router::new()
        .fallback(demiurge::proxy::handle_passthrough)
        .with_state(state);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("x-custom-thing", "kept")
        .header("connection", "close")
        .body(Body::from("hello-lane"))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers().get("x-demiurge-lane").unwrap(),
        "stub",
        "audit header names the lane"
    );
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"echo:kept:hello-lane");
}

#[tokio::test]
async fn streams_sse_unmodified() {
    let base = spawn_stub().await;
    let state = proxy_for(&base);
    let app = Router::new()
        .fallback(demiurge::proxy::handle_passthrough)
        .with_state(state);

    let req = Request::builder()
        .uri("/v1/events")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(
        res.headers().get("content-type").unwrap(),
        "text/event-stream",
        "content type preserved for SSE clients"
    );
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&bytes[..], b"data: one\n\ndata: two\n\n");
}

#[tokio::test]
async fn unreachable_lane_is_502_with_cause() {
    // port 1 is reserved and refuses connections
    let state = proxy_for("http://127.0.0.1:1");
    let app = Router::new()
        .fallback(demiurge::proxy::handle_passthrough)
        .with_state(state);

    let req = Request::builder()
        .uri("/v1/chat/completions")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), 502);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("unreachable"));
}

#[tokio::test]
async fn demiurge_routes_bypass_proxy() {
    let base = spawn_stub().await;
    let json = LANES_JSON.replace("PLACEHOLDER", &base);
    let registry = Arc::new(Registry::parse(&json).unwrap());
    let board = PressureBoard::new();
    let app = demiurge::app::build_router(registry, board, reqwest::Client::new());

    let req = Request::builder()
        .uri("/demiurge/health")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), 200);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("\"demiurge\""));
}

#[tokio::test]
async fn lanes_endpoint_reports_pressure() {
    let base = spawn_stub().await;
    let json = LANES_JSON.replace("PLACEHOLDER", &base);
    let registry = Arc::new(Registry::parse(&json).unwrap());
    let board = PressureBoard::new();
    let slots = r#"[{"id":0,"is_processing":true},{"id":1,"is_processing":false}]"#;
    board.set(
        "stub",
        demiurge::pressure::snapshot_from(slots, "llamacpp:requests_deferred 2").unwrap(),
    );

    let app = demiurge::app::build_router(registry, board, reqwest::Client::new());
    let req = Request::builder()
        .uri("/demiurge/lanes")
        .body(Body::empty())
        .unwrap();
    let res = app.oneshot(req).await.unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("\"busy_slots\":1"),
        "pressure visible: {text}"
    );
    assert!(text.contains("boxb"));
}
