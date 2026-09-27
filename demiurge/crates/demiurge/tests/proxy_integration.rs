// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). M2 integration tests: arbiter
// routing end-to-end over in-process stubs - local headroom, pressure
// overflow to the free lane (model rewrite + key injection), privacy gate
// (sensitive stays local even saturated), affinity stickiness, audit
// headers.

use std::sync::Arc;

use axum::body::Body;
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;

use demiurge::affinity::SessionTable;
use demiurge::arbiter::{Arbiter, Thresholds};
use demiurge::audit::Audit;
use demiurge::config::Registry;
use demiurge::pressure::{Pressure, PressureBoard};

/// env vars are process-global and the suite runs parallel - tests that
/// set/remove the zen key must hold this lock for their whole body.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const LANES_PAID: &str = r#"{
    "lanes": [
        { "id": "lapis", "kind": "local", "endpoint": "LOCAL", "always_preferred": true },
        { "id": "ascensus", "kind": "cloud-paid", "endpoint": "http://ignored.example", "models": ["glm-5.3-flash"], "overflow": true, "gateway": "GATEWAY", "budget": "ascensusd" }
    ]
}"#;

const LANES: &str = r#"{
    "lanes": [
        { "id": "lapis", "kind": "local", "endpoint": "PLACEHOLDER_LOCAL", "always_preferred": true },
        { "id": "zen", "kind": "cloud-free", "endpoint": "PLACEHOLDER_ZEN", "models": ["space-bunny-free"], "overflow": true, "api_key_env": "DEMIURGE_TEST_ZEN_KEY" }
    ]
}"#;

async fn spawn_stub(marker: &'static str) -> String {
    let handler = |marker: &'static str| {
        move |headers: axum::http::HeaderMap, body: String| async move {
            let auth = headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("none")
                .to_string();
            axum::Json(serde_json::json!({
                "stub": true,
                "auth": auth,
                "got": serde_json::from_str::<serde_json::Value>(&body).ok(),
                "marker": marker,
            }))
        }
    };
    let app = Router::new()
        .route("/v1/chat/completions", post(handler(marker)))
        .route(
            // cloud gateways carry their own /v1 root - demeiurge strips the
            // client's /v1 prefix, so the stub serves the stripped path too
            "/chat/completions",
            post(handler(marker)),
        )
        .route(
            "/health",
            get(|| async { axum::Json(serde_json::json!({"status":"ok"})) }),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

struct Rig {
    app: Router,
    board: Arc<PressureBoard>,
}

async fn rig() -> Rig {
    let local = spawn_stub("local-answer").await;
    let zen = spawn_stub("zen-answer").await;

    let json = LANES
        .replace("PLACEHOLDER_LOCAL", &local)
        .replace("PLACEHOLDER_ZEN", &zen);
    let registry = Arc::new(Registry::parse(&json).unwrap());
    let board = PressureBoard::new();

    let arbiter = Arc::new(Arbiter {
        registry: (*registry).clone(),
        board: Arc::clone(&board),
        sessions: SessionTable::new(1800),
        thresholds: Thresholds::default(),
    });
    let audit = Arc::new(Audit::new(
        std::env::temp_dir().join("demiurge-test-decisions.jsonl"),
    ));
    let privacy = Arc::new(privacy::PrivacyEngine::new().unwrap());

    let app = demiurge::app::build_router(
        Arc::clone(&registry),
        Arc::clone(&board),
        reqwest::Client::new(),
        arbiter,
        audit,
        privacy,
    );

    Rig { app, board }
}

impl Rig {
    fn set_pressure(&self, busy: u32, total: u32) {
        self.board.set(
            "lapis",
            Pressure {
                busy_slots: busy,
                total_slots: total,
                deferred: 0,
                tokens_per_second: None,
                healthy: true,
                updated_at_ms: 0,
            },
        );
    }

    async fn chat(&self, session: Option<&str>, body: &str) -> axum::response::Response {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json");
        if let Some(s) = session {
            builder = builder.header("x-session-id", s);
        }
        self.app
            .clone()
            .oneshot(builder.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap()
    }
}

fn chat_body(text: &str) -> String {
    serde_json::json!({
        "model": "whatever-local",
        "messages": [{"role": "user", "content": text}]
    })
    .to_string()
}

#[tokio::test]
async fn quiet_engine_answers_locally() {
    let r = rig().await;
    r.set_pressure(0, 2);
    let res = r.chat(Some("ses_1"), &chat_body("hi")).await;
    assert_eq!(res.headers().get("x-demiurge-lane").unwrap(), "lapis");
    assert_eq!(
        res.headers().get("x-demiurge-cause").unwrap(),
        "local-headroom"
    );
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["marker"], "local-answer");
    assert_eq!(
        v["got"]["model"], "whatever-local",
        "local lane keeps client model"
    );
}

#[tokio::test]
async fn saturated_engine_overflows_to_free_lane_with_model_rewrite() {
    let r = rig().await;
    r.set_pressure(2, 2);
    let res = r.chat(Some("ses_1"), &chat_body("hi")).await;
    let (parts, body) = res.into_parts();
    eprintln!("STATUS: {}", parts.status);
    eprintln!("HEADERS: {:?}", parts.headers);
    let bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    eprintln!("BODY: {}", String::from_utf8_lossy(&bytes));
    assert_eq!(parts.headers.get("x-demiurge-lane").unwrap(), "zen");
    assert_eq!(parts.headers.get("x-demiurge-cause").unwrap(), "overflow");
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["marker"], "zen-answer");
    assert_eq!(
        v["got"]["model"], "space-bunny-free",
        "cloud dispatch rewrites the model to the lane's declared model"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // test-only: the key must outlive the awaits
async fn overflow_injects_lane_key() {
    let _env = ENV_LOCK.lock().unwrap();
    std::env::set_var("DEMIURGE_TEST_ZEN_KEY", "test-key-123");
    let r = rig().await;
    r.set_pressure(2, 2);
    let res = r.chat(Some("ses_1"), &chat_body("hi")).await;
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    eprintln!("KEYTEST BODY: {v}");
    assert_eq!(v["auth"], "Bearer test-key-123");
    std::env::remove_var("DEMIURGE_TEST_ZEN_KEY");
}

#[tokio::test]
async fn sensitive_stays_local_even_saturated() {
    // IBAN + credential context from the Dutch fail-closed category set
    let body = chat_body("betaling naar NL91ABNA0417164300, wachtwoord: hunter2");
    let r = rig().await;
    r.set_pressure(2, 2);
    let res = r.chat(Some("ses_1"), &body).await;
    let lane = res.headers().get("x-demiurge-lane").unwrap().clone();
    let cause = res.headers().get("x-demiurge-cause").unwrap().clone();
    // fail-closed: scanned-and-gated OR scan-refused, either way local
    assert_eq!(lane, "lapis", "sensitive request must not leave local");
    assert_eq!(cause, "privacy-gate");
}

#[tokio::test]
async fn affinity_sticks_then_recalls() {
    let r = rig().await;
    r.set_pressure(2, 2);
    let res = r.chat(Some("ses_stick"), &chat_body("hi")).await;
    assert_eq!(res.headers().get("x-demiurge-lane").unwrap(), "zen");

    // still saturated: affinity holds
    let res = r.chat(Some("ses_stick"), &chat_body("hi")).await;
    assert_eq!(res.headers().get("x-demiurge-cause").unwrap(), "affinity");

    // pressure drops: session recalls home
    r.set_pressure(0, 2);
    let res = r.chat(Some("ses_stick"), &chat_body("hi")).await;
    assert_eq!(res.headers().get("x-demiurge-cause").unwrap(), "recall");
    assert_eq!(res.headers().get("x-demiurge-lane").unwrap(), "lapis");
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // test-only: the key must outlive the awaits
async fn privacy_gate_beats_affinity_pin() {
    let _env = ENV_LOCK.lock().unwrap();
    std::env::set_var("DEMIURGE_TEST_ZEN_KEY", "k");
    let r = rig().await;
    r.set_pressure(2, 2);
    // safe request pins the session to zen
    let res = r.chat(Some("ses_pin"), &chat_body("hi")).await;
    let (parts, _) = res.into_parts();
    assert_eq!(parts.headers.get("x-demiurge-lane").unwrap(), "zen");
    // sensitive follow-up must NOT ride the pin
    let res = r
        .chat(
            Some("ses_pin"),
            &chat_body("betaling naar NL91ABNA0417164300"),
        )
        .await;
    let (parts, _) = res.into_parts();
    assert_eq!(parts.headers.get("x-demiurge-lane").unwrap(), "lapis");
    assert_eq!(
        parts.headers.get("x-demiurge-cause").unwrap(),
        "privacy-gate"
    );
    std::env::remove_var("DEMIURGE_TEST_ZEN_KEY");
}

async fn spawn_gateway(escalate_status: u16, body: serde_json::Value) -> String {
    let app = Router::new().route(
        "/escalate",
        post(move || async move {
            (
                axum::http::StatusCode::from_u16(escalate_status).unwrap(),
                axum::Json(body.clone()),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

async fn paid_rig(
    gateway_status: u16,
    gateway_body: serde_json::Value,
) -> (Router, Arc<PressureBoard>) {
    let local = spawn_stub("local-answer").await;
    let gateway = spawn_gateway(gateway_status, gateway_body).await;

    let json = LANES_PAID
        .replace("LOCAL", &local)
        .replace("GATEWAY", &gateway);
    let registry = Arc::new(Registry::parse(&json).unwrap());
    let board = PressureBoard::new();

    let arbiter = Arc::new(Arbiter {
        registry: (*registry).clone(),
        board: Arc::clone(&board),
        sessions: SessionTable::new(1800),
        thresholds: Thresholds::default(),
    });
    let audit = Arc::new(Audit::new(
        std::env::temp_dir().join("demiurge-test-paid.jsonl"),
    ));
    let privacy = Arc::new(privacy::PrivacyEngine::new().unwrap());

    let app = demiurge::app::build_router(
        registry,
        board.clone(),
        reqwest::Client::new(),
        arbiter,
        audit,
        privacy,
    );
    (app, board)
}

#[tokio::test]
async fn paid_overflow_escalates_via_gateway() {
    let (app, board) = paid_rig(
        200,
        serde_json::json!({"status":"escalated","answer":"the cloud verdict","eur_spent":0.012}),
    )
    .await;
    board.set(
        "lapis",
        Pressure {
            busy_slots: 2,
            total_slots: 2,
            deferred: 0,
            tokens_per_second: None,
            healthy: true,
            updated_at_ms: 0,
        },
    );
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("x-session-id", "ses_paid")
                .body(Body::from(chat_body("hard problem please")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.headers().get("x-demiurge-lane").unwrap(), "ascensus");
    assert_eq!(
        res.headers().get("x-demiurge-cause").unwrap(),
        "overflow-escalation"
    );
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["choices"][0]["message"]["content"], "the cloud verdict");
    assert_eq!(v["object"], "chat.completion");
    assert_eq!(v["demiurge"]["eur_spent"], 0.012);
}

#[tokio::test]
async fn paid_overflow_budget_refusal_is_402() {
    let (app, board) = paid_rig(
        429,
        serde_json::json!({"status":"budget_exhausted","error":{"message":"euro budget exhausted. Spent EUR 1.0000 of 1.0000"}}),
    )
    .await;
    board.set(
        "lapis",
        Pressure {
            busy_slots: 2,
            total_slots: 2,
            deferred: 0,
            tokens_per_second: None,
            healthy: true,
            updated_at_ms: 0,
        },
    );
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(chat_body("hard problem")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 402);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(v["error"]["type"], "budget_exhausted");
}

#[tokio::test]
async fn health_and_lanes_still_served() {
    let r = rig().await;
    let res = r
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/demiurge/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), 200);

    let res = r
        .app
        .oneshot(
            Request::builder()
                .uri("/demiurge/lanes")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("zen") && text.contains("overflow"));
}
