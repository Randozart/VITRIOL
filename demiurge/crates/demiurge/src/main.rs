// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md). DEMIURGE - VITRIOL Lanes
// arbiter. M1 entry: load the lane registry, start the pressure poller,
// serve the proxy on 127.0.0.1:8265 (DEMIURGE_PORT overrides).

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let registry = match demiurge::config::Registry::load() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("demiurge: {e}");
            eprintln!(
                "  expected lanes file at {} (or DEMIURGE_LANES_FILE)",
                demiurge::config::Registry::default_path().display()
            );
            std::process::exit(1);
        }
    };

    let port: u16 = std::env::var("DEMIURGE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8265);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    if let Err(e) = runtime.block_on(demiurge::app::serve(registry, port)) {
        eprintln!("demiurge: fatal: {e}");
        std::process::exit(1);
    }
}
