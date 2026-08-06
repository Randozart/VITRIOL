//! UI application state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::control::{self, Action, Event};
use crate::model::Snapshot;
use crate::profile::{self, Profile};

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
    /// Stack control: start/stop/restart, doctor, profile load.
    Controls,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 4] = [Tab::Dashboard, Tab::Gpu, Tab::Logs, Tab::Controls];

    /// Short label used in the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "DASHBOARD",
            Tab::Gpu => "GPU",
            Tab::Logs => "LOGS",
            Tab::Controls => "CONTROLS",
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
    /// Discovered launch profiles for the CONTROLS tab.
    pub profiles: Vec<Profile>,
    /// Cursor into the CONTROLS action list.
    pub selected_action: usize,
    /// Whether a control action is currently running.
    pub control_running: bool,
    /// Label of the running action.
    pub control_action: String,
    /// Label of the running step.
    pub control_step: String,
    /// Control output log ring, newest last.
    pub control_log: VecDeque<String>,
    /// Shared abort flag for the control executor.
    pub control_abort: Arc<AtomicBool>,
    /// When the previous tick was consumed, for per-tick hooks.
    last_tick: Instant,
}

impl App {
    /// Create empty app state. `history_cap` bounds the sparkline sample ring.
    pub fn new(cfg: Config, history_cap: usize) -> Self {
        let profiles = profile::discover(&cfg);
        Self {
            cfg,
            snapshot: Snapshot::default(),
            decode_history: VecDeque::with_capacity(history_cap),
            tab: Tab::Dashboard,
            log_source: LogSource::Gen,
            profiles,
            selected_action: 0,
            control_running: false,
            control_action: String::new(),
            control_step: String::new(),
            control_log: VecDeque::with_capacity(200),
            control_abort: Arc::new(AtomicBool::new(false)),
            last_tick: Instant::now(),
        }
    }

    /// The CONTROLS action list.
    pub fn actions(&self) -> Vec<Action> {
        Action::all(&self.profiles)
    }

    /// Start the currently selected control action, if none is running.
    pub fn run_selected_action(&mut self, ctrl_tx: &std::sync::mpsc::Sender<Event>) {
        if self.control_running {
            self.push_control_line("action already running".into());
            return;
        }
        let actions = self.actions();
        let Some(action) = actions.get(self.selected_action).cloned() else {
            return;
        };
        self.control_running = true;
        self.control_action = action.label();
        self.control_abort.store(false, Ordering::Relaxed);
        control::spawn(
            action,
            &self.cfg,
            ctrl_tx.clone(),
            Arc::clone(&self.control_abort),
        );
    }

    /// Request abort of the running control action.
    pub fn abort_control(&mut self) {
        if self.control_running {
            self.control_abort.store(true, Ordering::Relaxed);
            self.push_control_line("abort requested".into());
        }
    }

    /// Move the CONTROLS cursor by `delta`, clamping to the action list.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.actions().len();
        if len == 0 {
            return;
        }
        let cur = self.selected_action as isize;
        self.selected_action = (cur + delta).clamp(0, len as isize - 1) as usize;
    }

    /// Fold a control-thread event into app state.
    pub fn apply_control_event(&mut self, event: Event) {
        match event {
            Event::Started(action) => {
                self.control_action = action;
            }
            Event::StepStarted(step) => {
                self.control_step = step.clone();
                self.push_control_line(format!("▸ {step}"));
            }
            Event::Line(line) => {
                self.push_control_line(line);
            }
            Event::Done(ok) => {
                self.control_running = false;
                self.control_step.clear();
                self.control_abort.store(false, Ordering::Relaxed);
                let verdict = if ok { "✓ done" } else { "✗ failed" };
                self.push_control_line(format!("{verdict}: {}", self.control_action));
            }
        }
    }

    /// Append a control log line, capping the ring.
    pub fn push_control_line(&mut self, line: String) {
        if self.control_log.len() == 200 {
            self.control_log.pop_front();
        }
        self.control_log.push_back(line);
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
