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
use ureq::Agent;

use crate::config::Config;
use crate::control::{AUTOSAVE_UNIT, ENGINE_UNIT};
use crate::intel;
use crate::model::{
    DraftSnapshot, GenSnapshot, LogsSnapshot, MetricsTotals, PerfSnapshot, SlotSnapshot, Snapshot,
};
use crate::{nvidia, profile};

const LOG_TAIL_CAP: usize = 200;

struct Poller {
    cfg: Config,
    gen_tail: LogTail,
    decode_beat_offset: Option<u64>,
    last_live_t_s: f64,
}

/// Spawn the poller thread. `refresh_flag` is raised by the UI to force an
/// immediate poll (the `r` key).
pub fn spawn(cfg: Config, tx: Sender<Snapshot>, refresh_flag: Arc<AtomicBool>) {
    thread::Builder::new()
        .name("vitriol-tui-poller".into())
        .spawn(move || {
            let agent = http_agent(Duration::from_secs(3));
            let mut poller = Poller {
                gen_tail: LogTail::new(LOG_TAIL_CAP),
                decode_beat_offset: None,
                last_live_t_s: 0.0,
                cfg,
            };
            loop {
                if !poller.poll_once(&agent, &tx) {
                    break;
                }
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
    fn poll_once(&mut self, agent: &Agent, tx: &Sender<Snapshot>) -> bool {
        self.gen_tail.poll(&self.cfg.gen_log());
        let mut gpus = nvidia::query_gpus();
        if gpus.is_empty() {
            gpus = intel::query_gpus();
        }
        let gpu_processes = if gpus.first().is_some_and(|gpu| gpu.uuid.is_empty()) {
            intel::query_processes(&gpus)
        } else {
            nvidia::query_processes(&gpus)
        };
        let config = fs::read_to_string(self.cfg.active_config()).unwrap_or_default();
        let settings = profile::parse_ini(&config);
        let configured_port = self
            .cfg
            .gen_port
            .or_else(|| settings.get("server.port")?.parse().ok())
            .unwrap_or(8279);
        let (service_state, service_pid) = poll_service();
        let gen = self.poll_gen(agent, configured_port, service_pid);
        let selected_model = if settings
            .get("server.launcher")
            .is_some_and(|v| v == "kintsugi")
        {
            Some("Kintsugi-27B-12G.gguf")
        } else {
            settings
                .get("model.path")
                .and_then(|path| std::path::Path::new(path).file_name()?.to_str())
        };
        let mismatch = gen.up
            && selected_model
                .zip(gen.model_path.as_deref())
                .is_some_and(|(selected, live)| selected != live);
        let configured_profile = profile::active_name(&config, &profile::discover(&self.cfg))
            .or_else(|| (!config.is_empty()).then(|| "Custom config".into()));
        tx.send(Snapshot {
            gen,
            service_state,
            configured_profile,
            mismatch,
            gpus,
            gpu_processes,
            logs: LogsSnapshot {
                gen: self.gen_tail.snapshot(),
                journal: journal(ENGINE_UNIT),
                autosave: journal(AUTOSAVE_UNIT),
            },
        })
        .is_ok()
    }

    fn poll_gen(&mut self, agent: &Agent, port: u16, service_pid: Option<u32>) -> GenSnapshot {
        let (port, pid, up) = endpoint(agent, port, service_pid, self.cfg.gen_port.is_some());
        if !up {
            self.last_live_t_s = 0.0;
            return GenSnapshot {
                effective_port: Some(port),
                ..GenSnapshot::default()
            };
        }
        let base = format!("http://127.0.0.1:{port}");
        let models = get_json(agent, &format!("{base}/v1/models"));
        let model_entry = models
            .as_ref()
            .and_then(|body| body["data"].as_array()?.first());
        let props = get_json(agent, &format!("{base}/props"));
        let model_path = props.as_ref().and_then(model_filename);
        let slots = poll_slots(agent, &base);
        let totals = poll_metrics_totals(agent, &base);
        let draft = props.and_then(parse_draft).or_else(|| {
            (totals.spec_draft_total > 0).then_some(DraftSnapshot {
                n_total: totals.spec_draft_total,
                n_accepted: totals.spec_accepted_total,
            })
        });
        let log_path = self.cfg.gen_log();
        GenSnapshot {
            up,
            model: model_entry
                .and_then(|entry| entry["id"].as_str())
                .map(str::to_owned),
            model_path,
            model_pid: pid,
            n_ctx: model_entry.and_then(first_model_size),
            n_parallel: (!slots.is_empty()).then_some(slots.len() as u64),
            decode_t_s: parse_decode_t_s(&log_path),
            decode_speed: self.live_decode(&log_path, slots.iter().any(|slot| slot.is_processing)),
            perf: parse_perf(&log_path),
            slots,
            draft,
            totals,
            effective_port: Some(port),
        }
    }

    fn live_decode(&mut self, log: &std::path::Path, busy: bool) -> f64 {
        let next = parse_decode_speed(log, &mut self.decode_beat_offset);
        if !busy {
            self.last_live_t_s = 0.0;
        } else if next > 0.0 {
            self.last_live_t_s = next;
        }
        self.last_live_t_s
    }
}

fn endpoint(
    agent: &Agent,
    port: u16,
    service_pid: Option<u32>,
    forced: bool,
) -> (u16, Option<u32>, bool) {
    let engines = discover_engines();
    let service = engines.iter().find(|(pid, _)| Some(*pid) == service_pid);
    let preferred = service.filter(|_| !forced).map_or(port, |(_, port)| *port);
    if health_up(agent, preferred) {
        let mut candidates = engines.iter().filter(|(_, port)| *port == preferred);
        let first = candidates.next();
        let owner = service
            .filter(|(_, port)| *port == preferred)
            .or_else(|| first.filter(|_| candidates.next().is_none()));
        return (preferred, owner.map(|(pid, _)| *pid), true);
    }
    if !forced && service.is_none() {
        if let [(pid, port)] = engines.as_slice() {
            if *port != preferred && health_up(agent, *port) {
                return (*port, Some(*pid), true);
            }
        }
    }
    (preferred, None, false)
}

fn model_filename(props: &Value) -> Option<String> {
    let path = props.get("model_path")?.as_str()?;
    std::path::Path::new(path)
        .file_name()?
        .to_str()
        .map(str::to_owned)
}

fn poll_service() -> (String, Option<u32>) {
    let output = std::process::Command::new("systemctl")
        .args([
            "--system",
            "show",
            "-p",
            "ActiveState",
            "-p",
            "MainPID",
            ENGINE_UNIT,
        ])
        .output();
    let Ok(output) = output else {
        return ("unknown".into(), None);
    };
    if !output.status.success() {
        return ("unknown".into(), None);
    }
    let body = String::from_utf8_lossy(&output.stdout);
    let state = body
        .lines()
        .find_map(|line| line.strip_prefix("ActiveState="))
        .unwrap_or("unknown");
    let pid = body
        .lines()
        .find_map(|line| line.strip_prefix("MainPID="))
        .and_then(|value| value.parse().ok());
    (state.into(), pid.filter(|pid| *pid > 0))
}

fn journal(unit: &str) -> Vec<String> {
    match std::process::Command::new("journalctl")
        .args([
            "--system",
            "--no-pager",
            "-b",
            "-n",
            "200",
            "-o",
            "short-iso",
            "-u",
            unit,
        ])
        .output()
    {
        Ok(output) if output.status.success() => {
            let mut lines = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let note = String::from_utf8_lossy(&output.stderr);
            if !note.trim().is_empty() {
                lines.push(format!("Journal: {}", note.trim().replace('\n', " ")));
            }
            lines
        }
        Ok(output) => vec![format!(
            "Journal unavailable: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )],
        Err(error) => vec![format!("Journal unavailable: {error}")],
    }
}

pub fn http_agent(timeout: Duration) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

/// Read a ureq 3 response body as text.
pub fn read_response_text(
    resp: &mut ureq::http::Response<ureq::Body>,
) -> Result<String, ureq::Error> {
    resp.body_mut().read_to_string()
}

/// Plain GET returning the body as String (for /metrics text endpoints).
fn health_up(agent: &Agent, port: u16) -> bool {
    let url = format!("http://127.0.0.1:{port}/health");
    get_json(agent, &url)
        .map(|v| v.get("status").and_then(Value::as_str) == Some("ok"))
        .unwrap_or(false)
}

/// Discover the local engine and legacy binary names using their launch arguments.
fn discover_engines() -> Vec<(u32, u16)> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let cmd = fs::read_to_string(entry.path().join("cmdline")).ok()?;
            Some((pid, engine_port(&cmd)?))
        })
        .collect()
}

fn engine_port(cmd: &str) -> Option<u16> {
    let mut args = cmd.split('\0');
    let binary = std::path::Path::new(args.next()?).file_name()?.to_str()?;
    if !matches!(binary, "vitriol-server" | "llama-server") {
        return None;
    }
    while let Some(arg) = args.next() {
        if matches!(arg, "--port" | "-port") {
            return args.next()?.parse().ok();
        }
        if let Some(port) = arg.strip_prefix("--port=") {
            return port.parse().ok();
        }
    }
    None
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
                // Slot window (2026-09-01): enables the filled-vs-window view.
                n_ctx: s.get("n_ctx").and_then(|v| v.as_u64()),
                filled: s.get("n_prompt_tokens").and_then(|v| v.as_u64()),
            }
        })
        .collect()
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
        Ok(mut resp) => match read_response_text(&mut resp) {
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
            "prompt_tokens_cached_total" => t.prompt_tokens_cached_total = parse_metric_u64(value),
            "spec_decode_num_draft_tokens_total" => t.spec_draft_total = parse_metric_u64(value),
            "spec_decode_num_accepted_tokens_total" => {
                t.spec_accepted_total = parse_metric_u64(value)
            }
            "spec_decode_num_drafts_total" => t.spec_draft_steps_total = parse_metric_u64(value),
            "kv_ejected_total" => t.kv_ejected_total = parse_metric_u64(value),
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

fn get_json(agent: &Agent, url: &str) -> Option<Value> {
    let mut resp = agent.get(url).call().ok()?;
    let text = read_response_text(&mut resp).ok()?;
    serde_json::from_str(&text).ok()
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
            let n: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
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
            if c.is_ascii_digit() {
                cap.push(c);
            } else {
                break;
            }
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
            if c.is_ascii_digit() {
                n.push(c);
            } else {
                break;
            }
        }
        p.n_sync = n.parse().unwrap_or(0);
        if let Some(j) = rest.find('(') {
            let tail = &rest[j + 1..];
            let mut m = String::new();
            for c in tail.chars() {
                if c.is_ascii_digit() || c == '.' {
                    m.push(c);
                } else {
                    break;
                }
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
        const MAX_READ: u64 = 256 * 1024;
        let start = self.pos.max(len.saturating_sub(MAX_READ));
        if f.seek(SeekFrom::Start(start)).is_err() {
            return;
        }
        let mut buf = vec![0u8; (len - start) as usize];
        if f.read_exact(&mut buf).is_err() {
            return;
        }
        let skipped = start > self.pos;
        self.pos = len;
        let text = String::from_utf8_lossy(&buf);
        let mut lines = text.lines();
        if skipped {
            lines.next();
        }
        for line in lines {
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
    fn live_schema_uses_model_path_filled_tokens_and_actual_server_binary() {
        assert_eq!(
            model_filename(&serde_json::json!({"model_alias": "Lapis Occultus", "model_path": "/models/Kintsugi-27B-12G.gguf"})).as_deref(),
            Some("Kintsugi-27B-12G.gguf")
        );
        let slots = parse_slots(serde_json::json!([{
            "id": 0, "n_ctx": 65536, "n_prompt_tokens": 4196,
            "next_token": [{"n_decoded": 6, "n_remain": 16}]
        }]));
        assert_eq!(slots[0].filled, Some(4196));
        assert_eq!(slots[0].total_tokens(), 22);
        assert_eq!(
            engine_port("/path/vitriol-server\0--port\08279\0"),
            Some(8279)
        );
        assert_eq!(engine_port("/path/llama-server\0--port=8280\0"), Some(8280));
        assert_eq!(engine_port("other-server\0--port\08279\0"), None);
    }

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
            "# HELP llamacpp:prompt_tokens_cached_total Number of prompt tokens reused from the cache",
            "# TYPE llamacpp:prompt_tokens_cached_total counter",
            "llamacpp:prompt_tokens_cached_total 745",
            "# HELP llamacpp:spec_decode_num_draft_tokens_total Speculative: Total draft tokens generated",
            "# TYPE llamacpp:spec_decode_num_draft_tokens_total counter",
            "llamacpp:spec_decode_num_draft_tokens_total 3000",
            "# HELP llamacpp:spec_decode_num_accepted_tokens_total Speculative: Total draft tokens accepted by the target model",
            "# TYPE llamacpp:spec_decode_num_accepted_tokens_total counter",
            "llamacpp:spec_decode_num_accepted_tokens_total 2010",
            "# HELP llamacpp:spec_decode_num_drafts_total Speculative: Total number of verification steps",
            "# TYPE llamacpp:spec_decode_num_drafts_total counter",
            "llamacpp:spec_decode_num_drafts_total 1000",
            "# HELP llamacpp:kv_ejected_total Sparse-KV tokens ejected",
            "# TYPE llamacpp:kv_ejected_total counter",
            "llamacpp:kv_ejected_total 512000",
        ]
        .join("\n");
        let t = parse_metrics_text(&text);
        assert_eq!(t.prompt_tokens_total, 1204);
        assert_eq!(t.tokens_predicted_total, 567);
        assert!((t.predicted_tokens_seconds - 9.98).abs() < 1e-9);
        assert!((t.prompt_tokens_seconds - 210.5).abs() < 1e-9);
        assert_eq!(t.requests_processing, 1);
        assert_eq!(t.prompt_tokens_cached_total, 745);
        assert_eq!(t.spec_draft_total, 3000);
        assert_eq!(t.spec_accepted_total, 2010);
        assert_eq!(t.spec_draft_steps_total, 1000);
        assert_eq!(t.kv_ejected_total, 512000);
    }

    #[test]
    fn metrics_per_position_series_do_not_pollute_aggregates() {
        // The labeled per-position acceptance series must not be mistaken for
        // the aggregate counters (rsplit on the value leaves the label suffix
        // glued to the name, which matches none of the aggregate arms).
        let text = [
            "llamacpp:spec_decode_num_accepted_tokens_per_pos_total{position=\"0\"} 41",
            "llamacpp:spec_decode_num_accepted_tokens_per_pos_total{position=\"1\"} 17",
        ]
        .join("\n");
        let t = parse_metrics_text(&text);
        assert_eq!(t.spec_accepted_total, 0);
        assert_eq!(t.spec_draft_total, 0);
    }

    #[test]
    fn draft_falls_back_to_spec_counters_when_props_silent() {
        // This engine serves no /props draft object; the poller derives the
        // acceptance snapshot from the /metrics counters instead.
        let mut totals = MetricsTotals::default();
        let draft = poll_draft_legacy_none().or_else(|| {
            (totals.spec_draft_total > 0).then(|| DraftSnapshot {
                n_total: totals.spec_draft_total,
                n_accepted: totals.spec_accepted_total,
            })
        });
        assert!(draft.is_none());
        totals.spec_draft_total = 3000;
        totals.spec_accepted_total = 2010;
        let draft = None::<DraftSnapshot>.or_else(|| {
            (totals.spec_draft_total > 0).then(|| DraftSnapshot {
                n_total: totals.spec_draft_total,
                n_accepted: totals.spec_accepted_total,
            })
        });
        let d = draft.expect("fallback draft");
        assert!((d.acceptance_rate().unwrap() - 0.67).abs() < 0.01);
    }

    /// Stand-in for `poll_draft` returning None (no /props draft object).
    fn poll_draft_legacy_none() -> Option<DraftSnapshot> {
        None
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
        std::fs::write(
            &p,
            [
                "slot test: id 0 | task 1 | ...\n",
                "decode heartbeat: 17 tokens,  23.40 tokens per second (live)\n",
            ]
            .join(""),
        )
        .unwrap();
        // First appearance is fresh → 23.40.
        assert_eq!(parse_decode_speed(&p, &mut last), 23.40);
        assert!(last.is_some());
        // No new bytes appended → idle again.
        assert_eq!(parse_decode_speed(&p, &mut last), 0.0);
        // A second, later beat advances past the first.
        std::fs::write(
            &p,
            [
                "slot test: id 0 | task 1 | ...\n",
                "decode heartbeat: 17 tokens,  23.40 tokens per second (live)\n",
                "decode heartbeat: 42 tokens,  40.12 tokens per second (live)\n",
            ]
            .join(""),
        )
        .unwrap();
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
        std::fs::write(
            &p,
            "decode heartbeat: 5 tokens,  25.00 tokens per second (live)\n",
        )
        .unwrap();
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
