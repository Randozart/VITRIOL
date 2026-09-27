// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). Router assembly + serve loop.
// M2: chat-completions via the arbiter; the rest streams to local.

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::affinity::SessionTable;
use crate::arbiter::{Arbiter, Thresholds};
use crate::audit::Audit;
use crate::config::Registry;
use crate::pressure::PressureBoard;
use crate::proxy::Proxy;

pub fn build_router(
    registry: Arc<Registry>,
    board: Arc<PressureBoard>,
    client: reqwest::Client,
    arbiter: Arc<Arbiter>,
    audit: Arc<Audit>,
    privacy: Arc<privacy::PrivacyEngine>,
) -> Router {
    let proxy = Proxy {
        registry,
        client,
        board,
        arbiter,
        audit,
        privacy,
    };

    Router::new()
        .route("/demiurge/health", get(crate::proxy::handle_health))
        .route("/demiurge/lanes", get(crate::proxy::handle_lanes))
        .route("/v1/chat/completions", post(crate::proxy::handle_chat))
        .fallback(crate::proxy::handle_passthrough)
        .with_state(proxy)
}

pub struct Components {
    pub registry: Registry,
    pub board: Arc<PressureBoard>,
    pub client: reqwest::Client,
    pub arbiter: Arc<Arbiter>,
    pub audit: Arc<Audit>,
    pub privacy: Arc<privacy::PrivacyEngine>,
}

pub fn assemble(registry: Registry) -> std::io::Result<Components> {
    let board = PressureBoard::new();
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        // no total timeout: SSE generations run for minutes
        .build()
        .map_err(|e| std::io::Error::other(e.to_string()))?;

    let arbiter = Arc::new(Arbiter {
        registry: registry.clone(),
        board: Arc::clone(&board),
        sessions: SessionTable::new(1800),
        thresholds: Thresholds::from_env(),
    });

    let audit = Arc::new(Audit::new(Audit::default_path()));

    let privacy = Arc::new(
        privacy::PrivacyEngine::new()
            .map_err(|e| std::io::Error::other(format!("privacy engine: {e}")))?,
    );

    Ok(Components {
        registry,
        board,
        client,
        arbiter,
        audit,
        privacy,
    })
}

pub async fn serve(registry: Registry, port: u16) -> std::io::Result<()> {
    let components = assemble(registry)?;

    crate::pressure::spawn_poller(
        Arc::clone(&components.board),
        components.registry.clone(),
        components.client.clone(),
        std::time::Duration::from_secs(2),
    );

    let registry = Arc::new(components.registry);
    let app = build_router(
        Arc::clone(&registry),
        Arc::clone(&components.board),
        components.client,
        Arc::clone(&components.arbiter),
        Arc::clone(&components.audit),
        Arc::clone(&components.privacy),
    );
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
