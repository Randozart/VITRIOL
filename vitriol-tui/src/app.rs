//! UI state for the Dashboard and Logs.

use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::Sender;

use ratatui::layout::{Position, Rect};

use crate::config::Config;
use crate::control::{self, Action, Event};
use crate::model::{SlotSnapshot, Snapshot};
use crate::profile::Profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Dashboard,
    Logs,
}

impl Tab {
    pub const ALL: [Self; 2] = [Self::Dashboard, Self::Logs];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dashboard => "Dashboard",
            Self::Logs => "Logs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    Engine,
    Service,
    Autosave,
    Slots,
    Events,
}

impl LogSource {
    pub const ALL: [Self; 5] = [
        Self::Engine,
        Self::Service,
        Self::Autosave,
        Self::Slots,
        Self::Events,
    ];

    pub fn is_slot_scoped(self) -> bool {
        matches!(self, Self::Slots | Self::Events)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Engine => "Engine",
            Self::Service => "Service",
            Self::Autosave => "Autosave",
            Self::Slots => "Slots",
            Self::Events => "Events",
        }
    }
}

pub enum Menu {
    Actions(usize),
    Profiles(usize, Vec<Profile>),
}

pub struct App {
    pub cfg: Config,
    pub snapshot: Snapshot,
    pub tab: Tab,
    pub tab_hits: Vec<(Rect, Tab)>,
    pub decode_history: VecDeque<f64>,
    pub log_source: LogSource,
    pub log_sidebar: bool,
    pub log_slot: Option<u64>,
    pub logs_verbose: bool,
    pub log_query: String,
    pub search_open: bool,
    pub log_offset: usize,
    pub paused_log: Option<Vec<String>>,
    pub menu: Option<Menu>,
    pub action_running: bool,
    pub action_status: Option<(bool, String)>,
    pub action_log: VecDeque<String>,
    slot_history: Vec<(u64, String)>,
    prev_slots: HashMap<u64, (bool, Option<u64>)>,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        Self {
            cfg,
            snapshot: Snapshot::default(),
            tab: Tab::Dashboard,
            tab_hits: Vec::new(),
            decode_history: VecDeque::new(),
            log_source: LogSource::Engine,
            log_sidebar: false,
            log_slot: None,
            logs_verbose: false,
            log_query: String::new(),
            search_open: false,
            log_offset: 0,
            paused_log: None,
            menu: None,
            action_running: false,
            action_status: None,
            action_log: VecDeque::new(),
            slot_history: Vec::new(),
            prev_slots: HashMap::new(),
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.menu = None;
        self.search_open = false;
    }

    pub fn next_tab(&mut self) {
        self.set_tab(match self.tab {
            Tab::Dashboard => Tab::Logs,
            Tab::Logs => Tab::Dashboard,
        });
    }

    pub fn tab_click(&self, x: u16, y: u16) -> Option<Tab> {
        self.tab_hits
            .iter()
            .find(|(area, _)| area.contains(Position::new(x, y)))
            .map(|(_, tab)| *tab)
    }

    pub fn logs_visible(&self) -> bool {
        self.tab == Tab::Logs || self.log_sidebar
    }

    pub fn toggle_log_sidebar(&mut self) {
        self.log_sidebar = !self.log_sidebar;
        if !self.log_sidebar {
            self.search_open = false;
        }
    }

    pub fn cycle_log_slot(&mut self, step: isize) {
        if !self.log_source.is_slot_scoped() {
            self.set_log_source(LogSource::Slots);
        }
        let mut ids: BTreeSet<u64> = self.snapshot.gen.slots.iter().map(|slot| slot.id).collect();
        if self.log_source == LogSource::Slots {
            ids.extend(self.snapshot.logs.slot_tails.keys().copied());
        } else {
            ids.extend(self.slot_history.iter().map(|(slot, _)| *slot));
        }
        ids.extend(self.log_slot);
        let options: Vec<Option<u64>> = std::iter::once(None)
            .chain(ids.into_iter().map(Some))
            .collect();
        let current = options
            .iter()
            .position(|slot| *slot == self.log_slot)
            .unwrap_or(0);
        self.log_slot =
            options[(current as isize + step).rem_euclid(options.len() as isize) as usize];
        self.paused_log = None;
        self.log_offset = 0;
    }

    pub fn set_log_source(&mut self, source: LogSource) {
        if self.log_source != source {
            self.log_source = source;
            self.paused_log = None;
            self.log_offset = 0;
        }
    }

    pub fn cycle_log_source(&mut self, step: isize) {
        let order = LogSource::ALL;
        let index = order
            .iter()
            .position(|source| *source == self.log_source)
            .unwrap_or(0);
        self.set_log_source(
            order[(index as isize + step).rem_euclid(order.len() as isize) as usize],
        );
    }

    pub fn log_lines(&self) -> Cow<'_, [String]> {
        if let Some(lines) = &self.paused_log {
            return Cow::Borrowed(lines);
        }
        match self.log_source {
            LogSource::Engine => Cow::Borrowed(&self.snapshot.logs.gen),
            LogSource::Service if !self.action_log.is_empty() => Cow::Owned(
                self.snapshot
                    .logs
                    .journal
                    .iter()
                    .chain(&self.action_log)
                    .cloned()
                    .collect(),
            ),
            LogSource::Service => Cow::Borrowed(&self.snapshot.logs.journal),
            LogSource::Autosave => Cow::Borrowed(&self.snapshot.logs.autosave),
            LogSource::Slots => Cow::Borrowed(self.log_slot.map_or_else(
                || self.snapshot.logs.slot_entries.as_slice(),
                |id| {
                    self.snapshot
                        .logs
                        .slot_tails
                        .get(&id)
                        .map_or(&[], Vec::as_slice)
                },
            )),
            LogSource::Events => Cow::Owned(
                self.slot_history
                    .iter()
                    .filter(|(id, _)| self.log_slot.is_none_or(|scope| scope == *id))
                    .map(|(_, line)| line.clone())
                    .collect(),
            ),
        }
    }

    pub fn toggle_pause(&mut self) {
        if self.paused_log.is_some() {
            self.paused_log = None;
        } else {
            self.paused_log = Some(self.log_lines().into_owned());
        }
        self.log_offset = 0;
    }

    pub fn apply_snapshot(&mut self, snapshot: Snapshot) {
        if !snapshot.gen.up || snapshot.gen.model_pid != self.snapshot.gen.model_pid {
            self.decode_history.clear();
            self.prev_slots.clear();
        }
        if snapshot.gen.up {
            self.decode_history.push_back(snapshot.gen.decode_speed);
            if self.decode_history.len() > 120 {
                self.decode_history.pop_front();
            }
        }
        self.observe_slots(&snapshot.gen.slots);
        self.snapshot = snapshot;
    }

    fn observe_slots(&mut self, slots: &[SlotSnapshot]) {
        for slot in slots {
            let current = (slot.is_processing, slot.id_task);
            let transition = match self.prev_slots.insert(slot.id, current) {
                Some((true, _)) if !current.0 => "released",
                Some((true, task)) if task != current.1 => "new task",
                Some((false, _)) | None if current.0 => "acquired",
                _ => continue,
            };
            let filled = slot
                .filled
                .map_or_else(String::new, |value| format!(" · {value} filled"));
            self.slot_history.push((
                slot.id,
                format!("{} · slot {} {transition}{filled}", time_hm(), slot.id),
            ));
            if self.slot_history.len() > 120 {
                self.slot_history.remove(0);
            }
        }
    }

    pub fn run_action(&mut self, action: Action, tx: &Sender<Event>) {
        if !self.action_running {
            self.menu = None;
            self.action_running = true;
            self.action_status = None;
            control::spawn(action, &self.cfg, tx.clone());
        }
    }

    pub fn finish_action(&mut self, event: Event) {
        let message = event
            .lines
            .last()
            .cloned()
            .unwrap_or_else(|| "No output".into());
        self.action_status = Some((event.ok, message));
        for line in event.lines {
            self.action_log.push_back(line);
            if self.action_log.len() > 160 {
                self.action_log.pop_front();
            }
        }
        self.action_running = false;
    }
}

fn time_hm() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    format!("{:02}:{:02} UTC", seconds / 3600 % 24, seconds / 60 % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_filter_is_shared_by_both_views_and_keeps_slot_logs_separate() {
        let mut app = App::new(Config::from_env());
        app.snapshot.logs.slot_entries = vec!["zero".into(), "one".into()];
        app.snapshot.logs.slot_tails.insert(0, vec!["zero".into()]);
        app.snapshot.logs.slot_tails.insert(1, vec!["one".into()]);
        app.toggle_log_sidebar();
        app.cycle_log_slot(1);
        assert_eq!(app.log_lines().as_ref(), ["zero"]);
        app.cycle_log_slot(1);
        assert_eq!(app.log_lines().as_ref(), ["one"]);
        app.toggle_pause();
        app.next_tab();
        assert_eq!(app.log_slot, Some(1));
        assert_eq!(app.log_lines().as_ref(), ["one"]);
        app.set_log_source(LogSource::Events);
        assert!(app.log_lines().is_empty());
    }

    #[test]
    fn pause_freezes_one_source_while_latest_data_keeps_arriving() {
        let mut app = App::new(Config::from_env());
        app.snapshot.logs.gen.push("first".into());
        app.toggle_pause();
        app.snapshot.logs.gen.push("second".into());
        assert_eq!(app.log_lines().as_ref(), ["first"]);
        app.set_log_source(LogSource::Service);
        app.set_log_source(LogSource::Engine);
        assert_eq!(app.log_lines().as_ref(), ["first", "second"]);
    }
}
