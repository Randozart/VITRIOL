//! Pure policy FSM: escalation ladder Full -> Relieved -> Resized -> Yielded
//! under sustained foreign-tenant pressure, walk-back on sustained recovery.
//! No side effects here - execution lives in main/engine. Unit-tested.

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Phase {
    /// No active mitigation.
    Full,
    /// Eager-sweep floor raised on the engine (/kv-policy).
    Relieved,
    /// Live context shrink applied (/ctx/resize).
    Resized,
    /// Engine restarted into the smaller profile.
    Yielded,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Raise the KV sweep floor + sweep once.
    Relieve(f32),
    /// Live-resize the engine context.
    ResizeTo(u32),
    /// Swap to the yield profile + restart the unit.
    Yield,
    /// Floor back to 0 (demand-driven eviction only).
    RelieveUndo,
    /// Resize back to the full window.
    ResizeUndo(u32),
    /// Swap back to the full profile + restart the unit.
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
    /// Unix secs when the current pressure streak started.
    #[serde(default)]
    pub press_since: Option<i64>,
    /// Unix secs when the current clear streak started.
    #[serde(default)]
    pub clear_since: Option<i64>,
    /// Unix secs of the last heavy transition (0 = never).
    pub last_transition: i64,
}

impl Default for Policy {
    fn default() -> Self {
        Policy {
            phase: Phase::Full,
            press_since: None,
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
    fn transition(&mut self, to: Phase, now: i64, heavy: bool) {
        self.phase = to;
        self.press_since = None;
        self.clear_since = None;
        if heavy {
            // heavy transitions arm the dwell clock
            self.last_transition = now;
        }
    }

    fn pressured(&self, s: &Sample, cfg: &Config) -> bool {
        s.engine_up
            && s.foreign_tenant
            && s.free_mib_per_gpu.iter().any(|&f| f < cfg.floor_free_mib)
    }

    fn cleared(&self, s: &Sample, cfg: &Config) -> bool {
        !s.foreign_tenant
            && !s.free_mib_per_gpu.is_empty()
            && s.free_mib_per_gpu.iter().all(|&f| f >= cfg.ceiling_free_mib)
    }

    /// Feed one sample; returns actions to execute (possibly empty).
    pub fn decide(&mut self, s: &Sample, cfg: &Config, now: i64) -> Vec<Action> {
        let press = self.pressured(s, cfg);
        let clear = self.cleared(s, cfg);
        match self.phase {
            Phase::Full => self.decide_full(press, cfg, now),
            Phase::Relieved => self.decide_relieved(press, clear, cfg, now),
            Phase::Resized => self.decide_resized(press, clear, cfg, now),
            Phase::Yielded => self.decide_yielded(clear, cfg, now),
        }
    }

    fn decide_full(&mut self, press: bool, cfg: &Config, now: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        if check_streak(press, &mut self.press_since, cfg.confirm_secs, now) {
            self.transition(Phase::Relieved, now, false);
            actions.push(Action::Relieve(cfg.relieve_floor));
        }
        actions
    }

    fn decide_relieved(&mut self, press: bool, clear: bool, cfg: &Config, now: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        if check_streak(clear, &mut self.clear_since, cfg.recover_secs, now) {
            self.transition(Phase::Full, now, false);
            actions.push(Action::RelieveUndo);
            return actions;
        }
        let escalate = check_streak(press, &mut self.press_since, cfg.resize_hold_secs, now)
            && dwell_ok(self.last_transition, cfg.min_dwell_secs, now);
        if escalate {
            if cfg.resize_enabled {
                self.transition(Phase::Resized, now, true);
                actions.push(Action::ResizeTo(cfg.resize_n_ctx));
            } else {
                self.transition(Phase::Yielded, now, true);
                actions.push(Action::Yield);
            }
        }
        actions
    }

    fn decide_resized(&mut self, press: bool, clear: bool, cfg: &Config, now: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        // streaks accumulate in parallel with dwell; the action fires when
        // both conditions hold. Yield escalation is guarded by its own
        // sustained-pressure hold (the resize already failed to help).
        let recovered = check_streak(clear, &mut self.clear_since, cfg.recover_secs, now);
        let dwell = dwell_ok(self.last_transition, cfg.min_dwell_secs, now);
        if recovered && dwell {
            self.transition(Phase::Full, now, true);
            actions.push(Action::ResizeUndo(cfg.full_n_ctx));
            actions.push(Action::RelieveUndo);
            return actions;
        }
        if check_streak(press, &mut self.press_since, cfg.yield_hold_secs, now) {
            self.transition(Phase::Yielded, now, true);
            actions.push(Action::Yield);
        }
        actions
    }

    fn decide_yielded(&mut self, clear: bool, cfg: &Config, now: i64) -> Vec<Action> {
        let mut actions = Vec::new();
        let recovered = check_streak(clear, &mut self.clear_since, cfg.recover_secs, now);
        let dwell = dwell_ok(self.last_transition, cfg.min_dwell_secs, now);
        if recovered && dwell {
            self.transition(Phase::Full, now, true);
            actions.push(Action::Restore);
        }
        actions
    }
}
