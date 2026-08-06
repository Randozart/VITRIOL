//! UI application state.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::model::Snapshot;

/// Maximum number of decode-t/s samples kept for the sparkline.
const SPARKLINE_CAP: usize = 120;

/// Application state held across the event loop.
pub struct App {
    /// Endpoint/log config.
    pub cfg: Config,
    /// Latest telemetry snapshot from the poller.
    pub snapshot: Snapshot,
    /// Decode t/s history for the sparkline, newest last.
    pub decode_history: VecDeque<f64>,
    /// When the previous tick was consumed, for per-tick hooks.
    last_tick: Instant,
}

impl App {
    /// Create empty app state. `history_cap` bounds the sparkline sample ring.
    pub fn new(cfg: Config, history_cap: usize) -> Self {
        Self {
            cfg,
            snapshot: Snapshot::default(),
            decode_history: VecDeque::with_capacity(history_cap),
            last_tick: Instant::now(),
        }
    }

    /// Fold a fresh snapshot into app state, updating the decode history.
    pub fn apply_snapshot(&mut self, snap: Snapshot) {
        let t_s = snap.gen.decode_t_s;
        self.decode_history.push_back(t_s);
        if self.decode_history.len() > SPARKLINE_CAP {
            self.decode_history.pop_front();
        }
        self.snapshot = snap;
    }

    /// Whether a full tick interval has elapsed since the last tick marker.
    pub fn should_tick(&mut self, interval: Duration) -> bool {
        let now = Instant::now();
        if now.duration_since(self.last_tick) >= interval {
            self.last_tick = now;
            true
        } else {
            false
        }
    }
}
