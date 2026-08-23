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
    DraftSnapshot, EmbedSnapshot, GenSnapshot, HermetisSnapshot, LogsSnapshot, MetricsTotals,
    PerfSnapshot, RebisEvent, RebisSnapshot, RecentStore, SlotSnapshot, Snapshot,
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
    luna_beat_offset: Option<u64>,
    last_gpus: Vec<crate::model::GpuSnapshot>,
    /// Tail of the Hermetis log.
    hermetis_tail: LogTail,
    /// Tail of the embed log.
    embed_tail: LogTail,
    luna_tail: LogTail,
    mercury_tail: LogTail,
    supervise_tail: LogTail,
    /// Byte offset of the newest `decode heartbeat` line seen last poll, or
    /// None before the first sighting. Lets the poller tell "still generating"
    /// (offset advanced) from "went idle" (offset unchanged since the previous
    /// poll).
    decode_beat_offset: Option<u64>,
    /// Last sane live-decode rate seen in a heartbeat. Replayed while
    /// `/slots` reports a busy slot but the log has gone quiet — the server's
    /// stdout is block-buffered under nohup, so heartbeat lines arrive in
    /// multi-second bursts even at ~1 Hz emission.
    last_live_t_s: f64,
    /// Gen port adopted via `/proc` discovery after the configured port failed
    /// (`vitriol serve` on a non-default port). Cleared when it stops answering
    /// so discovery can re-run.
    gen_port_override: Option<u16>,
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
                luna_beat_offset: None,
                last_gpus: Vec::new(),
                hermetis_tail: LogTail::new(LOG_TAIL_CAP),
                embed_tail: LogTail::new(LOG_TAIL_CAP),
                luna_tail: LogTail::new(LOG_TAIL_CAP),
                mercury_tail: LogTail::new(LOG_TAIL_CAP),
                supervise_tail: LogTail::new(LOG_TAIL_CAP),
                decode_beat_offset: None,
                last_live_t_s: 0.0,
                gen_port_override: None,
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
        self.luna_tail.poll(&self.cfg.luna_log);
        self.mercury_tail.poll(&self.cfg.mercury_log);
        self.supervise_tail.poll(&self.cfg.supervise_log);
        let gpus = nvidia::query_gpus();
        self.last_gpus = gpus.clone();
        let gpu_processes = nvidia::query_processes(&gpus);
        let rebis = self.poll_rebis(agent);
        let snap = Snapshot {
            gen: self.poll_gen(agent),
            hermetis: poll_hermetis(agent, &self.cfg),
            embed: poll_embed(agent, &self.cfg),
            gpus,
            gpu_processes,
            rebis,
            logs: LogsSnapshot {
                gen: self.gen_tail.snapshot(),
                hermetis: self.hermetis_tail.snapshot(),
                embed: self.embed_tail.snapshot(),
                luna: self.luna_tail.snapshot(),
                mercury: self.mercury_tail.snapshot(),
                supervise: self.supervise_tail.snapshot(),
            },
        };
        let _ = tx.send(snap);
    }

    /// Poll the REBIS layer: Mercury gateway + Sol/Luna head health,
    /// Luna velocity, and the shim event stream from the distill store.
    fn poll_rebis(&mut self, agent: &Agent) -> RebisSnapshot {
        let mut snap = RebisSnapshot::default();
        let mut latency = |port: u16| -> (bool, u32) {
            let t0 = std::time::Instant::now();
            let up = health_up(agent, port);
            (up, t0.elapsed().as_millis() as u32)
        };
        (snap.sol_up, snap.sol_latency_ms) = latency(self.cfg.gen_port);
        (snap.luna_up, snap.luna_latency_ms) = latency(self.cfg.luna_port);
        (snap.mercury_up, snap.mercury_latency_ms) = latency(self.cfg.gateway_port);

        // cumulative predicted tokens from /metrics on both heads
        for (port, slot) in [(self.cfg.gen_port, 0u8), (self.cfg.luna_port, 1u8)] {
            if let Some(text) = req_text(agent, &format!("http://127.0.0.1:{port}/metrics")) {
                for line in text.lines() {
                    if let Some(v) = line.strip_prefix("llamacpp:tokens_predicted_total ") {
                        let n: u64 = v.trim().parse().unwrap_or(0);
                        if slot == 0 { snap.sol_tokens_total = n; }
                        else { snap.luna_tokens_total = n; }
                    }
                }
            }
        }

        // map head -> its GPU utilisation (Sol=GPU0, Luna=GPU1)
        if let Some(g0) = self.last_gpus.iter().find(|g| g.index == 0) {
            snap.sol_util_pct = g0.util_pct;
        }
        if let Some(g1) = self.last_gpus.iter().find(|g| g.index == 1) {
            snap.luna_util_pct = g1.util_pct;
        }

        if snap.luna_up {
            if let Some(models) = get_json(
                agent, &format!("http://127.0.0.1:{}/v1/models", self.cfg.luna_port))
            {
                snap.luna_model = models["models"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|m| m.get("model"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
                    .or_else(|| models["data"].as_array()
                        .and_then(|a| a.first())
                        .and_then(|m| m.get("id"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned));
            }
        }
        // Luna heartbeat lives in her log (same format as Sol's).
        snap.luna_decode_t_s = parse_decode_speed(
            &std::path::PathBuf::from("/tmp/mellum.log"),
            &mut self.luna_beat_offset);

        // Shim/distill event stream: aggregate + recent tail.
        const MAX_LINES: usize = 400;
        let path = self.cfg.distill_dir.join("shim-events.jsonl");
        let Ok(content) = std::fs::read_to_string(&path) else {
            return snap;
        };
        let lines: Vec<&str> = content.lines().collect();
        for line in lines.iter().rev().take(MAX_LINES).rev() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let kind = v.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let session = v.get("session").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let session_c = session.clone();
            match kind.as_str() {
                "gateway_turn" => {
                    let route = v.get("route").and_then(|x| x.as_str()).unwrap_or("");
                    match route {
                        "reason" => snap.routes[0] += 1,
                        "draft" => snap.routes[1] += 1,
                        "pipeline" => snap.routes[2] += 1,
                        _ => {}
                    }
                    snap.recent.push(RebisEvent {
                        ts: v.get("ts").cloned().unwrap_or_default().to_string(),
                        kind: kind.clone(),
                        session: session.clone(),
                        detail: format!("route {route}"),
                    });
                }
                "pipeline_audited" => {
                    let complete = v.get("complete").and_then(|x| x.as_bool()).unwrap_or(false);
                    if complete { snap.audits_pass += 1 } else { snap.audits_fail += 1 }
                    snap.recent.push(RebisEvent {
                        ts: v.get("ts").cloned().unwrap_or_default().to_string(),
                        kind: kind.clone(),
                        session: session.clone(),
                        detail: format!("audit {}",
                            if complete { "PASS" } else { "FAIL" }),
                    });
                }
                _ => {}
            }
            // other kinds folded into recent below via generic push
            if matches!(kind.as_str(), "steer_correct" | "compaction") {
                snap.recent.push(RebisEvent {
                    ts: v.get("ts").cloned().unwrap_or_default().to_string(),
                    kind: kind.clone(),
                    session,
                    detail: match kind.as_str() {
                        "steer_correct" => format!("corrected: {}",
                            v.get("missing_actions").and_then(|x| x.as_array())
                                .map(|a| a.len()).unwrap_or(0)),
                        "compaction" => format!("digested {} turns",
                            v.get("summarized_turns").and_then(|x| x.as_u64()).unwrap_or(0)),
                        _ => String::new(),
                    },
                });
                if kind == "compaction" { snap.compactions += 1; }
            }
        }
        // de-dup: pipeline_audited pushed once above already handled; trim
        for ev in snap.recent.iter_mut() { if ev.ts.is_empty() { ev.ts = "-".into(); } }
        if snap.recent.len() > 60 {
            let cut = snap.recent.len() - 60;
            snap.recent.drain(0..cut);
        }
        snap
    }

    /// Poll the gen server: `/health`, `/v1/models`, `/slots`, `/props`,
    /// `/metrics`, and the log-derived decode state (sticky last-completion
    /// throughput plus the live heartbeat rate).
    ///
    /// When `/health` fails on the configured port, llama-server processes are
    /// discovered via `/proc` and their `--port` adopted (covers
    /// `vitriol serve` on a non-default port and the memory-mode internal
    /// server on PORT−1).
    fn poll_gen(&mut self, agent: &Agent) -> GenSnapshot {
        let mut effective_port = self.gen_port_override.unwrap_or(self.cfg.gen_port);
        let mut up = health_up(agent, effective_port);
        if !up {
            // Configured/override port dead: re-discover.
            for port in discover_llama_ports() {
                if port == self.cfg.gen_port {
                    continue;
                }
                if health_up(agent, port) {
                    effective_port = port;
                    up = true;
                    break;
                }
            }
        }
        self.gen_port_override = if up && effective_port != self.cfg.gen_port {
            Some(effective_port)
        } else {
            None
        };
        let base = format!("http://127.0.0.1:{effective_port}");
        let mut model = None;
        let mut n_ctx = None;
        if up {
            if let Some(models) = get_json(agent, &format!("{base}/v1/models")) {
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
        }
        let slots = poll_slots(agent, &base);
        let busy = slots.iter().any(|s| s.is_processing);
        // Heartbeat rate, replayed while a slot is busy but the block-buffered
        // log has not flushed a fresh beat yet.
        let mut live = parse_decode_speed(&self.cfg.gen_log(), &mut self.decode_beat_offset);
        if live > 0.0 {
            self.last_live_t_s = live;
        } else if busy {
            live = self.last_live_t_s;
        } else {
            self.last_live_t_s = 0.0;
        }
        GenSnapshot {
            up,
            model,
            n_ctx,
            n_parallel: None,
            decode_t_s: parse_decode_t_s(&self.cfg.gen_log()),
            decode_speed: live,
            perf: parse_perf(&self.cfg.gen_log()),
            slots,
            draft: poll_draft(agent, &base),
            totals: poll_metrics_totals(agent, &base),
            effective_port: if effective_port != self.cfg.gen_port {
                Some(effective_port)
            } else {
                None
            },
        }
    }
}

/// Whether `/health` answers on `port`.
/// Plain GET returning the body as String (for /metrics text endpoints).
fn req_text(agent: &Agent, url: &str) -> Option<String> {
    match agent.get(url).call() {
        Ok(resp) => resp.into_string().ok(),
        Err(_) => None,
    }
}

fn health_up(agent: &Agent, port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    get_json(agent, &url)
        .map(|v| v.get("status").is_some())
        .unwrap_or(false)
}

/// Scan `/proc` for running `llama-server` processes and return the distinct
/// ports from their `--port N` arguments, ascending.
fn discover_llama_ports() -> Vec<u16> {
    let mut ports = Vec::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return ports;
    };
    for entry in entries.flatten() {
        let Ok(pid_text) = entry.file_name().into_string() else {
            continue;
        };
        if pid_text.parse::<u32>().is_err() {
            continue; // numeric-only dirs are processes
        }
        let Ok(cmd) = std::fs::read_to_string(format!("/proc/{pid_text}/cmdline")) else {
            continue; // vanished or not ours
        };
        let args: Vec<&str> = cmd.split('\0').filter(|a| !a.is_empty()).collect();
        let is_llama_server = args
            .first()
            .map(|bin| bin.ends_with("/llama-server") || *bin == "llama-server")
            .unwrap_or(false);
        if !is_llama_server {
            continue;
        }
        if let Some(i) = args.iter().position(|a| *a == "--port") {
            if let Some(p) = args.get(i + 1).and_then(|v| v.parse::<u16>().ok()) {
                ports.push(p);
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Poll `GET /slots` into per-slot snapshots; empty when disabled or down.
fn poll_slots(agent: &Agent, base: &str) -> Vec<SlotSnapshot> {
    get_json(agent, &format!("{base}/slots"))
        .map(parse_slots)
        .unwrap_or_default()
}

/// Parse the `/slots` response body (bare array or `{slots: [...]}`).
fn parse_slots(body: Value) -> Vec<SlotSnapshot> {
    let Some(slots) = body
        .as_array()
        .cloned()
        .or_else(|| body.get("slots").and_then(|s| s.as_array()).cloned())
    else {
        return Vec::new();
    };
    slots
        .iter()
        .map(|s| {
            // `next_token` serializes as a one-element array in this fork
            // ({ [{...}] } in C++), but tolerate the plain-object form too.
            let next_token = s.get("next_token").and_then(|n| {
                if let Some(arr) = n.as_array() {
                    arr.first().cloned()
                } else {
                    Some(n.clone())
                }
            });
            SlotSnapshot {
                id: s.get("id").and_then(|v| v.as_u64()).unwrap_or(0),
                id_task: s.get("id_task").and_then(|v| v.as_u64()),
                is_processing: s
                    .get("is_processing")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
                n_decoded: next_token
                    .as_ref()
                    .and_then(|n| n.get("n_decoded"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                n_remain: next_token
                    .as_ref()
                    .and_then(|n| n.get("n_remain"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
            }
        })
        .collect()
}

/// Poll `GET /props` for speculative-draft aggregates; `None` when absent.
fn poll_draft(agent: &Agent, base: &str) -> Option<DraftSnapshot> {
    get_json(agent, &format!("{base}/props")).and_then(parse_draft)
}

/// Parse the `/props` draft object.
fn parse_draft(props: Value) -> Option<DraftSnapshot> {
    let draft = props.get("draft")?;
    Some(DraftSnapshot {
        n_total: draft.get("n_total").and_then(|v| v.as_u64()).unwrap_or(0),
        n_accepted: draft
            .get("n_accepted")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    })
}

/// Poll `GET /metrics` (prometheus text) and extract the counters/gauges the
/// dashboard shows; defaults when the endpoint is disabled.
fn poll_metrics_totals(agent: &Agent, base: &str) -> MetricsTotals {
    let text = match agent.get(&format!("{base}/metrics")).call() {
        Ok(resp) => match resp.into_string() {
            Ok(t) => t,
            Err(_) => return MetricsTotals::default(),
        },
        Err(_) => return MetricsTotals::default(),
    };
    parse_metrics_text(&text)
}

/// Parse the prometheus exposition text into dashboard totals.
fn parse_metrics_text(text: &str) -> MetricsTotals {
    let mut t = MetricsTotals::default();
    for line in text.lines() {
        let (name, value) = match line.rsplit_once(' ') {
            Some(pair) if line.starts_with("llamacpp:") => pair,
            _ => continue,
        };
        let name = name.trim_start_matches("llamacpp:");
        match name {
            "prompt_tokens_total" => t.prompt_tokens_total = parse_metric_u64(value),
            "tokens_predicted_total" => t.tokens_predicted_total = parse_metric_u64(value),
            "predicted_tokens_seconds" => t.predicted_tokens_seconds = parse_metric_f64(value),
            "prompt_tokens_seconds" => t.prompt_tokens_seconds = parse_metric_f64(value),
            "requests_processing" => t.requests_processing = parse_metric_u64(value),
            _ => {}
        }
    }
    t
}

/// Parse an unsigned prometheus sample value, tolerating junk.
fn parse_metric_u64(value: &str) -> u64 {
    value.trim().parse::<f64>().map(|v| v as u64).unwrap_or(0)
}

/// Parse a float prometheus sample value, tolerating junk.
fn parse_metric_f64(value: &str) -> f64 {
    value.trim().parse::<f64>().unwrap_or(0.0)
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

/// Live-decode rates at or above this are server rounding artifacts (a 1-token
/// decode inside one millisecond reports 1e6 t/s) and are ignored.
const DECODE_SPEED_SANITY_CAP: f64 = 1000.0;

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
                if v.is_finite() && v > 0.0 && v < DECODE_SPEED_SANITY_CAP {
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

    #[test]
    fn slots_body_parses_processing_and_idle() {
        // `next_token` as one-element array — the shape this fork emits.
        let body = serde_json::json!([
            {
                "id": 0, "n_ctx": 8192, "is_processing": true, "id_task": 42,
                "next_token": [{"n_decoded": 123, "n_remain": 389}]
            },
            {
                "id": 1, "n_ctx": 8192, "is_processing": false,
                "next_token": [{"n_decoded": 0, "n_remain": 0}]
            }
        ]);
        let slots = parse_slots(body);
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].id, 0);
        assert_eq!(slots[0].id_task, Some(42));
        assert!(slots[0].is_processing);
        assert_eq!(slots[0].n_decoded, 123);
        assert_eq!(slots[0].n_remain, 389);
        assert!((slots[0].progress().unwrap() - 0.24).abs() < 0.01);
        assert!(!slots[1].is_processing);
        assert!(slots[1].progress().is_none());
    }

    #[test]
    fn slots_next_token_object_form_parses() {
        let body = serde_json::json!([
            {"id": 0, "is_processing": true,
             "next_token": {"n_decoded": 5, "n_remain": 15}}
        ]);
        let slots = parse_slots(body);
        assert_eq!(slots[0].n_decoded, 5);
        assert_eq!(slots[0].n_remain, 15);
    }

    #[test]
    fn slots_wrapped_object_parses() {
        let body = serde_json::json!({"slots": [
            {"id": 2, "is_processing": false}
        ]});
        let slots = parse_slots(body);
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].id, 2);
        assert!(!slots[0].is_processing);
    }

    #[test]
    fn draft_metrics_parse_with_rate() {
        let props = serde_json::json!({"draft": {"n_total": 100, "n_accepted": 87}});
        let d = parse_draft(props).expect("draft present");
        assert_eq!(d.n_total, 100);
        assert_eq!(d.n_accepted, 87);
        assert!((d.acceptance_rate().unwrap() - 0.87).abs() < 1e-9);
    }

    #[test]
    fn draft_absent_is_none_and_zero_total_has_no_rate() {
        assert!(parse_draft(serde_json::json!({})).is_none());
        let d = parse_draft(serde_json::json!({"draft": {"n_total": 0, "n_accepted": 0}})).unwrap();
        assert!(d.acceptance_rate().is_none());
    }

    #[test]
    fn metrics_text_extracts_counters_and_gauges() {
        let text = [
            "# HELP llamacpp:prompt_tokens_total Number of prompt tokens processed.",
            "# TYPE llamacpp:prompt_tokens_total counter",
            "llamacpp:prompt_tokens_total 1204",
            "# HELP llamacpp:tokens_predicted_total Predicted tokens.",
            "# TYPE llamacpp:tokens_predicted_total counter",
            "llamacpp:tokens_predicted_total 567",
            "# HELP llamacpp:predicted_tokens_seconds Average generation t/s.",
            "# TYPE llamacpp:predicted_tokens_seconds gauge",
            "llamacpp:predicted_tokens_seconds 9.98",
            "# HELP llamacpp:prompt_tokens_seconds Average prompt t/s.",
            "# TYPE llamacpp:prompt_tokens_seconds gauge",
            "llamacpp:prompt_tokens_seconds 210.5",
            "# HELP llamacpp:requests_processing Requests processing.",
            "# TYPE llamacpp:requests_processing gauge",
            "llamacpp:requests_processing 1",
        ]
        .join("\n");
        let t = parse_metrics_text(&text);
        assert_eq!(t.prompt_tokens_total, 1204);
        assert_eq!(t.tokens_predicted_total, 567);
        assert!((t.predicted_tokens_seconds - 9.98).abs() < 1e-9);
        assert!((t.prompt_tokens_seconds - 210.5).abs() < 1e-9);
        assert_eq!(t.requests_processing, 1);
    }

    #[test]
    fn metrics_text_ignores_help_lines_and_junk() {
        let t = parse_metrics_text("# HELP llamacpp:tokens_predicted_total x\nnot a metric\n");
        assert_eq!(t.tokens_predicted_total, 0);
    }

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
    fn decode_speed_ignores_sub_millisecond_artifacts() {
        let p = temp_log(
            "beatcap",
            &[
                "decode heartbeat: 1 tokens,  1000000.00 tokens per second (live)\n",
                "decode heartbeat: 17 tokens,  23.40 tokens per second (live)\n",
            ]
            .join(""),
        );
        let mut last = None;
        // The 1e6 beat is bogus; the newest sane beat wins.
        assert_eq!(parse_decode_speed(&p, &mut last), 23.40);
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
