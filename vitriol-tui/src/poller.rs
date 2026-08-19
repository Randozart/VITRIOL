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
use crate::model::{
    EmbedSnapshot, GenSnapshot, HermetisSnapshot, LogsSnapshot, PerfSnapshot, RecentStore, Snapshot,
};
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
    /// Byte offset of the newest `decode heartbeat` line seen last poll, or
    /// None before the first sighting. Lets the poller tell "still generating"
    /// (offset advanced) from "went idle" (offset unchanged since the previous
    /// poll).
    decode_beat_offset: Option<u64>,
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
                decode_beat_offset: None,
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
            gen: self.poll_gen(agent),
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

    /// Poll the gen server: `/health`, `/v1/models`, and the log-derived decode
    /// state (sticky last-completion throughput plus the live heartbeat rate).
    fn poll_gen(&mut self, agent: &Agent) -> GenSnapshot {
        let up = get_json(agent, &format!("{}/health", self.cfg.gen_base()))
            .map(|v| v.get("status").is_some())
            .unwrap_or(false);
        let mut model = None;
        let mut n_ctx = None;
        if let Some(models) = get_json(agent, &format!("{}/v1/models", self.cfg.gen_base())) {
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
            decode_t_s: parse_decode_t_s(&self.cfg.gen_log()),
            decode_speed: parse_decode_speed(&self.cfg.gen_log(), &mut self.decode_beat_offset),
            perf: parse_perf(&self.cfg.gen_log()),
        }
    }
}

/// Poll the Hermetis server: `/health`, `/hermetis/stats?project_id=`, and
/// `/hermetis/recent?project_id=` for the most recent stores.
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
    let recent_url = format!(
        "{}/hermetis/recent?project_id={}&limit=5",
        cfg.hermetis_base(),
        cfg.project_id
    );
    let recent = get_json(agent, &recent_url)
        .and_then(|r| r.get("recent").cloned())
        .and_then(|r| r.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|it| {
                    Some(RecentStore {
                        id: it.get("id")?.as_i64()?,
                        role: it.get("role")?.as_str()?.to_string(),
                        snippet: it.get("snippet")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    HermetisSnapshot {
        up,
        episodes,
        nodes,
        sessions,
        recent,
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

/// Parse the live decode heartbeat from the gen log and report whether a slot
/// is still generating.
///
/// The VITRIOL server writes one `decode heartbeat: N tokens, X.XX tokens per
/// second (live)` line per second while any slot is decoding. The newest such
/// line is parsed for its throughput; freshness is derived by comparing the
/// byte offset of that line to the offset seen on the previous poll — the line
/// must have moved forward since, or the slot went idle. Returns 0.0 when no
/// beat arrived since the last poll (including when the server predates the
/// heartbeat).
fn parse_decode_speed(log_path: &std::path::Path, last_offset: &mut Option<u64>) -> f64 {
    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return 0.0,
    };
    let marker = " tokens per second (live)";
    let mut beat_offset = 0u64;
    let mut t_s = 0.0f64;
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        if let Some(pos) = line.find(marker) {
            beat_offset = offset as u64;
            let before = &line[..pos];
            let num = before.split_whitespace().last().unwrap_or("");
            if let Ok(v) = num.trim_end_matches(',').parse::<f64>() {
                if v.is_finite() && v > 0.0 {
                    t_s = v;
                }
            }
        }
        offset += line.len();
    }
    if t_s <= 0.0 {
        return 0.0;
    }
    match *last_offset {
        // First beat ever seen this session: always fresh.
        None => {
            *last_offset = Some(beat_offset);
            t_s
        }
        Some(prev) => {
            // Log rotation/truncation shrinks the file: accept the beat even if
            // its absolute offset is small.
            if content.len() < prev as usize {
                *last_offset = Some(beat_offset);
                return t_s;
            }
            // A beat moved forward inside an un-rotated log: still generating.
            if beat_offset > prev {
                *last_offset = Some(beat_offset);
                return t_s;
            }
            0.0
        }
    }
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

/// Parse the newest VITRIOL `[PERF]` decode-breakdown line from the gen log.
/// The server writes one such line per outermost `llama_decode` when launched
/// with `GGML_CUDA_GDN_PROFILE=1`. Format:
///   `[PERF] total=97.5ms build=12.0ms compute=80.0ms post=5.5ms
///    graph=1C/57R sync=2(2.0ms) top_ops=FFN=64...`
/// Returns `None` when no parseable line exists (server not in perf mode).
fn parse_perf(log_path: &std::path::Path) -> Option<PerfSnapshot> {
    let content = match std::fs::read_to_string(log_path) {
        Ok(c) => c,
        Err(_) => return None,
    };
    content.lines().filter_map(parse_perf_line).next_back()
}

/// Parse a single `[PERF]` line into a [`PerfSnapshot`].
fn parse_perf_line(line: &str) -> Option<PerfSnapshot> {
    if !line.contains("[PERF] ") {
        return None;
    }
    let mut p = PerfSnapshot::default();
    // numbers: total, build, compute, post (ms, same order as server emits)
    let nums: Vec<f64> = ["total=", "build=", "compute=", "post="]
        .iter()
        .filter_map(|k| {
            let i = line.find(k)?;
            let rest = &line[i + k.len()..];
            let n: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
            n.parse::<f64>().ok()
        })
        .collect();
    if nums.len() == 4 {
        p.total_ms = nums[0];
        p.build_ms = nums[1];
        p.compute_ms = nums[2];
        p.post_ms = nums[3];
    }
    // graph=NC/MR
    if let Some(i) = line.find("graph=") {
        let rest = &line[i + 6..];
        let mut cap = String::new();
        for c in rest.chars() {
            if c.is_ascii_digit() { cap.push(c); } else { break; }
        }
        p.n_capture = cap.parse().unwrap_or(0);
        if let Some(c_idx) = rest.find('C') {
            // tail like `/57R top_ops=...` — scan digits skipping separators.
            let mut rep = String::new();
            for c in rest[c_idx + 1..].chars() {
                if c.is_ascii_digit() {
                    rep.push(c);
                } else if !rep.is_empty() && c == 'R' {
                    break;
                }
            }
            p.n_replay = rep.parse().unwrap_or(0);
        }
    }
    // sync=N(ms)
    if let Some(i) = line.find("sync=") {
        let rest = &line[i + 5..];
        let mut n = String::new();
        for c in rest.chars() {
            if c.is_ascii_digit() { n.push(c); } else { break; }
        }
        p.n_sync = n.parse().unwrap_or(0);
        if let Some(j) = rest.find('(') {
            let tail = &rest[j + 1..];
            let mut m = String::new();
            for c in tail.chars() {
                if c.is_ascii_digit() || c == '.' { m.push(c); } else { break; }
            }
            p.sync_ms = m.parse().unwrap_or(0.0);
        }
    }
    // top_ops=NAME=N NAME=N ...
    if let Some(i) = line.find("top_ops=") {
        let rest = &line[i + 8..];
        for tok in rest.split_whitespace() {
            let mut it = tok.splitn(2, '=');
            if let (Some(name), Some(cnt)) = (it.next(), it.next()) {
                if let Ok(c) = cnt.parse::<u64>() {
                    p.top_ops.push((name.to_string(), c));
                }
            }
        }
    }
    Some(p)
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
    fn decode_speed_live_only_when_new_beat() {
        let p = temp_log(
            "beat1",
            "       eval time = 1000.00 ms / 42 tokens (23.81 ms per token, 42.00 tokens per second)\n"
        );
        let mut last = None;
        // No heartbeat line yet: nothing decoding.
        assert_eq!(parse_decode_speed(&p, &mut last), 0.0);
        std::fs::write(&p, [
            "slot test: id 0 | task 1 | ...\n",
            "decode heartbeat: 17 tokens,  23.40 tokens per second (live)\n",
        ].join("")).unwrap();
        // First appearance is fresh → 23.40.
        assert_eq!(parse_decode_speed(&p, &mut last), 23.40);
        assert!(last.is_some());
        // No new bytes appended → idle again.
        assert_eq!(parse_decode_speed(&p, &mut last), 0.0);
        // A second, later beat advances past the first.
        std::fs::write(&p, [
            "slot test: id 0 | task 1 | ...\n",
            "decode heartbeat: 17 tokens,  23.40 tokens per second (live)\n",
            "decode heartbeat: 42 tokens,  40.12 tokens per second (live)\n",
        ].join("")).unwrap();
        assert_eq!(parse_decode_speed(&p, &mut last), 40.12);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn perf_line_parses_full_breakdown() {
        let line = "[PERF] total=97.5ms build=12.0ms compute=80.2ms post=5.3ms \
            graph=1C/57R sync=2(2.0ms) top_ops=FFN=64 attn=32 MUL_MAT=16";
        let p = parse_perf_line(line).expect("should parse");
        assert_eq!(p.total_ms, 97.5);
        assert_eq!(p.build_ms, 12.0);
        assert_eq!(p.compute_ms, 80.2);
        assert_eq!(p.post_ms, 5.3);
        assert_eq!(p.n_capture, 1);
        assert_eq!(p.n_replay, 57);
        assert_eq!(p.n_sync, 2);
        assert_eq!(p.sync_ms, 2.0);
        assert_eq!(
            p.top_ops,
            vec![
                ("FFN".to_string(), 64),
                ("attn".to_string(), 32),
                ("MUL_MAT".to_string(), 16),
            ]
        );
    }

    #[test]
    fn perf_line_absent_when_not_perf_mode() {
        assert!(parse_perf_line("slot test: id 0 | task 1 | ...").is_none());
        assert!(parse_perf_line("decode heartbeat: 17 tokens,  23.40 t/s (live)").is_none());
    }

    #[test]
    fn decode_speed_resets_on_rotation() {
        let filler = "x".repeat(200);
        let p = temp_log(
            "beatrot",
            &format!("{filler}\ndecode heartbeat: 10 tokens,  30.00 tokens per second (live)\n"),
        );
        let mut last = None;
        assert_eq!(parse_decode_speed(&p, &mut last), 30.00);
        // Log rotated to a shorter file (no filler) with a fresh beat.
        std::fs::write(&p, "decode heartbeat: 5 tokens,  25.00 tokens per second (live)\n").unwrap();
        assert_eq!(parse_decode_speed(&p, &mut last), 25.00);
        let _ = std::fs::remove_file(&p);
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
