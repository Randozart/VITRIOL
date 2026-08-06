//! Background telemetry poller.
//!
//! A single thread wakes every [`POLL_INTERVAL`](crate::POLL_INTERVAL) (or
//! immediately when the refresh flag is raised) and publishes one
//! [`Snapshot`] per cycle. Each request carries its own timeout, so a hung
//! service degrades to `up: false` and never stalls the UI thread. The decode
//! throughput is parsed from the gen server log because the server's `/health`
//! endpoint only returns `{"status":"ok"}`.

use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::Value;
use ureq::{Agent, AgentBuilder};

use crate::config::Config;
use crate::model::{EmbedSnapshot, GenSnapshot, HermetisSnapshot, LogsSnapshot, Snapshot};
use crate::nvidia;

/// Number of trailing lines kept per service log.
const LOG_TAIL_CAP: usize = 200;

/// Per-poll mutable state: config plus the three incremental log tails.
struct Poller {
    /// Endpoint/log config.
    cfg: Config,
    /// Tail of the gen log.
    gen_tail: LogTail,
    /// Tail of the Hermetis log.
    hermetis_tail: LogTail,
    /// Tail of the embed log.
    embed_tail: LogTail,
}

/// Spawn the poller thread. `refresh_flag` is raised by the UI to force an
/// immediate poll (the `r` key).
pub fn spawn(cfg: Config, tx: Sender<Snapshot>, refresh_flag: Arc<AtomicBool>) {
    thread::Builder::new()
        .name("vitriol-tui-poller".into())
        .spawn(move || {
            let agent = AgentBuilder::new().timeout(Duration::from_secs(3)).build();
            let mut poller = Poller {
                gen_tail: LogTail::new(LOG_TAIL_CAP),
                hermetis_tail: LogTail::new(LOG_TAIL_CAP),
                embed_tail: LogTail::new(LOG_TAIL_CAP),
                cfg,
            };
            loop {
                poller.poll_once(&agent, &tx);
                if refresh_flag.swap(false, Ordering::Relaxed) {
                    continue;
                }
                thread::sleep(crate::POLL_INTERVAL);
            }
        })
        .expect("spawn vitriol-tui poller thread");
}

impl Poller {
    /// Run one full poll cycle and publish the snapshot.
    fn poll_once(&mut self, agent: &Agent, tx: &Sender<Snapshot>) {
        self.gen_tail.poll(&self.cfg.gen_log());
        self.hermetis_tail.poll(&self.cfg.hermetis_log());
        self.embed_tail.poll(&self.cfg.embed_log());
        let snap = Snapshot {
            gen: poll_gen(agent, &self.cfg),
            hermetis: poll_hermetis(agent, &self.cfg),
            embed: poll_embed(agent, &self.cfg),
            gpu: nvidia::query_gpu(),
            logs: LogsSnapshot {
                gen: self.gen_tail.snapshot(),
                hermetis: self.hermetis_tail.snapshot(),
                embed: self.embed_tail.snapshot(),
            },
        };
        let _ = tx.send(snap);
    }
}

/// Poll the gen server: `/health`, `/v1/models`, and the log-derived decode t/s.
fn poll_gen(agent: &Agent, cfg: &Config) -> GenSnapshot {
    let up = get_json(agent, &format!("{}/health", cfg.gen_base()))
        .map(|v| v.get("status").is_some())
        .unwrap_or(false);
    let mut model = None;
    let mut n_ctx = None;
    if let Some(models) = get_json(agent, &format!("{}/v1/models", cfg.gen_base())) {
        let first = models
            .get("data")
            .and_then(|d| d.as_array())
            .and_then(|a| a.first());
        model = first
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        n_ctx = first.and_then(first_model_size);
    }
    GenSnapshot {
        up,
        model,
        n_ctx,
        n_parallel: None,
        decode_t_s: parse_decode_t_s(&cfg.gen_log()),
    }
}

/// Poll the Hermetis server: `/health` and `/hermetis/stats?project_id=`.
fn poll_hermetis(agent: &Agent, cfg: &Config) -> HermetisSnapshot {
    let up = get_json(agent, &format!("{}/health", cfg.hermetis_base()))
        .map(|v| v.get("status").is_some())
        .unwrap_or(false);
    let stats_url = format!(
        "{}/hermetis/stats?project_id={}",
        cfg.hermetis_base(),
        cfg.project_id
    );
    let (episodes, nodes, sessions) = match get_json(agent, &stats_url) {
        Some(stats) => (
            stats.get("episodes").and_then(|v| v.as_u64()),
            stats.get("nodes").and_then(|v| v.as_u64()),
            stats.get("sessions").and_then(|v| v.as_u64()),
        ),
        None => (None, None, None),
    };
    HermetisSnapshot {
        up,
        episodes,
        nodes,
        sessions,
    }
}

/// Poll the embed server's `/health`.
fn poll_embed(agent: &Agent, cfg: &Config) -> EmbedSnapshot {
    let up = get_json(agent, &format!("{}/health", cfg.embed_base()))
        .map(|v| v.get("status").is_some())
        .unwrap_or(false);
    EmbedSnapshot { up }
}

/// GET a JSON body, returning `None` on any transport or parse failure.
fn get_json(agent: &Agent, url: &str) -> Option<Value> {
    let resp = agent.get(url).call().ok()?;
    resp.into_json::<Value>().ok()
}

/// Extract the first model's reported context size, tolerating the differing
/// field names across llama-server versions.
fn first_model_size(model: &Value) -> Option<u64> {
    for key in ["n_ctx", "context_length"] {
        if let Some(v) = model.get(key).and_then(|v| v.as_u64()) {
            return Some(v);
        }
    }
    model
        .get("meta")
        .and_then(|m| m.get("n_ctx"))
        .and_then(|v| v.as_u64())
}

/// Parse the latest decode throughput (tokens/sec) from a llama-server log.
/// The slot timing line has the form
/// `eval time = 1234.56 ms / 42 tokens (29.39 ms per token, 34.02 tokens per second)`.
/// Only the final such line in the file is used, so the value is the most
/// recent completed generation. Returns 0.0 when nothing parseable exists.
fn parse_decode_t_s(log_path: &std::path::Path) -> f64 {
    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return 0.0,
    };
    content
        .lines()
        .filter_map(|line| {
            // The decode timing is the line beginning `eval time =`; the prompt
            // line begins `prompt eval time =` and must not be counted.
            if !line.trim_start().starts_with("eval time =") {
                return None;
            }
            let marker = " tokens per second)";
            let idx = line.find(marker)?;
            let before = &line[..idx];
            let num = before.split_whitespace().last()?;
            num.trim_end_matches(',')
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
        })
        .next_back()
        .unwrap_or(0.0)
}

/// Incremental reader for a service log.
///
/// On each poll only the bytes appended since the previous read are pulled and
/// pushed into a capped ring of trailing lines, so a long-lived log never gets
/// re-read from the top. A file shrink (truncation/rotation) resets the read
/// position so the new content is picked up cleanly.
struct LogTail {
    /// Byte offset of the next unread byte.
    pos: u64,
    /// Trailing lines, oldest first, capped at [`LOG_TAIL_CAP`].
    tail: VecDeque<String>,
    /// Maximum number of retained lines.
    cap: usize,
}

impl LogTail {
    /// Create an empty tail with the given line cap.
    fn new(cap: usize) -> Self {
        Self {
            pos: 0,
            tail: VecDeque::with_capacity(cap),
            cap,
        }
    }

    /// Pull any new bytes from `path` into the trailing-line ring.
    fn poll(&mut self, path: &std::path::Path) {
        let Ok(meta) = fs::metadata(path) else {
            return;
        };
        let len = meta.len();
        if len < self.pos {
            self.pos = 0;
        }
        if len == self.pos {
            return;
        }
        let Ok(mut f) = fs::File::open(path) else {
            return;
        };
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return;
        }
        let mut buf = vec![0u8; (len - self.pos) as usize];
        if f.read_exact(&mut buf).is_err() {
            return;
        }
        self.pos = len;
        for line in String::from_utf8_lossy(&buf).lines() {
            if self.tail.len() == self.cap {
                self.tail.pop_front();
            }
            self.tail.push_back(line.to_string());
        }
    }

    /// A copy of the retained trailing lines, oldest first.
    fn snapshot(&self) -> Vec<String> {
        self.tail.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a temp log file and return its path. `tag` keeps parallel tests
    /// from colliding on the same file.
    fn temp_log(tag: &str, content: &str) -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("vitriol-tui-test-{}-{tag}.log", std::process::id()));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parse_decode_single_line() {
        let p = temp_log("single", "       eval time = 1234.56 ms / 42 tokens (29.39 ms per token, 34.02 tokens per second)\n");
        assert_eq!(parse_decode_t_s(&p), 34.02);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_decode_takes_last_line() {
        let content = [
            "       eval time = 100.00 ms / 10 tokens (10.00 ms per token, 100.00 tokens per second)",
            "prompt eval time = 5.00 ms / 10 tokens (0.50 ms per token, 2000.00 tokens per second)",
            "total time = 105.00 ms / 20 tokens",
            "eval time = 50.00 ms / 10 tokens (5.00 ms per token, 200.00 tokens per second)",
        ]
        .join("\n")
            + "\n";
        let p = temp_log("last", &content);
        assert_eq!(parse_decode_t_s(&p), 200.00);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_decode_skips_prompt_eval_and_absent() {
        let content = [
            "prompt eval time = 5.00 ms / 10 tokens (0.50 ms per token, 2000.00 tokens per second)",
            "total time = 5.00 ms / 10 tokens",
        ]
        .join("\n")
            + "\n";
        let p = temp_log("absent", &content);
        assert_eq!(parse_decode_t_s(&p), 0.0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn parse_decode_missing_log() {
        assert_eq!(
            parse_decode_t_s(&std::path::PathBuf::from("/nonexistent/log")),
            0.0
        );
    }

    #[test]
    fn model_size_accepts_variant_fields() {
        let n_ctx = serde_json::json!({"id": "m", "n_ctx": 32768});
        assert_eq!(first_model_size(&n_ctx), Some(32768));
        let context_length = serde_json::json!({"id": "m", "context_length": 4096});
        assert_eq!(first_model_size(&context_length), Some(4096));
        let in_meta = serde_json::json!({"id": "m", "meta": {"n_ctx": 8192}});
        assert_eq!(first_model_size(&in_meta), Some(8192));
        let none = serde_json::json!({"id": "m"});
        assert_eq!(first_model_size(&none), None);
    }

    #[test]
    fn log_tail_reads_incrementally_and_caps() {
        let mut path = std::env::temp_dir();
        path.push(format!("vitriol-tui-tail-{}.log", std::process::id()));
        let mut tail = LogTail::new(2);
        std::fs::write(&path, "a\nb\n").unwrap();
        tail.poll(&path);
        assert_eq!(tail.snapshot(), vec!["a", "b"]);
        // Second poll only pulls the appended bytes, keeping the ring capped.
        std::fs::write(&path, "a\nb\nc\nd\n").unwrap();
        tail.poll(&path);
        assert_eq!(tail.snapshot(), vec!["c", "d"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn log_tail_resets_on_truncation() {
        let mut path = std::env::temp_dir();
        path.push(format!("vitriol-tui-trunc-{}.log", std::process::id()));
        let mut tail = LogTail::new(10);
        std::fs::write(&path, "old line one\nold line two\n").unwrap();
        tail.poll(&path);
        std::fs::write(&path, "fresh line\n").unwrap();
        tail.poll(&path);
        assert_eq!(
            tail.snapshot(),
            vec!["old line one", "old line two", "fresh line"]
        );
        let _ = std::fs::remove_file(&path);
    }
}
