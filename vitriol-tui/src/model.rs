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
}

/// GPU telemetry from `nvidia-smi`.
#[derive(Debug, Clone, Default)]
pub struct GpuSnapshot {
    /// Whether `nvidia-smi` answered with a usable GPU line.
    pub present: bool,
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
    /// Compute processes using the GPU.
    pub processes: Vec<GpuProcess>,
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
    /// GPU state; `None` when `nvidia-smi` is unavailable.
    pub gpu: Option<GpuSnapshot>,
    /// Live log tails for each service.
    pub logs: LogsSnapshot,
}

impl Snapshot {
    /// Whether at least one service or the GPU reported anything. Used by the
    /// UI to hint that the stack is not reachable.
    pub fn is_empty(&self) -> bool {
        !self.gen.up && !self.hermetis.up && !self.embed.up && self.gpu.is_none()
    }
}
