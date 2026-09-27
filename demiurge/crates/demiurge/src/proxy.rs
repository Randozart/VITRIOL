// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). M1 proxy: byte-exact passthrough
// to the always-preferred (local) lane with x-demiurge-* audit headers.
// SSE streams pass through unmodified (reqwest bytes_stream -> axum body);
// mid-stream upstream drops surface as stream errors - partial data already
// delivered stays delivered. SSE handling pattern informed by knooppunt's
// stream_tap design (MIT OR Apache-2.0, same author) - no code copied.
// Lane SELECTION arrives in M2; M1 pins every request to the local lane.

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;

use crate::config::Registry;
use crate::pressure::PressureBoard;

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
}

/// Proxy `req` to `base`, streaming both directions.
pub async fn forward(proxy: &Proxy, req: Request, lane_base: &str, lane_id: &str) -> Response {
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

    for (name, value) in parts.headers.iter() {
        if STRIP_REQUEST.contains(&name.as_str()) {
            continue;
        }
        if let (Ok(n), Ok(v)) = (
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes()),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            upstream = upstream.header(n, v);
        }
    }

    let reqwest_body = reqwest::Body::wrap_stream(body.into_data_stream());
    upstream = upstream.body(reqwest_body);

    let sent = match upstream.send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(lane = lane_id, error = %e, "upstream unavailable");
            return (
                StatusCode::BAD_GATEWAY,
                format!("demiurge: lane '{lane_id}' unreachable: {e}"),
            )
                .into_response();
        }
    };

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

/// Handle every non-/demiurge route: forward to the preferred lane.
pub async fn handle_passthrough(State(state): State<Proxy>, req: Request) -> Response {
    let lane = state.registry.local();
    let base = lane.endpoint.clone();
    let id = lane.id.clone();
    forward(&state, req, &base, &id).await
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
                "pressure": state.board.get(&lane.id),
            })
        })
        .collect();

    let mut resp = (axum::Json(serde_json::json!({ "lanes": lanes })),).into_response();
    resp.headers_mut()
        .insert("x-demiurge-lane", HeaderValue::from_static("demiurge"));
    resp
}

/// GET /demiurge/health - daemon liveness.
pub async fn handle_health() -> Response {
    axum::Json(serde_json::json!({ "status": "ok", "daemon": "demiurge" })).into_response()
}
