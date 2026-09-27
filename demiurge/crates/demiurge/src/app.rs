// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). Application state + router
// assembly. M1: /demiurge/{health,lanes} + fallback passthrough to the
// local lane. Decision layer (affinity, hysteresis, privacy) lands in M2.

use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::config::Registry;
use crate::pressure::PressureBoard;
use crate::proxy::Proxy;

pub fn build_router(
    registry: Arc<Registry>,
    board: Arc<PressureBoard>,
    client: reqwest::Client,
) -> Router {
    let proxy = Proxy {
        registry,
        client,
        board,
    };

    Router::new()
        .route("/demiurge/health", get(crate::proxy::handle_health))
        .route("/demiurge/lanes", get(crate::proxy::handle_lanes))
        .fallback(crate::proxy::handle_passthrough)
        .with_state(proxy)
}

pub async fn serve(registry: Registry, port: u16) -> std::io::Result<()> {
    let board = PressureBoard::new();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        // no total timeout: SSE generations run for minutes
        .build()
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    crate::pressure::spawn_poller(
        Arc::clone(&board),
        registry.clone(),
        client.clone(),
        std::time::Duration::from_secs(2),
    );

    let registry = Arc::new(registry);
    let app = build_router(Arc::clone(&registry), Arc::clone(&board), client);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    tracing::info!(
        "demiurge listening on {addr} (lanes: {})",
        registry.lanes.len()
    );

    axum::serve(listener, app)
        .await
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    Ok(())
}
