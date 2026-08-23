//! Telemetry snapshots published by the poller to the UI thread.
//!
//! Every field is a plain value or `Option` — the poller never panics on a
//! down service, it just records `up: false`. The UI renders from whatever the
//! latest snapshot carries.

/// Live gen-server state.
#[derive(Debug, Clone, Default)]
pub struct GenSnapshot {
    /// Whether `/health` answered within the poll timeout.
    pub up: bool,
    /// Model id reported by `/v1/models`.
    pub model: Option<String>,
    /// Model context length, when the server reports it.
    pub n_ctx: Option<u64>,
    /// Number of parallel slots, when the server reports it.
    pub n_parallel: Option<u64>,
    /// Latest decode throughput parsed from the gen log (`eval time` line).
    pub decode_t_s: f64,
    /// Live decode throughput from the in-progress heartbeat line, or 0.0 when
    /// no slot is actively decoding. Unlike `decode_t_s` (sticky last
    /// completion), this falls to 0.0 as soon as generation stops.
    pub decode_speed: f64,
    /// Latest `[PERF]` decode breakdown line from the gen log, when present.
    pub perf: Option<PerfSnapshot>,
    /// Per-slot state from `/slots` (empty when the endpoint is disabled).
    pub slots: Vec<SlotSnapshot>,
    /// Speculative-draft aggregates from `/props`, when reported.
    pub draft: Option<DraftSnapshot>,
    /// Server-wide counters from `/metrics` (prometheus text), when enabled.
    pub totals: MetricsTotals,
    /// Port the poller actually talks to (differs from config when the port
    /// was auto-discovered via `/proc`).
    pub effective_port: Option<u16>,
}

/// One slot row from `GET /slots`.
#[derive(Debug, Clone, Default)]
pub struct SlotSnapshot {
    /// Slot index.
    pub id: u64,
    /// Task id currently assigned to the slot.
    pub id_task: Option<u64>,
    /// Whether the slot is mid-generation.
    pub is_processing: bool,
    /// Tokens decoded for the current task.
    pub n_decoded: u64,
    /// Tokens remaining until the current task finishes.
    pub n_remain: u64,
}

impl SlotSnapshot {
    /// Decode progress fraction 0..1; `None` when nothing to show.
    pub fn progress(&self) -> Option<f64> {
        if !self.is_processing {
            return None;
        }
        let total = self.n_decoded + self.n_remain;
        if total == 0 {
            return None;
        }
        Some(self.n_decoded as f64 / total as f64)
    }

    pub fn total_tokens(&self) -> u64 {
        self.n_decoded + self.n_remain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_tokens_sums() {
        let snapshot = SlotSnapshot { n_decoded: 123, n_remain: 389, ..Default::default() };
        assert_eq!(snapshot.total_tokens(), 512);
    }
}

/// Speculative-decoding counters from `/props` (`draft` object).
#[derive(Debug, Clone, Copy, Default)]
pub struct DraftSnapshot {
    /// Draft tokens generated (server-wide).
    pub n_total: u64,
    /// Draft tokens accepted (server-wide).
    pub n_accepted: u64,
}

impl DraftSnapshot {
    /// Acceptance rate 0..1, or `None` before any draft token.
    pub fn acceptance_rate(&self) -> Option<f64> {
        if self.n_total == 0 {
            return None;
        }
        Some(self.n_accepted as f64 / self.n_total as f64)
    }
}

/// Server-wide counters parsed from the `/metrics` prometheus text.
#[derive(Debug, Clone, Copy, Default)]
pub struct MetricsTotals {
    /// Total prompt tokens processed since start.
    pub prompt_tokens_total: u64,
    /// Total predicted tokens since start.
    pub tokens_predicted_total: u64,
    /// Average generation t/s gauge (current window).
    pub predicted_tokens_seconds: f64,
    /// Average prompt t/s gauge (current window).
    pub prompt_tokens_seconds: f64,
    /// Slots currently processing.
    pub requests_processing: u64,
}

/// One decoded `[PERF]` line emitted by the VITRIOL server (`GGML_CUDA_GDN_PROFILE=1`).
/// Breaks a single `llama_decode` into graph-build / graph-compute / post,
/// plus the CUDA-graph capture-vs-replay tally and the MTP-hook sync stalls.
#[derive(Debug, Clone, Default)]
pub struct PerfSnapshot {
    /// Whole-decode wall time in ms.
    pub total_ms: f64,
    /// Graph build + alloc + set_inputs in ms.
    pub build_ms: f64,
    /// `graph_compute` (synchronous CUDA work) in ms.
    pub compute_ms: f64,
    /// Extraction + MTP hook (ctx_mtp decode + sync) in ms.
    pub post_ms: f64,
    /// CUDA graph captures this decode.
    pub n_capture: u64,
    /// CUDA graph replays this decode.
    pub n_replay: u64,
    /// Synchronize() stalls in the MTP hook.
    pub n_sync: u64,
    /// Accumulated sync stall time in ms.
    pub sync_ms: f64,
    /// Top-4 op classes by node count: (op name, count).
    pub top_ops: Vec<(String, u64)>,
}

/// One recent episode row from `/hermetis/recent`.
#[derive(Debug, Clone, Default)]
pub struct RecentStore {
    /// Episode id.
    pub id: i64,
    /// Speaker role (user/assistant/tool).
    pub role: String,
    /// Truncated content snippet for display.
    pub snippet: String,
}

/// Live Hermetis memory-server state.
#[derive(Debug, Clone, Default)]
pub struct HermetisSnapshot {
    /// Whether `/health` answered.
    pub up: bool,
    /// Episode count for the configured project, when reported.
    pub episodes: Option<u64>,
    /// Knowledge-node count for the configured project, when reported.
    pub nodes: Option<u64>,
    /// Session count for the configured project, when reported.
    pub sessions: Option<u64>,
    /// Most recent episodes for the configured project, newest first.
    pub recent: Vec<RecentStore>,
}

/// Live embed-server state.
#[derive(Debug, Clone, Default)]
pub struct EmbedSnapshot {
    /// Whether `/health` answered.
    pub up: bool,
}

/// One row of `nvidia-smi --query-compute-apps`.
#[derive(Debug, Clone)]
pub struct GpuProcess {
    /// Process id.
    pub pid: u32,
    /// Process name.
    pub name: String,
    /// GPU memory in MiB.
    pub vram_mib: u64,
    /// GPU index the process runs on, when attributable via uuid.
    pub gpu_index: Option<u8>,
}

/// GPU telemetry from `nvidia-smi` (one entry per physical GPU).
#[derive(Debug, Clone, Default)]
pub struct GpuSnapshot {
    /// Zero-based GPU index (`--query-gpu=index`).
    pub index: u8,
    /// GPU product name.
    pub name: String,
    /// Used GPU memory in MiB.
    pub vram_used_mib: u64,
    /// Total GPU memory in MiB.
    pub vram_total_mib: u64,
    /// Utilisation in percent.
    pub util_pct: u8,
    /// Temperature in Celsius.
    pub temp_c: u8,
    /// Power draw in watts (0.0 when not reported).
    pub power_w: f64,
    /// Power limit in watts (0.0 when not reported).
    pub power_limit_w: f64,
    /// SM clock in MHz.
    pub sm_clock_mhz: u16,
    /// Memory clock in MHz.
    pub mem_clock_mhz: u16,
    /// GPU uuid, used to attribute compute processes.
    pub uuid: String,
}

/// One captured REBIS gateway/shim event (from the distill store).
#[derive(Debug, Clone, Default)]
pub struct RebisEvent {
    /// ISO timestamp of the event.
    pub ts: String,
    /// Event kind: gateway_turn / pipeline_audited / steer_correct /
    /// compaction / shim_judged ...
    pub kind: String,
    /// Session key it belonged to.
    pub session: String,
    /// One-line human summary.
    pub detail: String,
}

/// Aggregated REBIS layer state for the dashboard tab.
#[derive(Debug, Clone, Default)]
pub struct RebisSnapshot {
    /// Mercury gateway answering on its port.
    pub mercury_up: bool,
    /// Sol head answering.
    pub sol_up: bool,
    /// Luna head answering.
    pub luna_up: bool,
    /// Luna model id when reported.
    pub luna_model: Option<String>,
    /// Luna live decode tok/s from her heartbeat log.
    pub luna_decode_t_s: f64,
    /// Route counters: [reason, draft, pipeline].
    pub routes: [u32; 3],
    /// Audited turns that passed.
    pub audits_pass: u32,
    /// Audited turns that failed (corrected or escalated).
    pub audits_fail: u32,
    /// Compaction events.
    pub compactions: u32,
    /// Newest events, capped, oldest first.
    pub recent: Vec<RebisEvent>,
    /// Health-check round-trip in ms (lower = responsive).
    pub mercury_latency_ms: u32,
    pub sol_latency_ms: u32,
    pub luna_latency_ms: u32,
    /// Cumulative predicted tokens since server start (from /metrics).
    pub sol_tokens_total: u64,
    pub luna_tokens_total: u64,
    /// GPU utilisation of each head's card.
    pub sol_util_pct: u8,
    pub luna_util_pct: u8,
}

/// Live tail of a service log, newest last, capped at [`LOG_TAIL_CAP`] lines.
#[derive(Debug, Clone, Default)]
pub struct LogsSnapshot {
    /// Tail of the gen (`vitriol_gen.log`) log.
    pub gen: Vec<String>,
    /// Tail of the Hermetis (`copula_hermetis.log`) log.
    pub hermetis: Vec<String>,
    /// Tail of the embed (`copula_embed.log`) log.
    pub embed: Vec<String>,
}

/// One full poll result. All services are optional; a down service simply
/// leaves its flags false and its values `None`.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Gen server state.
    pub gen: GenSnapshot,
    /// Hermetis state.
    pub hermetis: HermetisSnapshot,
    /// Embed state.
    pub embed: EmbedSnapshot,
    /// GPU state, one entry per physical GPU; empty when `nvidia-smi` is
    /// unavailable.
    pub gpus: Vec<GpuSnapshot>,
    /// Compute processes across all GPUs (uuid-attributed).
    pub gpu_processes: Vec<GpuProcess>,
    /// REBIS gateway/head state + event stream aggregates.
    pub rebis: RebisSnapshot,
    /// Live log tails for each service.
    pub logs: LogsSnapshot,
}

impl Snapshot {
    /// Summed VRAM across all GPUs: (used MiB, total MiB).
    pub fn vram_totals(&self) -> (u64, u64) {
        (
            self.gpus.iter().map(|g| g.vram_used_mib).sum(),
            self.gpus.iter().map(|g| g.vram_total_mib).sum(),
        )
    }

    /// Whether at least one service or the GPU reported anything. Used by the
    /// UI to hint that the stack is not reachable.
    pub fn is_empty(&self) -> bool {
        !self.gen.up && !self.hermetis.up && !self.embed.up && self.gpus.is_empty()
    }
}
