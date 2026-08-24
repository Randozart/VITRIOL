//! UI application state.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use crate::config::Config;
use crate::control::{self, Action, Event};
use crate::model::Snapshot;
use crate::profile::{self, Profile};
use crate::search::{self, SearchHit};

/// Maximum number of decode speed samples kept for the velocity gauge.
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
    /// Sweep workshop: model/GPU/memory selection, tok/s search.
    Sweep,
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
    /// REBIS: gateway routes, draft-audit ledger, head status, event stream.
    Rebis,
}

impl Tab {
    /// All tabs in display order.
    pub const ALL: [Tab; 10] = [
        Tab::Dashboard,
        Tab::Gpu,
        Tab::Logs,
        Tab::Controls,
        Tab::Sweep,
        Tab::Hermetis,
        Tab::Subsystems,
        Tab::Profiles,
        Tab::Guide,
        // Tab::Officina — disabled: model-surgery REPL not ready for use.
        // Variant + renderer retained; re-add here to restore.
        Tab::Rebis,
    ];

    /// Short label used in the tab bar.
    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "DASHBOARD",
            Tab::Gpu => "GPU",
            Tab::Logs => "LOGS",
            Tab::Controls => "CONTROLS",
            Tab::Sweep => "SWEEP",
            Tab::Hermetis => "HERMETIS",
            Tab::Subsystems => "SUBSYSTEMS",
            Tab::Profiles => "PROFILES",
            Tab::Guide => "GUIDE",
            Tab::Officina => "OFFICINA",
            Tab::Rebis => "REBIS",
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

/// PROFILES footer actions: mouse-clickable, each with a matching key badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileAction {
    /// Toggle between the config rows and the profile list.
    SwitchPane,
    /// Save the active config as a new profile.
    Add,
    /// Duplicate the selected profile under a new name.
    Duplicate,
    /// Delete the selected profile (list) or the selected config entry (config).
    Delete,
    /// Reload the current focus pane from disk.
    Reload,
    /// Load the selected profile's config into the active config.
    Load,
    /// Select the profile at the cursor as the Start target.
    Start,
    /// Overwrite the selected installed profile with the active config.
    Overwrite,
    /// Run the Spagyric sweep on the selected profile.
    Sweep,
}

/// One clickable button drawn in the PROFILES footer this frame.
#[derive(Debug, Clone, Copy)]
pub struct ProfileButton {
    pub action: ProfileAction,
    /// On-screen rect for mouse hit-testing.
    pub area: Rect,
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
    /// Luna head (Mellum2) log.
    Luna,
    /// Mercury gateway log.
    Mercury,
    /// Inter-model traffic capture.
    Traffic,
    /// Supervisor log (head respawns).
    Supervise,
}

impl LogSource {
    /// Short label for the LOGS panel title.
    pub fn label(self) -> &'static str {
        match self {
            LogSource::Luna => "LUNA",
            LogSource::Mercury => "MERCURY",
            LogSource::Traffic => "TRAFFIC",
            LogSource::Supervise => "SUPERVISE",
            LogSource::Gen => "GEN",
            LogSource::Hermetis => "HERMETIS",
            LogSource::Embed => "EMBED",
        }
    }

    /// Display order for the LOGS source chips.
    pub const LOG_ORDER: [LogSource; 7] = [
        LogSource::Gen,
        LogSource::Luna,
        LogSource::Mercury,
        LogSource::Traffic,
        LogSource::Supervise,
        LogSource::Hermetis,
        LogSource::Embed,
    ];

    /// Cycle the active log source (LOGS tab ◄ ►).
    pub fn cycle(self, dir: i32) -> LogSource {
        let order = Self::LOG_ORDER;
        let idx = order.iter().position(|s| *s == self).unwrap_or(0);
        let next = (idx as i32 + dir).rem_euclid(order.len() as i32) as usize;
        order[next]
    }
}

/// Application state held across the event loop.
pub const SWEEP_GPU_OPTS: [&str; 3] =
    ["GPU0 — Sol card (12 GiB)", "GPU1 — Luna card (8 GiB)", "split across both"];
pub const SWEEP_CTX_PRESETS: [u32; 4] = [8192, 16384, 32768, 65536];
pub const SWEEP_MINFREE_PRESETS: [u32; 3] = [1024, 2048, 4096];
const SWEEP_HEAD_VRAM_MIB: [u32; 2] = [12042, 8113];

/// Form state for the SWEEP tab.
#[derive(Debug, Clone)]
pub struct SweepState {
    pub model_path: String,
    pub focus: usize, // 0=model 1=gpu 2=ctx 3=minfree
    pub gpu_sel: usize,
    pub ctx_idx: usize,
    pub min_free_idx: usize,
}

impl Default for SweepState {
    fn default() -> Self {
        Self {
            model_path: String::new(),
            focus: 0,
            gpu_sel: 1,
            ctx_idx: 2,
            min_free_idx: 1,
        }
    }
}

impl SweepState {
    pub fn focus_down(&mut self) { self.focus = (self.focus + 1) % 4; }
    pub fn focus_up(&mut self) { self.focus = (self.focus + 3) % 4; }

    pub fn adjust(&mut self, dir: i32) {
        match self.focus {
            1 => {
                self.gpu_sel = (self.gpu_sel as i32 + dir).rem_euclid(3) as usize;
            }
            2 => {
                self.ctx_idx = (self.ctx_idx as i32 + dir)
                    .rem_euclid(SWEEP_CTX_PRESETS.len() as i32)
                    as usize;
            }
            3 => {
                self.min_free_idx = (self.min_free_idx as i32 + dir)
                    .rem_euclid(SWEEP_MINFREE_PRESETS.len() as i32)
                    as usize;
            }
            _ => {}
        }
    }

    pub fn type_char(&mut self, c: char) {
        if self.focus == 0 {
            self.model_path.push(c);
        }
    }

    pub fn backspace(&mut self) {
        if self.focus == 0 {
            self.model_path.pop();
        }
    }

    /// (fits, used_mib, vram_mib) for the selected target.
    pub fn feasibility(&self) -> (bool, u32, u32) {
        let vram = if self.gpu_sel == 2 {
            SWEEP_HEAD_VRAM_MIB[0] + SWEEP_HEAD_VRAM_MIB[1]
        } else {
            SWEEP_HEAD_VRAM_MIB[self.gpu_sel]
        };
        let min_free = SWEEP_MINFREE_PRESETS[self.min_free_idx];
        let weights = std::fs::metadata(&self.model_path)
            .map(|m| (m.len() / 1024 / 1024) as u32)
            .unwrap_or(0);
        let kv = SWEEP_CTX_PRESETS[self.ctx_idx] * 32 / 1024; // ~32 KiB/tok conservative
        let used = weights + kv;
        (weights > 0 && used <= vram.saturating_sub(min_free), used, vram - min_free)
    }

    pub fn devices_arg(&self) -> (String, Option<String>) {
        match self.gpu_sel {
            0 => ("0".into(), None),
            1 => ("1".into(), None),
            _ => ("split".into(), Some("3,1".into())),
        }
    }
}

/// REBIS tunables persisted to ~/.vitriol/rebis.env (KEY=VALUE), sourced by
/// the launcher and gateway on start. Changes apply on next launch.
#[derive(Debug, Clone)]
pub struct RebisConfig {
    pub reasoning_budget: u32,
    pub sol_cache_ram: u32,
    pub luna_cache_ram: u32,
    pub backoff_s: u32,
    pub compact_threshold: u32,
}

impl Default for RebisConfig {
    fn default() -> Self {
        Self { reasoning_budget: 2048, sol_cache_ram: 1024,
               luna_cache_ram: 512, backoff_s: 15,
               compact_threshold: 48000 }
    }
}

pub const REBIS_ENV_PATH: &str = ".vitriol/rebis.env";

impl RebisConfig {
    pub fn load(home: &std::path::Path) -> Self {
        let mut cfg = Self::default();
        let path = home.join(REBIS_ENV_PATH);
        let Ok(text) = std::fs::read_to_string(path) else { return cfg };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v: u32 = v.trim().parse().unwrap_or(0);
            if v == 0 { continue; }
            match k.trim() {
                "REBIS_REASONING_BUDGET" => cfg.reasoning_budget = v,
                "REBIS_SOL_CACHE_RAM" => cfg.sol_cache_ram = v,
                "REBIS_LUNA_CACHE_RAM" => cfg.luna_cache_ram = v,
                "REBIS_BACKOFF" => cfg.backoff_s = v,
                "REBIS_COMPACT_THRESHOLD" => cfg.compact_threshold = v,
                _ => {}
            }
        }
        cfg
    }

    pub fn save(&self, home: &std::path::Path) -> std::io::Result<()> {
        let path = home.join(REBIS_ENV_PATH);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = format!(
            "REBIS_REASONING_BUDGET={}\nREBIS_SOL_CACHE_RAM={}\nREBIS_LUNA_CACHE_RAM={}\nREBIS_BACKOFF={}\nREBIS_COMPACT_THRESHOLD={}\n",
            self.reasoning_budget, self.sol_cache_ram,
            self.luna_cache_ram, self.backoff_s, self.compact_threshold);
        std::fs::write(path, body)
    }
}

pub struct App {
    /// Endpoint/log config.
    pub cfg: Config,
    /// Latest telemetry snapshot from the poller.
    pub snapshot: Snapshot,
    /// Decode speed history for the velocity gauge, newest last.
    pub decode_history: VecDeque<f64>,
    /// Active tab.
    pub tab: Tab,
    /// Service log selected in the LOGS tab.
    pub log_source: LogSource,
    /// LOGS detail toggle: false hides heartbeat noise + truncates lines.
    pub logs_verbose: bool,
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
    pub sweep: SweepState,
    pub rebis_cfg: RebisConfig,
    pub rebis_cfg_focus: usize,
    pub rebis_cfg_dirty: bool,
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
    /// While the prompt is open for a DUPLICATE, the source profile name;
    /// None for the plain save-as flow.
    pub profile_dup_source: Option<String>,
    /// Clickable buttons drawn in the PROFILES footer this frame.
    pub profile_buttons: Vec<ProfileButton>,
    /// Tab hit-boxes drawn in the header this frame (mouse-clickable tabs).
    pub tab_hits: Vec<(Rect, Tab)>,
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
        let home_dir = cfg.home_dir.clone();
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
            logs_verbose: false,
            profiles,
            selected_action: 0,
            control_running: false,
            control_action: String::new(),
            control_step: String::new(),
            sweep: SweepState::default(),
            rebis_cfg: RebisConfig::load(&home_dir),
            rebis_cfg_focus: 0,
            rebis_cfg_dirty: false,
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
            profile_dup_source: None,
            profile_buttons: Vec::new(),
            tab_hits: Vec::new(),
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

    /// Abort the save-as/duplicate prompt.
    pub fn profile_save_cancel(&mut self) {
        self.profile_prompt = None;
        self.profile_dup_source = None;
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

    /// Begin duplicating the selected profile: prompt prefilled with
    /// `<name>-copy`. Works for bundled sources too — the copy always lands in
    /// the installed dir.
    pub fn profile_duplicate_start(&mut self) {
        let Some(profile) = self.profiles.get(self.profile_list_selection) else {
            return;
        };
        self.profile_dup_source = Some(profile.name.clone());
        self.profile_prompt = Some(format!("{}-copy", profile.name));
    }

    /// Commit the duplicate prompt: copy the source profile's config (and its
    /// meta description) into a new installed profile.
    pub fn profile_duplicate_commit(&mut self) -> Result<(), String> {
        let Some(buf) = self.profile_prompt.clone() else {
            return Ok(());
        };
        let name = buf.trim().to_string();
        if !valid_profile_name(&name) {
            return Err(format!(
                "invalid profile name '{name}' (letters, numbers, hyphens, underscores)"
            ));
        }
        if self.profiles.iter().any(|p| p.name == name) {
            return Err(format!("profile '{name}' already exists"));
        }
        let Some(src_name) = self.profile_dup_source.take() else {
            return Ok(());
        };
        self.profile_prompt = None;
        let Some(src) = self.profiles.iter().find(|p| p.name == src_name).cloned() else {
            return Err(format!("source profile '{src_name}' gone"));
        };
        let src_dir = match src.source {
            crate::profile::ProfileSource::Installed => {
                self.cfg.installed_profiles_dir().join(&src.name)
            }
            crate::profile::ProfileSource::Bundled => {
                self.cfg.bundled_profiles_dir().join(&src.name)
            }
        };
        let config_text = std::fs::read_to_string(src_dir.join("config"))
            .map_err(|e| format!("read {}: {e}", src_dir.join("config").display()))?;
        let meta_text = std::fs::read_to_string(src_dir.join("meta")).unwrap_or_default();
        let description = meta_text
            .lines()
            .find_map(|l| l.strip_prefix("description="))
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let dst = self.cfg.installed_profiles_dir().join(&name);
        std::fs::create_dir_all(&dst).map_err(|e| format!("mkdir {}: {e}", dst.display()))?;
        crate::config_edit::atomic_write_path(&dst.join("config"), &config_text)?;
        let meta = format!(
            "name={name}\ndescription={description}\ncreated={}\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        std::fs::write(dst.join("meta"), meta).map_err(|e| format!("write meta: {e}"))?;
        self.profile_reload_list();
        if let Some(idx) = self.profiles.iter().position(|p| p.name == name) {
            self.profile_list_selection = idx;
            self.profile_focus = ProfileFocus::List;
            self.select_profile(Some(name.clone()));
            self.push_control_line(format!("✓ duplicated '{src_name}' -> '{name}'"));
        }
        Ok(())
    }
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

    /// Clear the PROFILES footer buttons (called at the top of each render so
    /// stale hit-boxes never survive a resize).
    pub fn reset_profile_buttons(&mut self) {
        self.profile_buttons.clear();
    }

    /// Map a mouse click to a PROFILES footer action, or None if it missed.
    pub fn profile_click(&self, x: u16, y: u16) -> Option<ProfileAction> {
        self.profile_buttons
            .iter()
            .find(|b| b.area.contains(Position::new(x, y)))
            .map(|b| b.action)
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

    pub fn rebis_cfg_focus_up(&mut self) {
        self.rebis_cfg_focus = (self.rebis_cfg_focus + 4) % 5;
    }

    pub fn rebis_cfg_focus_down(&mut self) {
        self.rebis_cfg_focus = (self.rebis_cfg_focus + 1) % 5;
    }

    pub fn rebis_cfg_adjust(&mut self, dir: i32) {
        let step = |v: u32, d: i32, lo: u32, hi: u32, sz: u32| {
            (v as i32 + d * sz as i32).clamp(lo as i32, hi as i32) as u32
        };
        let changed = match self.rebis_cfg_focus {
            0 => { self.rebis_cfg.reasoning_budget =
                       step(self.rebis_cfg.reasoning_budget, dir, 256, 8192, 256); true }
            1 => { self.rebis_cfg.sol_cache_ram =
                       step(self.rebis_cfg.sol_cache_ram, dir, 256, 8192, 256); true }
            2 => { self.rebis_cfg.luna_cache_ram =
                       step(self.rebis_cfg.luna_cache_ram, dir, 128, 4096, 128); true }
            3 => { self.rebis_cfg.backoff_s =
                       step(self.rebis_cfg.backoff_s, dir, 5, 120, 5); true }
            4 => { self.rebis_cfg.compact_threshold =
                       step(self.rebis_cfg.compact_threshold, dir, 8000, 120000, 4000); true }
            _ => false,
        };
        if changed {
            match self.rebis_cfg.save(&self.cfg.home_dir) {
                Ok(()) => self.rebis_cfg_dirty = false,
                Err(e) => self.push_control_line(format!(
                    "rebis config save failed: {e}")),
            }
        }
    }

    /// The sweep action for the current form state.
    pub fn sweep_action(&self) -> Action {
        let (devices, ts) = self.sweep.devices_arg();
        Action::RunSweepConfig {
            model: self.sweep.model_path.clone(),
            devices,
            ts,
            ctx: SWEEP_CTX_PRESETS[self.sweep.ctx_idx],
        }
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
                self.finished_action = None;
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

    /// Fold a fresh snapshot into app state, updating the decode history. The
    /// history tracks the live heartbeat speed (0.0 while idle) so the peak and
    /// gauge reflect only real generation, never a sticky last-completion value.
    pub fn apply_snapshot(&mut self, snap: Snapshot) {
        let t_s = snap.gen.decode_speed;
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

    /// Switch straight to `tab` (mouse click on a header tab).
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Clear the header tab hit-boxes (called at the top of each render).
    pub fn reset_tab_hits(&mut self) {
        self.tab_hits.clear();
    }

    /// Map a mouse click to the header tab under the cursor, or None.
    pub fn tab_click(&self, x: u16, y: u16) -> Option<Tab> {
        self.tab_hits
            .iter()
            .find(|(area, _)| area.contains(Position::new(x, y)))
            .map(|&(_, tab)| tab)
    }

    /// Step back to the previous tab, wrapping.
    pub fn prev_tab(&mut self) {
        let idx = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        self.tab = Tab::ALL[(idx + Tab::ALL.len() - 1) % Tab::ALL.len()];
    }

    /// Lines of the currently selected service log, oldest first.
    pub fn cycle_log_source(&mut self, dir: i32) {
        self.log_source = self.log_source.cycle(dir);
    }

    pub fn current_log_lines(&self) -> &[String] {
        match self.log_source {
            LogSource::Gen => &self.snapshot.logs.gen,
            LogSource::Hermetis => &self.snapshot.logs.hermetis,
            LogSource::Embed => &self.snapshot.logs.embed,
            LogSource::Luna => &self.snapshot.logs.luna,
            LogSource::Mercury => &self.snapshot.logs.mercury,
            LogSource::Traffic => &self.snapshot.logs.traffic,
            LogSource::Supervise => &self.snapshot.logs.supervise,
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
        assert_eq!(Tab::ALL.len(), 10);
        for tab in Tab::ALL {
            assert!(!tab.label().is_empty());
        }
        assert_eq!(Tab::ALL[0], Tab::Dashboard);
        assert_eq!(Tab::ALL[Tab::ALL.len() - 1], Tab::Rebis);
        assert!(!Tab::ALL.contains(&Tab::Officina));
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
    fn profile_duplicate_copy_lands_in_installed() {
        let tmp = std::env::temp_dir().join("vitriol_profile_dup_copy_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let mut cfg = Config::from_env();
        cfg.home_dir = tmp.clone();
        let mut app = App::new(cfg, 120);

        app.profile_prompt = Some("dup_src_tmp".into());
        app.profile_save_commit().unwrap();
        app.profile_list_selection = app
            .profiles
            .iter()
            .position(|p| p.name == "dup_src_tmp")
            .unwrap();
        assert!(app.profiles.iter().any(|p| p.name == "dup_src_tmp"));

        app.profile_duplicate_start();
        assert_eq!(app.profile_prompt.as_deref(), Some("dup_src_tmp-copy"));
        assert_eq!(app.profile_dup_source.as_deref(), Some("dup_src_tmp"));
        app.profile_duplicate_commit().unwrap();

        let base = app.cfg.installed_profiles_dir();
        assert!(base.join("dup_src_tmp-copy").join("config").is_file());
        assert!(base.join("dup_src_tmp-copy").join("meta").is_file());
        assert!(app.profiles.iter().any(|p| p.name == "dup_src_tmp-copy"));
        assert!(app.profile_prompt.is_none());
        assert!(app.profile_dup_source.is_none());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn profile_duplicate_rejects_existing_name() {
        let tmp = std::env::temp_dir().join("vitriol_profile_dup_reject_test");
        let _ = std::fs::remove_dir_all(&tmp);
        let mut cfg = Config::from_env();
        cfg.home_dir = tmp.clone();
        let mut app = App::new(cfg, 120);

        app.profile_prompt = Some("x".into());
        app.profile_save_commit().unwrap();
        app.profile_duplicate_start();
        app.profile_prompt = Some("x".into());
        assert!(app.profile_duplicate_commit().is_err());
        assert!(app.profile_dup_source.is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn profile_cancel_clears_dup_source() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        app.profile_prompt = Some("source-copy".into());
        app.profile_dup_source = Some("source".into());
        app.profile_save_cancel();
        assert!(app.profile_prompt.is_none());
        assert!(app.profile_dup_source.is_none());
    }

    #[test]
    fn profile_click_hits_none_outside_buttons() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        assert_eq!(app.profile_click(0, 0), None);
        app.profile_buttons.push(ProfileButton {
            action: ProfileAction::Add,
            area: Rect::new(10, 10, 5, 1),
        });
        assert_eq!(app.profile_click(12, 10), Some(ProfileAction::Add));
        assert_eq!(app.profile_click(30, 10), None);
        assert_eq!(app.profile_click(12, 30), None);
    }

    #[test]
    fn tab_click_resolves_to_tab() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        assert_eq!(app.tab_click(0, 0), None);
        app.tab_hits.push((Rect::new(9, 0, 6, 1), Tab::Profiles));
        app.tab_hits.push((Rect::new(15, 0, 4, 1), Tab::Guide));
        assert_eq!(app.tab_click(11, 0), Some(Tab::Profiles));
        assert_eq!(app.tab_click(16, 0), Some(Tab::Guide));
        assert_eq!(app.tab_click(30, 0), None);
        app.reset_tab_hits();
        assert_eq!(app.tab_click(11, 0), None);
    }

    #[test]
    fn set_tab_switches_directly() {
        let cfg = Config::from_env();
        let mut app = App::new(cfg, 120);
        app.set_tab(Tab::Profiles);
        assert_eq!(app.tab, Tab::Profiles);
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
