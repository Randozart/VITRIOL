//! Pure policy FSM: observes samples, emits actions. No side effects here -
//! execution lives in main/engine. Unit-tested in tests/policy.rs.

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    Full,
    Yielded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Yield,
    Restore,
    Note(String),
}

/// One observation tick.
#[derive(Debug, Clone)]
pub struct Sample {
    /// Free VRAM per GPU, MiB.
    pub free_mib_per_gpu: Vec<u64>,
    /// A compute process that is not the engine is resident.
    pub foreign_tenant: bool,
    /// Engine answers /health.
    pub engine_up: bool,
}

impl Sample {
    pub fn min_free(&self) -> u64 {
        self.free_mib_per_gpu.iter().copied().min().unwrap_or(u64::MAX)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Policy {
    pub phase: Phase,
    /// Unix secs when the current pressure/clear streak started.
    pub pressured_since: Option<i64>,
    pub clear_since: Option<i64>,
    /// Unix secs of the last Yield/Restore transition (0 = never).
    pub last_transition: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            phase: Phase::Full,
            pressured_since: None,
            clear_since: None,
            last_transition: 0,
        }
    }
}

fn dwell_ok(last_transition: i64, min_dwell_secs: u64, now: i64) -> bool {
    last_transition == 0 || now - last_transition >= min_dwell_secs as i64
}

/// Track a condition streak; true only when held for `hold_secs`.
/// A broken condition resets the streak.
fn check_streak(active: bool, since: &mut Option<i64>, hold_secs: u64, now: i64) -> bool {
    if !active {
        *since = None;
        return false;
    }
    let start = *since.get_or_insert(now);
    now - start >= hold_secs as i64
}

impl Policy {
    fn transition(&mut self, to: Phase, now: i64) {
        self.phase = to;
        self.last_transition = now;
        self.pressured_since = None;
        self.clear_since = None;
    }

    /// Feed one sample; returns actions to execute (possibly empty).
    /// Streaks reset when the condition breaks. Dwell guards transitions.
    pub fn decide(&mut self, s: &Sample, cfg: &Config, now: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        let dwell = dwell_ok(self.last_transition, cfg.min_dwell_secs, now);
        match self.phase {
            Phase::Full => {
                let pressured = s.engine_up
                    && s.foreign_tenant
                    && s.free_mib_per_gpu.iter().any(|&f| f < cfg.floor_free_mib);
                if check_streak(pressured, &mut self.pressured_since, cfg.confirm_secs, now) && dwell {
                    self.transition(Phase::Yielded, now);
                    actions.push(Action::Yield);
                }
            }
            Phase::Yielded => {
                let clear = !s.foreign_tenant
                    && !s.free_mib_per_gpu.is_empty()
                    && s.free_mib_per_gpu.iter().all(|&f| f >= cfg.ceiling_free_mib);
                if check_streak(clear, &mut self.clear_since, cfg.recover_secs, now) && dwell {
                    self.transition(Phase::Full, now);
                    actions.push(Action::Restore);
                }
            }
        }
        actions
    }
}
