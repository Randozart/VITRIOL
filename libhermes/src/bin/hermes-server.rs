//! The `hermes` HTTP server binary — the Rust replacement for
//! `hermetis_server.py`. Serves `/health`, `/hermetis/*`, and (once P4/P5 land)
//! `/pymander/*` with the same JSON contracts.

use std::sync::Arc;

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
    let state = ServerState {
        h: Arc::new(Hermes::new(&Hermes::default_root())),
    };
    println!("hermes: serving on http://{host}:{port}");
    libhermes::server::serve(&host, port, state).await
}
