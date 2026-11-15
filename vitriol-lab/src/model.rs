// PROVENANCE: inspiration — none; explicit VITRIOL experiment inputs, not hardware specifications.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const MAX_EVENTS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Synthetic,
    Recorded,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostOrigin {
    Assumed,
    Measured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub origin: Origin,
    pub costs: CostOrigin,
    pub engine_fingerprint: String,
    pub hardware_fingerprint: String,
}

impl Provenance {
    pub fn validate(&self) -> Result<(), String> {
        crate::label(&self.engine_fingerprint)?;
        crate::label(&self.hardware_fingerprint)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gpu {
    pub id: String,
    pub capacity_bytes: u64,
    pub fixed_bytes: u64,
    pub slot_bytes: u64,
    pub external_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: String,
    pub at_us: u64,
    pub session: String,
    pub branch: String,
    pub model: String,
    pub layout: String,
    pub prefix_ids: Vec<u64>,
    pub state_prefix_ids: Vec<u64>,
    pub snapshot_bytes: u64,
    pub protected: bool,
    pub prefill_us_per_token: u64,
    pub decode_us: u64,
    pub ram_restore_us: u64,
    pub disk_restore_us: u64,
    #[serde(default)]
    pub reprocess_tokens: usize,
    #[serde(default)]
    pub gpu_external_bytes: Option<Vec<u64>>,
}

impl Request {
    fn validate(&self) -> Result<(), String> {
        for value in [
            &self.id,
            &self.session,
            &self.branch,
            &self.model,
            &self.layout,
        ] {
            crate::label(value)?;
        }
        if self.prefix_ids.is_empty()
            || self.prefix_ids.len() > 262_144
            || self.state_prefix_ids.len() > 262_144
        {
            return Err("prefix lengths must be bounded; request prefix cannot be empty".into());
        }
        if self.decode_us == 0
            || self.prefill_us_per_token == 0
            || self.reprocess_tokens > self.prefix_ids.len()
        {
            return Err("positive costs and valid reprocess_tokens are required".into());
        }
        if self.snapshot_bytes > (1 << 40) {
            return Err("snapshot exceeds 1 TiB experiment bound".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trace {
    pub version: u32,
    pub provenance: Provenance,
    pub gpus: Vec<Gpu>,
    pub requests: Vec<Request>,
}

impl Trace {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.requests.is_empty()
            || self.requests.len() > MAX_EVENTS
            || self.gpus.len() > 16
        {
            return Err("unsupported version or invalid event/device count".into());
        }
        self.provenance.validate()?;
        let mut ids = HashSet::new();
        let mut previous = 0;
        for request in &self.requests {
            request.validate()?;
            if request.at_us < previous || !ids.insert(&request.id) {
                return Err("request IDs must be unique and arrival times nondecreasing".into());
            }
            if request
                .gpu_external_bytes
                .as_ref()
                .is_some_and(|v| v.len() != self.gpus.len())
            {
                return Err("external allocation vector must cover every GPU".into());
            }
            previous = request.at_us;
        }
        self.validate_gpus()
    }

    fn validate_gpus(&self) -> Result<(), String> {
        let mut ids = HashSet::new();
        for gpu in &self.gpus {
            crate::label(&gpu.id)?;
            if gpu.capacity_bytes == 0 || !ids.insert(&gpu.id) {
                return Err("GPU IDs must be unique and capacities positive".into());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Eviction {
    Lru,
    Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Dispatch {
    Fifo,
    Affinity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub name: String,
    pub slots: usize,
    pub ram_bytes: u64,
    pub disk_bytes: u64,
    pub max_entries: usize,
    pub eviction: Eviction,
    pub dispatch: Dispatch,
    pub rewind_tokens: usize,
}

impl Config {
    pub fn validate(&self) -> Result<(), String> {
        crate::label(&self.name)?;
        if self.slots == 0 || self.slots > 16 || self.max_entries == 0 || self.max_entries > 4096 {
            return Err("slots must be 1..16 and max_entries 1..4096".into());
        }
        Ok(())
    }
}

pub fn reusable(state: &Request, request: &Request, rewind: usize) -> usize {
    if state.model != request.model || state.layout != request.layout {
        return 0;
    }
    let common = state
        .state_prefix_ids
        .iter()
        .zip(&request.prefix_ids)
        .take_while(|(a, b)| a == b)
        .count();
    if state.state_prefix_ids.len().saturating_sub(common) > rewind {
        return 0;
    }
    common.min(request.prefix_ids.len() - request.reprocess_tokens)
}
