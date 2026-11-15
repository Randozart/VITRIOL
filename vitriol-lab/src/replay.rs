// PROVENANCE: inspiration — none; VITRIOL-original deterministic event and capacity model.
use crate::cache::{Cache, Choice, Source, Stats};
use crate::model::{reusable, Config, Dispatch, Provenance, Trace};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

#[derive(Serialize)]
pub struct Outcome {
    pub id: String,
    pub slot: usize,
    pub start_us: u64,
    pub end_us: u64,
    pub queue_us: u64,
    pub cached_tokens: usize,
    pub processed_tokens: usize,
    pub prefill_us: u64,
    pub restore_us: u64,
    pub source: Source,
    pub uncached_reason: Option<&'static str>,
}

#[derive(Serialize)]
pub struct MemoryViolation {
    pub gpu: String,
    pub request: String,
    pub required_bytes: String,
    pub capacity_bytes: u64,
}

#[derive(Serialize)]
pub struct Capacity {
    pub status: &'static str,
    pub violations: Vec<MemoryViolation>,
}

fn capacity(trace: &Trace, cfg: &Config) -> Capacity {
    let mut violations = Vec::new();
    for request in &trace.requests {
        violations.extend(trace.gpus.iter().enumerate().filter_map(|(i, gpu)| {
            let external = request
                .gpu_external_bytes
                .as_ref()
                .map_or(gpu.external_bytes, |v| v[i]);
            let needed = u128::from(gpu.fixed_bytes)
                + u128::from(gpu.slot_bytes) * cfg.slots as u128
                + u128::from(external);
            (needed > u128::from(gpu.capacity_bytes)).then(|| MemoryViolation {
                gpu: gpu.id.clone(),
                request: request.id.clone(),
                required_bytes: needed.to_string(),
                capacity_bytes: gpu.capacity_bytes,
            })
        }));
    }
    let status = if trace.gpus.is_empty() {
        "unknown"
    } else if violations.is_empty() {
        "estimated_fit"
    } else {
        "exceeds_estimate"
    };
    Capacity { status, violations }
}

#[derive(Serialize)]
pub struct Totals {
    pub processed_tokens: u64,
    pub cached_tokens: u64,
    pub prefill_us: u64,
    pub queue_us: u64,
    pub makespan_us: u64,
    pub p95_latency_us: u64,
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub evidence: &'static str,
    pub hardware_verified: bool,
    pub trace_sha256: String,
    pub config_sha256: String,
    pub provenance: Provenance,
    pub config: Config,
    pub capacity: Capacity,
    pub totals: Totals,
    pub cache: Stats,
    pub requests: Vec<Outcome>,
    pub limitations: Vec<&'static str>,
}

struct Pending {
    request: usize,
    end: u64,
    lease: Option<usize>,
}
#[derive(Default)]
struct Slot {
    previous: Option<usize>,
    pending: Option<Pending>,
}

struct Simulation<'a> {
    trace: &'a Trace,
    cfg: &'a Config,
    slots: Vec<Slot>,
    cache: Cache,
    completed: Vec<usize>,
    outcomes: Vec<Outcome>,
}

impl Simulation<'_> {
    fn complete(&mut self, now: u64) -> Result<(), String> {
        for slot in &mut self.slots {
            if slot.pending.as_ref().is_none_or(|p| p.end > now) {
                continue;
            }
            let pending = slot.pending.take().unwrap();
            self.cache.release(pending.lease);
            self.cache.admit(
                pending.request,
                &self.trace.requests[pending.request],
                now,
                self.cfg,
            )?;
            slot.previous = Some(pending.request);
            self.completed.push(pending.request);
        }
        Ok(())
    }

    fn choice(&self, slot: usize, request: usize) -> Result<Choice, String> {
        let req = &self.trace.requests[request];
        let cached = self.slots[slot].previous.map_or(0, |id| {
            reusable(&self.trace.requests[id], req, self.cfg.rewind_tokens)
        });
        let source = if cached == 0 {
            Source::Cold
        } else {
            Source::Slot
        };
        self.cache.choose(
            self.trace,
            req,
            self.cfg,
            Choice::new(source, cached, 0, None, req)?,
        )
    }

    fn select(&self, request: usize) -> Result<(usize, Choice), String> {
        let mut free = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.pending.is_none())
            .map(|(i, _)| i);
        let first = free.next().ok_or("no free slot")?;
        let mut best = (first, self.choice(first, request)?);
        if self.cfg.dispatch == Dispatch::Fifo {
            return Ok(best);
        }
        for id in free {
            let candidate = self.choice(id, request)?;
            if candidate.better_than(&best.1) {
                best = (id, candidate);
            }
        }
        Ok(best)
    }

    fn reason(&self, request: usize, cached: usize) -> Option<&'static str> {
        let req = &self.trace.requests[request];
        if cached == req.prefix_ids.len() {
            return None;
        }
        if self.completed.is_empty() {
            return Some("cold_start");
        }
        let potential = self
            .completed
            .iter()
            .map(|id| reusable(&self.trace.requests[*id], req, self.cfg.rewind_tokens))
            .max()
            .unwrap_or(0);
        if potential > cached {
            return Some("not_retained");
        }
        let same_identity = self.completed.iter().any(|id| {
            let state = &self.trace.requests[*id];
            state.model == req.model && state.layout == req.layout
        });
        Some(if same_identity {
            "new_diverged_or_rewind_limited_prefix"
        } else {
            "identity_mismatch"
        })
    }

    fn start(&mut self, request: usize, now: u64) -> Result<(), String> {
        let (slot, choice) = self.select(request)?;
        let req = &self.trace.requests[request];
        let end = now
            .checked_add(choice.work)
            .and_then(|v| v.checked_add(req.decode_us))
            .ok_or("timeline overflow")?;
        self.cache.acquire(choice.entry, now);
        self.outcomes.push(Outcome {
            id: req.id.clone(),
            slot,
            start_us: now,
            end_us: end,
            queue_us: now - req.at_us,
            cached_tokens: choice.cached,
            processed_tokens: req.prefix_ids.len() - choice.cached,
            prefill_us: choice.work - choice.restore,
            restore_us: choice.restore,
            source: choice.source,
            uncached_reason: self.reason(request, choice.cached),
        });
        self.slots[slot].pending = Some(Pending {
            request,
            end,
            lease: choice.entry,
        });
        Ok(())
    }

    fn enqueue(&self, incoming: &mut usize, waiting: &mut VecDeque<usize>, now: u64) {
        while *incoming < self.trace.requests.len() && self.trace.requests[*incoming].at_us <= now {
            waiting.push_back(*incoming);
            *incoming += 1;
        }
    }

    fn dispatch(&mut self, waiting: &mut VecDeque<usize>, now: u64) -> Result<(), String> {
        while !waiting.is_empty() && self.slots.iter().any(|s| s.pending.is_none()) {
            self.start(waiting.pop_front().unwrap(), now)?;
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), String> {
        let mut incoming = 0;
        let mut waiting = VecDeque::new();
        let mut now = self.trace.requests[0].at_us;
        loop {
            self.complete(now)?;
            self.enqueue(&mut incoming, &mut waiting, now);
            self.dispatch(&mut waiting, now)?;
            let arrival = self.trace.requests.get(incoming).map(|r| r.at_us);
            let finish = self
                .slots
                .iter()
                .filter_map(|s| s.pending.as_ref().map(|p| p.end))
                .min();
            let Some(next) = arrival.into_iter().chain(finish).min() else {
                return Ok(());
            };
            now = next;
        }
    }
}

fn sum(mut values: impl Iterator<Item = u64>) -> Result<u64, String> {
    values.try_fold(0u64, |a, b| {
        a.checked_add(b).ok_or_else(|| "total overflow".into())
    })
}

fn totals(trace: &Trace, outcomes: &[Outcome]) -> Result<Totals, String> {
    let mut latencies: Vec<_> = outcomes
        .iter()
        .zip(&trace.requests)
        .map(|(o, r)| o.end_us - r.at_us)
        .collect();
    latencies.sort_unstable();
    Ok(Totals {
        processed_tokens: sum(outcomes.iter().map(|o| o.processed_tokens as u64))?,
        cached_tokens: sum(outcomes.iter().map(|o| o.cached_tokens as u64))?,
        prefill_us: sum(outcomes.iter().map(|o| o.prefill_us))?,
        queue_us: sum(outcomes.iter().map(|o| o.queue_us))?,
        makespan_us: outcomes.iter().map(|o| o.end_us).max().unwrap() - trace.requests[0].at_us,
        p95_latency_us: latencies[(latencies.len() * 95).div_ceil(100) - 1],
    })
}

pub fn replay(trace: &Trace, cfg: &Config) -> Result<Report, String> {
    trace.validate()?;
    cfg.validate()?;
    let mut sim = Simulation {
        trace,
        cfg,
        slots: (0..cfg.slots).map(|_| Slot::default()).collect(),
        cache: Cache::default(),
        completed: vec![],
        outcomes: vec![],
    };
    sim.run()?;
    let totals = totals(trace, &sim.outcomes)?;
    Ok(Report {
        version: 1,
        evidence: "cpu_replay_prediction",
        hardware_verified: false,
        trace_sha256: crate::fingerprint(trace)?,
        config_sha256: crate::fingerprint(cfg)?,
        provenance: trace.provenance.clone(),
        config: cfg.clone(),
        capacity: capacity(trace, cfg),
        totals,
        cache: sim.cache.stats,
        requests: sim.outcomes,
        limitations: vec![
            "declared linear prefill and restore costs",
            "whole snapshots and declared protection only",
            "no CUDA execution, GPU overlap, dynamic allocation or generated tokens",
            "GPU capacity is an input estimate, not physical-fit proof",
        ],
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Experiment {
    pub trace: Trace,
    pub candidates: Vec<Config>,
}

#[derive(Serialize)]
pub struct Comparison {
    pub version: u32,
    pub hardware_verified: bool,
    pub fastest_feasible_estimate: Option<String>,
    pub reports: Vec<Report>,
}

pub fn compare(experiment: &Experiment) -> Result<Comparison, String> {
    if experiment.candidates.is_empty() || experiment.candidates.len() > 64 {
        return Err("candidate count must be 1..64".into());
    }
    let mut names = HashSet::new();
    let mut reports = Vec::new();
    for cfg in &experiment.candidates {
        if !names.insert(&cfg.name) {
            return Err("candidate names must be unique".into());
        }
        reports.push(replay(&experiment.trace, cfg)?);
    }
    reports.sort_by_key(|r| {
        (
            r.totals.p95_latency_us,
            r.totals.makespan_us,
            r.config.name.clone(),
        )
    });
    let fastest = reports
        .iter()
        .find(|r| r.capacity.status == "estimated_fit")
        .map(|r| r.config.name.clone());
    Ok(Comparison {
        version: 1,
        hardware_verified: false,
        fastest_feasible_estimate: fastest,
        reports,
    })
}
