//! Telemetry snapshots published by the poller to the UI thread.
//!
//! Every field is a plain value or `Option` — the poller never panics on a
//! down service, it just records `up: false`. The UI renders from whatever the
//! latest snapshot carries.

/// Live gen-server state.
#[derive(Debug, Clone, Default)]
pub struct GenSnapshot {
    /// Completed timing groups for this engine generation, latest per slot.
    pub completed_requests: Vec<crate::request_metrics::CompletedRequest>,
    pub restores: Vec<crate::request_metrics::SlotRestore>,
    pub checkpoint_io: Option<crate::request_metrics::CheckpointIo>,
    /// Whether `/health` answered within the poll timeout.
    pub up: bool,
    /// Model id reported by `/v1/models`.
    pub model: Option<String>,
    pub model_path: Option<String>,
    pub model_pid: Option<u32>,
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
    /// The slot's context window (2026-09-01 spring cleaning: previously
    /// dropped by the parser). Lets the UI show filled-vs-window per slot —
    /// the AGENTS.md "window ≠ depth" distinction.
    pub n_ctx: Option<u64>,
    /// Current filled positions from the server's prompt state.
    pub filled: Option<u64>,
    /// Newly evaluated prompt tokens for the active request. Idle counters
    /// may be reset by the server and are therefore unavailable here.
    pub prompt_processed: Option<u64>,
    /// Exact prefix tokens reused by the active request.
    pub prompt_cached: Option<u64>,
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

    /// True while the slot holds a task but has not decoded its first token
    /// yet — the prompt-eval phase (prefill), which the progress bar cannot
    /// represent.
    pub fn is_prompt_eval(&self) -> bool {
        self.is_processing && self.n_decoded == 0
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
        let snapshot = SlotSnapshot {
            n_decoded: 123,
            n_remain: 389,
            ..Default::default()
        };
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
    pub ttft_seconds_sum: Option<f64>,
    pub ttft_count: Option<u64>,
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
    /// Prompt tokens reused from the prompt cache (prefix matches).
    pub prompt_tokens_cached_total: u64,
    /// Speculative decoding: draft tokens generated.
    pub spec_draft_total: u64,
    /// Speculative decoding: draft tokens accepted by the target.
    pub spec_accepted_total: u64,
    /// Speculative decoding: verification steps taken (mean accepted/step).
    pub spec_draft_steps_total: u64,
    /// Sparse-KV tokens ejected by probe-scored eviction + eager sweeps.
    pub kv_ejected_total: u64,
}

impl MetricsTotals {
    /// Engine-wide task-post to first-token mean since process start.
    pub fn ttft_mean_seconds(&self) -> Option<f64> {
        let count = self.ttft_count.filter(|count| *count > 0)?;
        Some(self.ttft_seconds_sum? / count as f64)
    }
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

/// Bounded log views. Journal access errors are displayed in the corresponding source.
#[derive(Debug, Clone, Default)]
pub struct LogsSnapshot {
    pub gen: Vec<String>,
    pub slot_entries: Vec<String>,
    pub slot_tails: std::collections::BTreeMap<u64, Vec<String>>,
    pub journal: Vec<String>,
    pub autosave: Vec<String>,
    pub caduceus: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub gen: GenSnapshot,
    pub service_state: String,
    pub configured_profile: Option<String>,
    pub mismatch: bool,
    pub gpus: Vec<GpuSnapshot>,
    pub gpu_processes: Vec<GpuProcess>,
    pub logs: LogsSnapshot,
    pub caduceus: crate::caduceus::Status,
}
