//! UI application state.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::model::Snapshot;

/// Maximum number of decode-t/s samples kept for the sparkline.
const SPARKLINE_CAP: usize = 120;

/// Top-level UI tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// Live service/GPU overview.
    Dashboard,
    /// Full btop-style GPU panel.
    Gpu,
    /// Live log tails.
    Logs,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 3] = [Tab::Dashboard, Tab::Gpu, Tab::Logs];

    /// Short label used in the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "DASHBOARD",
            Tab::Gpu => "GPU",
            Tab::Logs => "LOGS",
        }
    }
}

/// Which service log the LOGS tab is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    /// Gen (llama-server) log.
    Gen,
    /// Hermetis memory-server log.
    Hermetis,
    /// Embed (bge) server log.
    Embed,
}

impl LogSource {
    /// Short label for the LOGS panel title.
    pub fn label(self) -> &'static str {
        match self {
            LogSource::Gen => "GEN",
            LogSource::Hermetis => "HERMETIS",
            LogSource::Embed => "EMBED",
        }
    }
}

/// Application state held across the event loop.
pub struct App {
    /// Endpoint/log config.
    pub cfg: Config,
    /// Latest telemetry snapshot from the poller.
    pub snapshot: Snapshot,
    /// Decode t/s history for the sparkline, newest last.
    pub decode_history: VecDeque<f64>,
    /// Active tab.
    pub tab: Tab,
    /// Service log selected in the LOGS tab.
    pub log_source: LogSource,
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
            tab: Tab::Dashboard,
            log_source: LogSource::Gen,
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

    /// Advance to the next tab, wrapping.
    pub fn next_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.tab = Tab::ALL[(idx + 1) % Tab::ALL.len()];
    }

    /// Step back to the previous tab, wrapping.
    pub fn prev_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.tab = Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()];
    }

    /// Lines of the currently selected service log, oldest first.
    pub fn current_log_lines(&self) -> &[String] {
        match self.log_source {
            LogSource::Gen => &self.snapshot.logs.gen,
            LogSource::Hermetis => &self.snapshot.logs.hermetis,
            LogSource::Embed => &self.snapshot.logs.embed,
        }
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
