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
    /// Officina: the model-surgery workshop REPL (Alka / SPQL).
    Officina,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 9] = [
        Tab::Dashboard,
        Tab::Gpu,
        Tab::Logs,
        Tab::Controls,
        Tab::Hermetis,
        Tab::Subsystems,
        Tab::Profiles,
        Tab::Guide,
        Tab::Officina,
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
            Tab::Officina => "OFFICINA",
        }
    }
}

/// Which PROFILES pane has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileFocus {
    /// The active-config entry list (editable rows).
    Config,
    /// The profile list (save/load/delete targets).
    List,
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
    /// The action that most recently finished (consumed by the Done handler so
    /// a successful sweep+save can auto-select its `<name>-swept` winner).
    pub finished_action: Option<Action>,
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
    /// Which PROFILES pane has focus: the active-config rows or the profile list.
    pub profile_focus: ProfileFocus,
    /// Cursor into the profile list pane.
    pub profile_list_selection: usize,
    /// Save-as profile name input buffer (Some = prompt active).
    pub profile_prompt: Option<String>,
    /// PROFILES-tab selected profile: Start/Restart apply its knobs as CLI
    /// overrides (flags-only — the active config file is left untouched).
    /// 2026-08-08: Start no longer auto-launches a loaded profile; the selected
    /// profile is the Start target, chosen explicitly with `t`.
    pub selected_profile: Option<String>,
    /// Discovered guide docs for the GUIDE tab.
    pub guide_docs: Vec<crate::guide::Doc>,
    /// Cursor into the GUIDE index.
    pub guide_selection: usize,
    /// Scroll offset within the rendered guide body.
    pub guide_scroll: usize,
    /// Reader pane width at last draw (markdown wraps to this).
    pub guide_width: usize,
    /// The Officina REPL session (OFFICINA tab).
    pub officina: crate::officina::Officina,
    /// Loaded Ascensus secrets for the SUBSYSTEMS tab.
    pub ascensus: crate::secrets::Secrets,
    /// Cursor into the SUBSYSTEMS row list.
    pub subsystem_selection: usize,
    /// Active Ascensus key/model editor (Some = editing).
    pub ascensus_edit: Option<AscensusEdit>,
    /// When the previous tick was consumed, for per-tick hooks.
    last_tick: Instant,
}

/// The ASCENSUS editor: key + model buffers; `field` is true when editing the
/// key, false when editing the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AscensusEdit {
    /// API key input buffer.
    pub api_key: String,
    /// Model input buffer.
    pub model: String,
    /// True = editing the API key field, false = editing the model field.
    pub key_field: bool,
}

impl App {
    /// Create empty app state. `history_cap` bounds the sparkline sample ring.
    pub fn new(cfg: Config, history_cap: usize) -> Self {
        let profiles = profile::discover(&cfg);
        let config_file = crate::config_edit::ConfigFile::load(&cfg);
        let guide_docs = crate::guide::discover(&cfg);
        let ascensus = crate::secrets::Secrets::load(&cfg.secrets_path());
        let officina = crate::officina::Officina::new(&cfg.home_dir);
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
            finished_action: None,
            search_query: String::new(),
            search_results: Vec::new(),
            search_in_flight: false,
            config_file,
            profile_selection: 0,
            profile_edit: None,
            profile_focus: ProfileFocus::Config,
            profile_list_selection: 0,
            profile_prompt: None,
            selected_profile: None,
            guide_docs,
            guide_selection: 0,
            guide_scroll: 0,
            guide_width: 80,
            officina,
            ascensus,
            subsystem_selection: 0,
            ascensus_edit: None,
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

    /// Toggle PROFILES pane focus (config rows <-> profile list).
    pub fn profile_pane_toggle(&mut self) {
        self.profile_focus = match self.profile_focus {
            ProfileFocus::Config => ProfileFocus::List,
            ProfileFocus::List => ProfileFocus::Config,
        };
        self.profile_edit = None;
    }

    /// Move the profile-list cursor, wrapping.
    pub fn profile_list_move(&mut self, delta: isize) {
        if self.profile_prompt.is_some() {
            return;
        }
        let len = self.profiles.len();
        if len == 0 {
            return;
        }
        let cur = self.profile_list_selection as isize;
        self.profile_list_selection = ((cur + delta).rem_euclid(len as isize)) as usize;
    }

    /// Begin the save-as-profile name prompt.
    pub fn profile_save_start(&mut self) {
        self.profile_prompt = Some(String::new());
    }

    /// Append a character to the save-as name buffer.
    pub fn profile_save_type(&mut self, c: char) {
        if let Some(buf) = &mut self.profile_prompt {
            buf.push(c);
        }
    }

    /// Remove the last character from the save-as name buffer.
    pub fn profile_save_backspace(&mut self) {
        if let Some(buf) = &mut self.profile_prompt {
            buf.pop();
        }
    }

    /// Abort the save-as prompt.
    pub fn profile_save_cancel(&mut self) {
        self.profile_prompt = None;
    }

    /// Commit the save-as prompt: write the active config as a new profile.
    pub fn profile_save_commit(&mut self) -> Result<(), String> {
        let Some(name) = self.profile_prompt.clone() else {
            return Ok(());
        };
        let name = name.trim().to_string();
        if !valid_profile_name(&name) {
            return Err(format!(
                "invalid profile name '{name}' (letters, numbers, hyphens, underscores)"
            ));
        }
        if self.profiles.iter().any(|p| p.name == name) {
            return Err(format!("profile '{name}' already exists"));
        }
        self.profile_prompt = None;
        let dir = self.cfg.installed_profiles_dir().join(&name);
        std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        let config_text = crate::config_edit::render_entries(&self.config_file.entries);
        std::fs::write(dir.join("config"), config_text)
            .map_err(|e| format!("write config: {e}"))?;
        let meta = format!(
            "name={name}\ndescription={}\ncreated={}\n",
            chrono_stamp(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        std::fs::write(dir.join("meta"), meta).map_err(|e| format!("write meta: {e}"))?;
        self.profile_reload_list();
        Ok(())
    }

    /// Load the selected profile's config into the active config (no restart).
    pub fn profile_load_selected(&mut self) -> Result<(), String> {
        let Some(profile) = self.profiles.get(self.profile_list_selection) else {
            return Ok(());
        };
        let src = match profile.source {
            crate::profile::ProfileSource::Installed => {
                self.cfg.installed_profiles_dir().join(&profile.name)
            }
            crate::profile::ProfileSource::Bundled => {
                self.cfg.bundled_profiles_dir().join(&profile.name)
            }
        };
        let text = std::fs::read_to_string(src.join("config"))
            .map_err(|e| format!("read {}: {e}", src.join("config").display()))?;
        let path = self.cfg.home_dir.join(".vitriol").join("config");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
        }
        crate::config_edit::atomic_write_path(&path, &text)?;
        self.config_file = crate::config_edit::ConfigFile::load(&self.cfg);
        Ok(())
    }

    /// Delete the selected INSTALLED profile (bundled profiles are protected).
    pub fn profile_delete_selected(&mut self) -> Result<(), String> {
        let Some(profile) = self.profiles.get(self.profile_list_selection) else {
            return Ok(());
        };
        if profile.source != crate::profile::ProfileSource::Installed {
            return Err(format!("'{}' is bundled — cannot delete", profile.name));
        }
        let dir = self.cfg.installed_profiles_dir().join(&profile.name);
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
        self.profile_list_selection = self
            .profile_list_selection
            .saturating_sub(1)
            .min(self.profiles.len().saturating_sub(1));
        self.profile_reload_list();
        Ok(())
    }

    /// Re-discover the profile list after a save/delete.
    pub fn profile_reload_list(&mut self) {
        self.profiles = crate::profile::discover(&self.cfg);
    }

    /// Select the profile at the cursor as the Start target (flags-only — no
    /// config write, no launch). None clears the selection.
    pub fn select_profile(&mut self, name: Option<String>) {
        self.selected_profile = name;
    }

    /// Overwrite the selected INSTALLED profile with the current active config.
    /// Bundled profiles are protected; the selection stays valid afterwards.
    pub fn profile_overwrite_selected(&mut self) -> Result<(), String> {
        let Some(profile) = self.profiles.get(self.profile_list_selection) else {
            return Ok(());
        };
        if profile.source != crate::profile::ProfileSource::Installed {
            return Err(format!("'{}' is bundled — cannot overwrite", profile.name));
        }
        let dir = self.cfg.installed_profiles_dir().join(&profile.name);
        let config_text = crate::config_edit::render_entries(&self.config_file.entries);
        crate::config_edit::atomic_write_path(&dir.join("config"), &config_text)?;
        self.profile_reload_list();
        Ok(())
    }

    /// Run the Spagyric sweep with save on the selected profile.
    pub fn profile_sweep_selected(&self) -> Option<Action> {
        let profile = self.profiles.get(self.profile_list_selection)?;
        Some(Action::SweepAndSave(profile.name.clone()))
    }

    /// After a successful sweep+save, the winner lives at `<name>-swept`.
    /// Reload the list and make it the selected Start target.
    pub fn profile_select_sweep_winner(&mut self, name: &str) {
        let winner = format!("{name}-swept");
        self.profile_reload_list();
        if let Some(idx) = self.profiles.iter().position(|p| p.name == winner) {
            self.profile_list_selection = idx;
            self.select_profile(Some(winner.clone()));
            self.profile_focus = ProfileFocus::List;
            self.push_control_line(format!(
                "✓ sweep winner '{winner}' selected — CONTROLS ▸ Start"
            ));
        } else {
            self.push_control_line(format!(
                "✗ sweep finished but '{winner}' not found (profile write failed?)"
            ));
        }
    }

    /// Move the SUBSYSTEMS row cursor, wrapping.
    pub fn subsystem_move(&mut self, delta: isize) {
        let len = crate::subsystems::rows_len(&self.cfg, &self.snapshot);
        if len == 0 {
            return;
        }
        let cur = self.subsystem_selection as isize;
        self.subsystem_selection = ((cur + delta).rem_euclid(len as isize)) as usize;
    }

    /// Build the Officina op context from the current app state.
    pub fn officina_ctx(&self) -> crate::officina::OpCtx<'_> {
        let model_path = self
            .config_file
            .entries
            .iter()
            .find(|e| e.section == "model" && e.key == "path")
            .map(|e| std::path::PathBuf::from(&e.value));
        let profile = self
            .profiles
            .get(self.profile_list_selection)
            .map(|p| p.name.clone());
        crate::officina::OpCtx {
            cfg: &self.cfg,
            snap: &self.snapshot,
            model_path,
            profile,
        }
    }

    /// Begin the ASCENSUS key/model editor, seeded from the current secrets.
    pub fn ascensus_edit_start(&mut self) {
        if self.ascensus_edit.is_none() {
            self.ascensus_edit = Some(AscensusEdit {
                api_key: self.ascensus.api_key.clone(),
                model: self.ascensus.model.clone(),
                key_field: true,
            });
        }
    }

    /// Append a character to the currently edited ASCENSUS field.
    pub fn ascensus_edit_type(&mut self, c: char) {
        if let Some(edit) = &mut self.ascensus_edit {
            if edit.key_field {
                edit.api_key.push(c);
            } else {
                edit.model.push(c);
            }
        }
    }

    /// Remove the last character from the currently edited ASCENSUS field.
    pub fn ascensus_edit_backspace(&mut self) {
        if let Some(edit) = &mut self.ascensus_edit {
            if edit.key_field {
                edit.api_key.pop();
            } else {
                edit.model.pop();
            }
        }
    }

    /// Toggle which ASCENSUS field is being edited (key <-> model).
    pub fn ascensus_edit_toggle_field(&mut self) {
        if let Some(edit) = &mut self.ascensus_edit {
            edit.key_field = !edit.key_field;
        }
    }

    /// Advance the ASCENSUS editor: on the key field -> model field; on the
    /// model field -> save and close.
    pub fn ascensus_edit_next(&mut self) -> Result<(), String> {
        let Some(mut edit) = self.ascensus_edit.clone() else {
            return Ok(());
        };
        if edit.key_field {
            edit.key_field = false;
            self.ascensus_edit = Some(edit);
            return Ok(());
        }
        self.ascensus_commit(edit)
    }

    /// Commit the ASCENSUS editor: write secrets to `~/.vitriol/secrets` (0600).
    pub fn ascensus_commit(&mut self, edit: AscensusEdit) -> Result<(), String> {
        let s = crate::secrets::Secrets {
            api_key: edit.api_key.trim().to_string(),
            model: edit.model.trim().to_string(),
        };
        let path = self.cfg.secrets_path();
        s.save(&path)?;
        self.ascensus = s;
        self.ascensus_edit = None;
        Ok(())
    }

    /// Abort the ASCENSUS editor without saving.
    pub fn ascensus_edit_cancel(&mut self) {
        self.ascensus_edit = None;
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
        Action::all(&self.profiles, self.selected_profile.as_deref())
    }

    /// Run a specific control action (from CONTROLS Enter or a PROFILES key).
    pub fn run_action(&mut self, action: Action, ctrl_tx: &std::sync::mpsc::Sender<Event>) {
        if self.control_running {
            self.push_control_line("action already running".into());
            return;
        }
        self.control_running = true;
        self.control_action = action.label();
        self.finished_action = Some(action.clone());
        self.control_abort.store(false, Ordering::Relaxed);
        control::spawn(
            action,
            &self.cfg,
            ctrl_tx.clone(),
            Arc::clone(&self.control_abort),
        );
    }

    /// Start the currently selected control action, if none is running.
    pub fn run_selected_action(&mut self, ctrl_tx: &std::sync::mpsc::Sender<Event>) {
        let actions = self.actions();
        let Some(action) = actions.get(self.selected_action).cloned() else {
            return;
        };
        self.run_action(action, ctrl_tx);
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
                let sweep_name = self.finished_action.clone().and_then(|a| match a {
                    Action::SweepAndSave(name) => Some(name),
                    _ => None,
                });
                self.finished_action = None;
                if ok {
                    if let Some(name) = sweep_name {
                        self.profile_select_sweep_winner(&name);
                    }
                }
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

/// Whether `name` is a valid profile name (alnum, hyphen, underscore).
fn valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Current UTC date as `YYYY-MM-DD HH:MM` for profile meta descriptions.
fn chrono_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm) = (rem / 3600, (rem % 3600) / 60);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
}

/// Convert days since epoch to (year, month, day) — Howard Hinnant's civil
/// calendar algorithm (public domain).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
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
        assert_eq!(Tab::ALL.len(), 9);
        for tab in Tab::ALL {
            assert!(!tab.label().is_empty());
        }
        assert_eq!(Tab::ALL[0], Tab::Dashboard);
        assert_eq!(Tab::ALL[Tab::ALL.len() - 1], Tab::Officina);
    }

    #[test]
    fn profile_name_validation() {
        assert!(valid_profile_name("mellum2"));
        assert!(valid_profile_name("deep-seek_v2"));
        assert!(!valid_profile_name(""));
        assert!(!valid_profile_name("bad name"));
        assert!(!valid_profile_name("bad/name"));
    }

    #[test]
    fn civil_calendar_known_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19723), (2024, 1, 1));
    }

    #[test]
    fn profile_save_requires_valid_unique_name() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        app.profile_prompt = Some("bad name".into());
        assert!(app.profile_save_commit().is_err());
        assert!(app.profile_prompt.is_some());
    }

    #[test]
    fn profile_pane_toggle_switches_focus() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        assert_eq!(app.profile_focus, ProfileFocus::Config);
        app.profile_pane_toggle();
        assert_eq!(app.profile_focus, ProfileFocus::List);
        app.profile_pane_toggle();
        assert_eq!(app.profile_focus, ProfileFocus::Config);
    }

    #[test]
    fn profile_list_move_wraps() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        let len = app.profiles.len();
        app.profile_list_move(-1);
        assert_eq!(app.profile_list_selection, len.saturating_sub(1));
    }

    #[test]
    fn ascensus_edit_roundtrip_saves_masked() {
        let tmp = std::env::temp_dir().join("vitriol_app_secrets_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let mut cfg = Config::from_env();
        cfg.home_dir = tmp.clone();
        let mut app = App::new(cfg, 120);
        app.ascensus_edit_start();
        app.ascensus_edit_type('k');
        app.ascensus_edit_backspace();
        let edit = AscensusEdit {
            api_key: "AIza-secret-key-9999".into(),
            model: "gemini-2.5-flash".into(),
            key_field: false,
        };
        app.ascensus_commit(edit).unwrap();
        assert!(app.ascensus.has_key());
        assert!(!app.ascensus.mask().contains("secret-key"));
        assert_eq!(app.ascensus.mask(), "••••9999");
        assert_eq!(app.ascensus.model, "gemini-2.5-flash");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ascensus_edit_next_advances_then_commits() {
        let tmp = std::env::temp_dir().join("vitriol_app_secrets_next_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let mut cfg = Config::from_env();
        cfg.home_dir = tmp.clone();
        let mut app = App::new(cfg, 120);
        app.ascensus_edit_start();
        assert!(app.ascensus_edit.as_ref().unwrap().key_field);
        app.ascensus_edit_next().unwrap();
        assert!(!app.ascensus_edit.as_ref().unwrap().key_field);
        app.ascensus_edit_next().unwrap();
        assert!(app.ascensus_edit.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
