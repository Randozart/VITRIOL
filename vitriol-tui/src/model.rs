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
    /// Compute processes using the GPU.
    pub processes: Vec<GpuProcess>,
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
}

impl Snapshot {
    /// Whether at least one service or the GPU reported anything. Used by the
    /// UI to hint that the stack is not reachable.
    pub fn is_empty(&self) -> bool {
        !self.gen.up && !self.hermetis.up && !self.embed.up && self.gpu.is_none()
    }
}
