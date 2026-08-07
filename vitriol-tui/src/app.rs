//! UI application state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::control::{self, Action, Event};
use crate::model::Snapshot;
use crate::profile::{self, Profile};
use crate::search::{self, SearchHit};

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
    /// Hermetis memory: stats, recent stores, search.
    Hermetis,
    /// Subsystem diagnostics: Tria Prima services + alchemical layers.
    Subsystems,
    /// Profiles: edit the active config INI (form-style) + manage profiles.
    Profiles,
    /// Guide: scroll VITRIOL docs, provenance, and the Pymander corpus.
    Guide,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 8] = [
        Tab::Dashboard,
        Tab::Gpu,
        Tab::Logs,
        Tab::Controls,
        Tab::Hermetis,
        Tab::Subsystems,
        Tab::Profiles,
        Tab::Guide,
    ];

    /// Short label used in the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "DASHBOARD",
            Tab::Gpu => "GPU",
            Tab::Logs => "LOGS",
            Tab::Controls => "CONTROLS",
            Tab::Hermetis => "HERMETIS",
            Tab::Subsystems => "SUBSYSTEMS",
            Tab::Profiles => "PROFILES",
            Tab::Guide => "GUIDE",
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
    /// Search query buffer for the HERMETIS tab.
    pub search_query: String,
    /// Last Hermetis search hits.
    pub search_results: Vec<SearchHit>,
    /// Whether a search request is in flight.
    pub search_in_flight: bool,
    /// Loaded active config for the PROFILES tab.
    pub config_file: crate::config_edit::ConfigFile,
    /// Cursor into the PROFILES entry list.
    pub profile_selection: usize,
    /// Inline edit buffer (editing the selected entry's value when Some).
    pub profile_edit: Option<String>,
    /// Discovered guide docs for the GUIDE tab.
    pub guide_docs: Vec<crate::guide::Doc>,
    /// Cursor into the GUIDE index.
    pub guide_selection: usize,
    /// Scroll offset within the rendered guide body.
    pub guide_scroll: usize,
    /// Reader pane width at last draw (markdown wraps to this).
    pub guide_width: usize,
    /// When the previous tick was consumed, for per-tick hooks.
    last_tick: Instant,
}

impl App {
    /// Create empty app state. `history_cap` bounds the sparkline sample ring.
    pub fn new(cfg: Config, history_cap: usize) -> Self {
        let profiles = profile::discover(&cfg);
        let config_file = crate::config_edit::ConfigFile::load(&cfg);
        let guide_docs = crate::guide::discover(&cfg);
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
            search_query: String::new(),
            search_results: Vec::new(),
            search_in_flight: false,
            config_file,
            profile_selection: 0,
            profile_edit: None,
            guide_docs,
            guide_selection: 0,
            guide_scroll: 0,
            guide_width: 80,
            last_tick: Instant::now(),
        }
    }

    /// Append a character to the search query.
    pub fn type_search_char(&mut self, c: char) {
        self.search_query.push(c);
    }

    /// Remove the last character from the search query.
    pub fn backspace_search(&mut self) {
        self.search_query.pop();
    }

    /// Clear the search query.
    pub fn clear_search(&mut self) {
        self.search_query.clear();
    }

    /// Run the current search query, if non-empty and nothing in flight.
    pub fn run_search(&mut self, search_tx: &std::sync::mpsc::Sender<Vec<SearchHit>>) {
        let query = self.search_query.trim().to_string();
        if query.is_empty() || self.search_in_flight {
            return;
        }
        self.search_in_flight = true;
        search::spawn(
            self.cfg.clone(),
            self.cfg.project_id.clone(),
            query,
            search_tx.clone(),
        );
    }

    /// Fold a search-result batch into app state.
    pub fn apply_search_results(&mut self, results: Vec<SearchHit>) {
        self.search_results = results;
        self.search_in_flight = false;
    }

    /// Move the PROFILES cursor, wrapping, unless editing inline.
    pub fn profile_move(&mut self, delta: isize) {
        if self.profile_edit.is_some() {
            return;
        }
        let len = self.config_file.entries.len();
        if len == 0 {
            return;
        }
        let cur = self.profile_selection as isize;
        self.profile_selection = ((cur + delta).rem_euclid(len as isize)) as usize;
    }

    /// Begin inline-editing the selected entry's value.
    pub fn profile_edit_selected(&mut self) {
        let idx = self.profile_selection;
        if let Some(e) = self.config_file.entries.get(idx) {
            self.profile_edit = Some(e.value.clone());
        }
    }

    /// Append a character to the inline edit buffer.
    pub fn profile_type(&mut self, c: char) {
        if let Some(buf) = &mut self.profile_edit {
            buf.push(c);
        }
    }

    /// Remove the last character from the inline edit buffer.
    pub fn profile_backspace(&mut self) {
        if let Some(buf) = &mut self.profile_edit {
            buf.pop();
        }
    }

    /// Commit the inline edit (or cancel if the buffer is empty) and save.
    pub fn profile_commit(&mut self) -> Result<(), String> {
        let Some(buf) = self.profile_edit.take() else {
            return Ok(());
        };
        if buf.is_empty() {
            return Ok(());
        }
        let idx = self.profile_selection;
        let Some(entry) = self.config_file.entries.get(idx).cloned() else {
            return Ok(());
        };
        self.config_file.upsert(&entry.section, &entry.key, buf);
        self.config_file.save()
    }

    /// Abort the inline edit without saving.
    pub fn profile_cancel_edit(&mut self) {
        self.profile_edit = None;
    }

    /// Remove the selected entry from the config and save.
    pub fn profile_remove_selected(&mut self) -> Result<(), String> {
        let idx = self.profile_selection;
        let Some(entry) = self.config_file.entries.get(idx) else {
            return Ok(());
        };
        let section = entry.section.clone();
        let key = entry.key.clone();
        self.config_file.remove(&section, &key);
        self.profile_selection = self
            .profile_selection
            .saturating_sub(1)
            .min(self.config_file.entries.len().saturating_sub(1));
        self.config_file.save()
    }

    /// Reload the config file from disk (e.g. after an external edit).
    pub fn profile_reload(&mut self) {
        self.config_file = crate::config_edit::ConfigFile::load(&self.cfg);
        self.profile_edit = None;
    }

    /// Move the GUIDE index cursor, wrapping.
    pub fn guide_move(&mut self, delta: isize) {
        let len = self.guide_docs.len();
        if len == 0 {
            return;
        }
        let cur = self.guide_selection as isize;
        self.guide_selection = ((cur + delta).rem_euclid(len as isize)) as usize;
        self.guide_scroll = 0;
    }

    /// Scroll the rendered guide body within `height` visible lines.
    pub fn guide_scroll_lines(&mut self, delta: isize, height: usize) {
        let Some(doc) = self.guide_docs.get(self.guide_selection) else {
            return;
        };
        let text = std::fs::read_to_string(&doc.path).unwrap_or_default();
        let max = crate::markdown::render(&text, self.guide_width.max(1))
            .len()
            .saturating_sub(height);
        let cur = self.guide_scroll as isize;
        self.guide_scroll = ((cur + delta).max(0)).min(max as isize) as usize;
    }

    /// The currently selected guide doc's rendered, wrapped lines.
    pub fn guide_body(&self) -> Vec<ratatui::text::Line<'static>> {
        let Some(doc) = self.guide_docs.get(self.guide_selection) else {
            return Vec::new();
        };
        let text = std::fs::read_to_string(&doc.path).unwrap_or_default();
        crate::markdown::render(&text, self.guide_width.max(1))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// No-auto-launch invariant (2026-08-07): constructing the app must not start
    /// the stack. It is pure monitoring until the user triggers an explicit
    /// control action from the CONTROLS tab. This guards against a later phase
    /// (sweep progress, auto-restart) silently spawning processes at init.
    #[test]
    fn fresh_app_does_not_launch_stack() {
        let cfg = Config::from_env();
        let app = App::new(cfg, 120);
        assert!(!app.control_running);
        assert!(app.control_action.is_empty());
        assert!(app.control_step.is_empty());
        assert!(app.control_log.is_empty());
        assert_eq!(app.tab, Tab::Dashboard);
    }

    /// Tab registry stays consistent with the labels rendered in the tab bar.
    #[test]
    fn tab_all_matches_labels() {
        assert_eq!(Tab::ALL.len(), 8);
        for tab in Tab::ALL {
            assert!(!tab.label().is_empty());
        }
        assert_eq!(Tab::ALL[0], Tab::Dashboard);
        assert_eq!(Tab::ALL[Tab::ALL.len() - 1], Tab::Guide);
    }
}
