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
    Caduceus,
}

impl LogSource {
    pub const ALL: [Self; 6] = [
        Self::Engine,
        Self::Service,
        Self::Autosave,
        Self::Slots,
        Self::Events,
        Self::Caduceus,
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
            Self::Caduceus => "Caduceus",
        }
    }
}

pub enum Menu {
    Actions(usize),
    Profiles(usize, Vec<Profile>),
    Caduceus(CaduceusMenu),
}

#[derive(Default)]
pub struct CaduceusMenu {
    pub selected: Option<String>,
    pub query: String,
    pub editing: bool,
}

impl CaduceusMenu {
    pub fn matching<'a>(
        &self,
        lanes: &'a [crate::caduceus::Lane],
    ) -> Vec<&'a crate::caduceus::Lane> {
        let query = self.query.to_ascii_lowercase();
        lanes
            .iter()
            .filter(|lane| query.is_empty() || lane.model.to_ascii_lowercase().contains(&query))
            .collect()
    }
}

pub struct App {
    pub cfg: Config,
    pub snapshot: Snapshot,
    pub tab: Tab,
    pub tab_hits: Vec<(Rect, Tab)>,
    pub caduceus_hit: Rect,
    pub decode_history: VecDeque<f64>,
    pub log_source: LogSource,
    pub log_sidebar: bool,
    pub log_slot: Option<u64>,
    pub log_route: Option<String>,
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
            caduceus_hit: Rect::default(),
            decode_history: VecDeque::new(),
            log_source: LogSource::Engine,
            log_sidebar: false,
            log_slot: None,
            log_route: None,
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

    pub fn click(&mut self, x: u16, y: u16) {
        let position = Position::new(x, y);
        if let Some((_, tab)) = self
            .tab_hits
            .iter()
            .find(|(area, _)| area.contains(position))
        {
            self.set_tab(*tab);
        } else if self.tab == Tab::Dashboard
            && self.menu.is_none()
            && self.caduceus_hit.contains(position)
        {
            self.open_caduceus();
        }
    }

    pub fn logs_visible(&self) -> bool {
        self.tab == Tab::Logs || self.log_sidebar
    }

    pub fn open_caduceus(&mut self) {
        let lanes = &self.snapshot.caduceus.lanes;
        let selected = lanes
            .iter()
            .find(|lane| lane.remaining.is_some())
            .or(lanes.first());
        self.menu = Some(Menu::Caduceus(CaduceusMenu {
            selected: selected.map(|lane| lane.model.clone()),
            ..Default::default()
        }));
    }

    pub fn cycle_caduceus(&mut self, step: isize) {
        let Some(Menu::Caduceus(menu)) = self.menu.as_mut() else {
            return;
        };
        let lanes = menu.matching(&self.snapshot.caduceus.lanes);
        let Some(first) = lanes.first() else { return };
        let next = lanes
            .iter()
            .position(|lane| Some(&lane.model) == menu.selected.as_ref())
            .map_or(first, |index| {
                &lanes[(index as isize + step).rem_euclid(lanes.len() as isize) as usize]
            });
        menu.selected = Some(next.model.clone());
    }

    pub fn update_caduceus_search(&mut self, key: crossterm::event::KeyCode) -> bool {
        use crossterm::event::KeyCode;
        let Some(Menu::Caduceus(menu)) = self.menu.as_mut() else {
            return false;
        };
        if !menu.editing {
            return false;
        }
        match key {
            KeyCode::Enter => menu.editing = false,
            KeyCode::Esc => {
                menu.editing = false;
                menu.query.clear();
            }
            KeyCode::Backspace => {
                menu.query.pop();
            }
            KeyCode::Char(c) => menu.query.push(c),
            _ => {}
        }
        let lanes = menu.matching(&self.snapshot.caduceus.lanes);
        if !lanes
            .iter()
            .any(|lane| Some(&lane.model) == menu.selected.as_ref())
        {
            menu.selected = lanes.first().map(|lane| lane.model.clone());
        }
        true
    }

    pub fn open_caduceus_logs(&mut self) {
        let route = match &self.menu {
            Some(Menu::Caduceus(menu)) => menu.selected.clone(),
            _ => return,
        };
        self.set_tab(Tab::Logs);
        self.set_log_source(LogSource::Caduceus);
        self.log_route = route;
        self.log_query.clear();
        self.paused_log = None;
        self.log_offset = 0;
    }

    pub fn clear_log_filter(&mut self) {
        self.log_query.clear();
        self.log_offset = 0;
        if self.log_source == LogSource::Caduceus && self.log_route.take().is_some() {
            self.paused_log = None;
        }
    }

    pub fn unpark_selected(&mut self, tx: &Sender<Event>) {
        let Some(Menu::Caduceus(CaduceusMenu {
            selected: Some(selected),
            ..
        })) = &self.menu
        else {
            return;
        };
        if self
            .snapshot
            .caduceus
            .lanes
            .iter()
            .any(|lane| &lane.model == selected && lane.remaining.is_some())
        {
            self.run_action(Action::Unpark(selected.clone()), tx);
        }
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
            LogSource::Caduceus => self.log_route.as_deref().map_or_else(
                || Cow::Borrowed(self.snapshot.logs.caduceus.as_slice()),
                |route| {
                    Cow::Owned(
                        self.snapshot
                            .logs
                            .caduceus
                            .iter()
                            .filter(|line| crate::caduceus::is_route_event(line, route))
                            .cloned()
                            .collect(),
                    )
                },
            ),
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
        if let Some(Menu::Caduceus(menu)) = self.menu.as_mut() {
            if menu.selected.is_none() {
                menu.selected = menu
                    .matching(&snapshot.caduceus.lanes)
                    .first()
                    .map(|lane| lane.model.clone());
            }
        }
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
            if !matches!(action, Action::Unpark(_) | Action::SyncCaduceus) {
                self.menu = None;
            }
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
