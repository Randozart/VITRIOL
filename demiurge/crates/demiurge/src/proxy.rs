// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). M2 proxy: chat-completions goes
// through the arbiter (privacy scan -> decision -> lane dispatch with
// model rewrite + key injection for cloud lanes); everything else streams
// byte-exact to the local lane. x-demiurge-lane / x-demiurge-cause audit
// headers on every response. SSE pattern informed by knooppunt's
// stream_tap design (MIT OR Apache-2.0, same author) - no code copied.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;

use crate::arbiter::Arbiter;
use crate::audit::Audit;
use crate::config::{Lane, LaneKind, Registry};
use crate::pressure::PressureBoard;

/// chat bodies above this stay local unscanned (fail-closed).
const MAX_CHAT_BYTES: usize = 8 * 1024 * 1024;

/// Hop-by-hop + length-managed headers never forwarded (RFC 9110 7.6.1).
const STRIP_REQUEST: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
    "authorization",
];
const STRIP_RESPONSE: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "te",
    "trailer",
    "transfer-encoding",
    "content-length",
];

#[derive(Clone)]
pub struct Proxy {
    pub registry: std::sync::Arc<Registry>,
    pub client: reqwest::Client,
    pub board: std::sync::Arc<PressureBoard>,
    pub arbiter: std::sync::Arc<Arbiter>,
    pub audit: std::sync::Arc<Audit>,
    pub privacy: std::sync::Arc<privacy::PrivacyEngine>,
}

/// URL join per lane kind: local/node keep the client path verbatim;
/// cloud gateways carry their own version root, so a leading /v1 is
/// stripped (zen base ends in /v1, client sends /v1/chat/completions).
fn lane_url(lane: &Lane, path_and_query: &str) -> String {
    let base = lane.endpoint.trim_end_matches('/');
    match lane.kind {
        LaneKind::Local | LaneKind::Node => format!("{base}{path_and_query}"),
        LaneKind::CloudFree | LaneKind::CloudPaid => {
            let stripped = path_and_query.strip_prefix("/v1").unwrap_or(path_and_query);
            format!("{base}{stripped}")
        }
    }
}

fn header_pairs(
    headers: &axum::http::HeaderMap,
) -> impl Iterator<Item = (reqwest::header::HeaderName, reqwest::header::HeaderValue)> + '_ {
    headers.iter().filter_map(|(name, value)| {
        Some((
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()).ok()?,
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()).ok()?,
        ))
    })
}

fn finish_response(sent: reqwest::Response, lane_id: &str, cause: &str) -> Response {
    let axum_status =
        StatusCode::from_u16(sent.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);

    let mut out = Response::builder().status(axum_status);
    for (name, value) in sent.headers().iter() {
        if STRIP_RESPONSE.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            out = out.header(n, v);
        }
    }
    out = out.header("x-demiurge-lane", lane_id);
    out = out.header("x-demiurge-cause", cause);

    let mapped = sent
        .bytes_stream()
        .map(|item| item.map_err(axum::Error::new));

    match out.body(Body::from_stream(mapped)) {
        Ok(resp) => resp,
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("demiurge: response build failed: {e}"),
        )
            .into_response(),
    }
}

/// Streaming passthrough (everything that is not the arbiter path).
pub async fn forward_stream(
    proxy: &Proxy,
    req: Request,
    lane_base: &str,
    lane_id: &str,
) -> Response {
    let (parts, body) = req.into_parts();

    let url = format!(
        "{}{}",
        lane_base.trim_end_matches('/'),
        parts
            .uri
            .path_and_query()
            .map(|pq| pq.as_str())
            .unwrap_or("/")
    );

    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::GET);
    let mut upstream = proxy.client.request(method, &url);

    for (name, value) in header_pairs(&parts.headers) {
        if STRIP_REQUEST.contains(&name.as_str()) {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    upstream = upstream.body(reqwest::Body::wrap_stream(body.into_data_stream()));

    match upstream.send().await {
        Ok(sent) => finish_response(sent, lane_id, "passthrough"),
        Err(e) => {
            tracing::warn!(lane = lane_id, error = %e, "upstream unavailable");
            (
                StatusCode::BAD_GATEWAY,
                format!("demiurge: lane '{lane_id}' unreachable: {e}"),
            )
                .into_response()
        }
    }
}

/// Buffered chat request: privacy scan -> arbiter -> lane dispatch.
pub async fn handle_chat(State(state): State<Proxy>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let session = parts
        .headers
        .get("x-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| "/v1/chat/completions".to_string());

    // buffered read with a cap; oversized bodies stay local unscanned
    let body_bytes = match axum::body::to_bytes(body, MAX_CHAT_BYTES).await {
        Ok(b) => b,
        Err(_) => {
            let local = state.registry.local().clone();
            state.audit.append(&serde_json::json!({
                "ts": now_ms(),
                "session": session,
                "lane": local.id,
                "cause": "oversized-local",
            }));
            return forward_stream(
                &state,
                restore_request(parts, Body::empty()),
                &local.endpoint,
                &local.id,
            )
            .await;
        }
    };

    // Law 1 input: privacy scan (None = failed scan -> fail-closed local)
    let texts = extract_texts(&body_bytes);
    let privacy_class = state.privacy.scan(&texts).ok();

    let local = state.registry.local().clone();
    let local_unhealthy = state
        .board
        .get(&local.id)
        .map(|p| !p.healthy)
        .unwrap_or(false);

    let decision = state
        .arbiter
        .decide(session.as_deref(), privacy_class, local_unhealthy);

    let lane = state
        .registry
        .get(&decision.lane_id)
        .cloned()
        .unwrap_or_else(|| state.registry.local().clone());

    // body + auth for cloud lanes: model rewrite + key injection
    let mut send_bytes = body_bytes.to_vec();
    if matches!(lane.kind, LaneKind::CloudFree | LaneKind::CloudPaid) {
        match prepare_cloud_body(&send_bytes, &lane) {
            Ok(prepared) => send_bytes = prepared,
            Err(e) => {
                tracing::warn!(lane = %lane.id, error = %e, "cloud body prepare failed, staying local");
                state.audit.append(&serde_json::json!({
                    "ts": now_ms(),
                    "session": session,
                    "lane": local.id,
                    "cause": "cloud-prepare-failed",
                }));
                return forward_buffered(
                    &state,
                    &parts,
                    &path_and_query,
                    &send_bytes,
                    &local,
                    &local.id,
                    "cloud-prepare-failed",
                )
                .await;
            }
        }
    }

    state.audit.append(&serde_json::json!({
        "ts": now_ms(),
        "session": session,
        "privacy": privacy_class,
        "lane": lane.id,
        "cause": decision.cause.as_str(),
    }));

    forward_buffered(
        &state,
        &parts,
        &path_and_query,
        &send_bytes,
        &lane,
        &lane.id,
        decision.cause.as_str(),
    )
    .await
}

fn restore_request(parts: axum::http::request::Parts, body: Body) -> Request {
    let mut builder = Request::builder()
        .method(parts.method.clone())
        .uri(parts.uri.clone());
    for (k, v) in parts.headers.iter() {
        builder = builder.header(k, v);
    }
    builder
        .body(body)
        .unwrap_or_else(|_| Request::new(Body::empty()))
}

/// Extract plain text from an OpenAI chat body for the privacy scan.
fn extract_texts(body: &[u8]) -> Vec<String> {
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            match m.get("content") {
                Some(serde_json::Value::String(s)) => out.push(s.clone()),
                Some(serde_json::Value::Array(parts)) => {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                            out.push(t.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Cloud dispatch: force the lane's model, then serialize.
fn prepare_cloud_body(body: &[u8], lane: &Lane) -> Result<Vec<u8>, String> {
    let model = lane
        .models
        .first()
        .ok_or_else(|| "lane declares no models".to_string())?;
    let mut v: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("body not json: {e}"))?;
    v["model"] = serde_json::Value::String(model.to_string());
    serde_json::to_vec(&v).map_err(|e| format!("re-serialize failed: {e}"))
}

#[allow(clippy::too_many_arguments)]
async fn forward_buffered(
    proxy: &Proxy,
    parts: &axum::http::request::Parts,
    path_and_query: &str,
    body_bytes: &[u8],
    lane: &Lane,
    lane_id: &str,
    cause: &str,
) -> Response {
    let url = lane_url(lane, path_and_query);
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes())
        .unwrap_or(reqwest::Method::POST);
    let mut upstream = proxy.client.request(method, &url);

    for (name, value) in header_pairs(&parts.headers) {
        if STRIP_REQUEST.contains(&name.as_str()) {
            continue;
        }
        upstream = upstream.header(name, value);
    }

    // lane key injection (cloud lanes): Authorization replaces the client's
    if let Some(env_name) = &lane.api_key_env {
        if let Ok(key) = std::env::var(env_name) {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
                upstream = upstream.header(reqwest::header::AUTHORIZATION, v);
            }
        }
    }

    upstream = upstream.body(reqwest::Body::from(body_bytes.to_vec()));

    match upstream.send().await {
        Ok(sent) => finish_response(sent, lane_id, cause),
        Err(e) => {
            tracing::warn!(lane = lane_id, error = %e, "upstream unavailable");
            (
                StatusCode::BAD_GATEWAY,
                format!("demiurge: lane '{lane_id}' unreachable: {e}"),
            )
                .into_response()
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Fallback handler: stream everything non-arbiter to the local lane.
pub async fn handle_passthrough(State(state): State<Proxy>, req: Request) -> Response {
    let lane = state.registry.local();
    let base = lane.endpoint.clone();
    let id = lane.id.clone();
    forward_stream(&state, req, &base, &id).await
}

/// GET /demiurge/lanes - registry + live pressure (observability surface).
pub async fn handle_lanes(State(state): State<Proxy>) -> Response {
    let lanes: Vec<serde_json::Value> = state
        .registry
        .lanes
        .iter()
        .map(|lane| {
            serde_json::json!({
                "id": lane.id,
                "kind": lane.kind,
                "endpoint": lane.endpoint,
                "kv": lane.kv,
                "fingerprint": lane.fingerprint,
                "overflow": lane.overflow,
                "pressure": state.board.get(&lane.id),
            })
        })
        .collect();

    let mut resp = axum::Json(serde_json::json!({
        "lanes": lanes,
        "sessions": state.arbiter.sessions.len(),
    }))
    .into_response();
    resp.headers_mut()
        .insert("x-demiurge-lane", HeaderValue::from_static("demiurge"));
    resp
}

/// GET /demiurge/health - daemon liveness.
pub async fn handle_health() -> Response {
    axum::Json(serde_json::json!({ "status": "ok", "daemon": "demiurge" })).into_response()
}
