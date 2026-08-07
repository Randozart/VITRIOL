//! The `hermes` HTTP server binary — the Rust replacement for
//! `hermetis_server.py`. Serves `/health`, `/hermetis/*`, and (once P5 lands)
//! `/pymander/*` with the same JSON contracts. Runs the idle-triggered memory
//! consolidation loop in the background; every request marks the worker active.

use std::sync::Arc;

use libhermes::consolidate::ConsolidationWorker;
use libhermes::{Hermes, ServerState};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 7980;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                if let Some(v) = args.next() {
                    host = v;
                }
            }
            "--port" => {
                if let Some(v) = args.next() {
                    port = v.parse().unwrap_or(7980);
                }
            }
            _ => {}
        }
    }
    let h = Arc::new(Hermes::new(&Hermes::default_root()));
    let worker = ConsolidationWorker::new();
    let ticker = worker.ticker();
    tokio::spawn(worker.run_loop(h.clone()));
    let state = ServerState {
        h: h.clone(),
        last_request: Some(ticker),
    };
    println!("hermes: serving on http://{host}:{port}");
    libhermes::server::serve(&host, port, state).await
}
