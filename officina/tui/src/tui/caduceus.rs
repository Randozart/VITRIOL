// /caduceus page data — the gateway FACTS file written by the caduceus
// extension (caduceus-state.json) and the POLICY file the TUI owns
// (caduceus-prefs.json). Contract I4: the TUI never talks HTTP to the gateway
// and never sees secrets; it reads the snapshot and writes routing prefs only.
//
// PROVENANCE: original work, this repo.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Connections {
    pub active: Vec<String>,
    pub backoff: Vec<String>,
    pub hard_down: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct KeyRow {
    pub provider: String,
    pub env: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Budget {
    pub daily_tokens: u64,
    pub daily_cap: u64,
    pub monthly_tokens: u64,
    pub monthly_cap: u64,
    pub allowed: bool,
    pub paid_allowed: bool,
}

/// One Providers-tab row (state v2). Facts only — free-ness of the row's
/// models is derived by the extension, never written by the UI (I3).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ProviderRow {
    pub id: String,
    pub name: String,
    pub website: String,
    pub category: String,
    pub has_free: bool,
    pub free_note: String,
    pub auth_hint: String,
    pub credential_env: String,
    pub key_present: bool,
    pub enabled: bool,
    pub signup_status: String,
    pub gateway: String,
    pub free_models: u64,
    pub method: String,
}

/// One Models-tab row (state v2).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CandidateRow {
    pub id: String,
    pub owner: String,
    pub context_window: u64,
    pub max_tokens: u64,
    pub free: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CaduceusState {
    /// 0 for v1 files (pre-provider snapshots)
    pub version: u32,
    /// ms epoch of the write (staleness rendering)
    pub ts: u64,
    pub base_url: String,
    pub gateway_up: bool,
    pub catalog_count: u64,
    pub catalog_fetched_at: String,
    pub connections: Connections,
    pub cooling: u64,
    pub inference_key: String,
    pub keys: Vec<KeyRow>,
    pub budget: Budget,
    /// v2 facts; empty for v1 snapshots (I5 backward compatibility)
    pub providers: Vec<ProviderRow>,
    pub candidates: Vec<CandidateRow>,
}

impl CaduceusState {
    /// Snapshot age in ms (0 when the stamp is from the future — clock skew
    /// must not render negative ages).
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.ts)
    }

    pub fn is_stale(&self, now_ms: u64) -> bool {
        self.age_ms(now_ms) > 180_000
    }

    pub fn free_candidate_count(&self) -> usize {
        self.candidates.iter().filter(|c| c.free).count()
    }
}

// ── Policy (prefs) ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPrefs {
    /// "on" | "off"; None = default on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_paid: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPrefs {
    pub pinned: Vec<String>,
    pub denied: Vec<String>,
    /// Display-only (2026-11-12): hidden from the Models tab unless H shows
    /// hidden rows. Never affects routing — `denied` is the routing control.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CaduceusPrefs {
    pub version: u32,
    pub free_only: bool,
    pub per_owner: u32,
    pub max_candidates: u32,
    pub providers: BTreeMap<String, ProviderPrefs>,
    pub models: ModelPrefs,
}

impl Default for CaduceusPrefs {
    fn default() -> Self {
        Self {
            version: 1,
            free_only: true,
            per_owner: 8,
            max_candidates: 48,
            providers: BTreeMap::new(),
            models: ModelPrefs::default(),
        }
    }
}

impl CaduceusPrefs {
    pub fn routing(&self, id: &str) -> &str {
        self.providers
            .get(id)
            .and_then(|p| p.routing.as_deref())
            .unwrap_or("on")
    }

    pub fn rank(&self, id: &str) -> Option<u32> {
        self.providers.get(id).and_then(|p| p.rank)
    }

    pub fn allow_paid(&self, id: &str) -> bool {
        self.providers
            .get(id)
            .and_then(|p| p.allow_paid)
            .unwrap_or(false)
    }

    pub fn is_hidden(&self, id: &str) -> bool {
        self.models.hidden.iter().any(|m| m == id)
    }

    pub fn model_state(&self, id: &str) -> ModelState {
        if self.models.pinned.iter().any(|m| m == id) {
            ModelState::Pinned
        } else if self.models.denied.iter().any(|m| m == id) {
            ModelState::Denied
        } else {
            ModelState::Neutral
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelState {
    Pinned,
    Denied,
    Neutral,
}

impl ModelState {
    pub fn label(self) -> &'static str {
        match self {
            ModelState::Pinned => "pinned",
            ModelState::Denied => "denied",
            ModelState::Neutral => "",
        }
    }
}

pub fn state_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("caduceus-state.json");
    p
}

pub fn prefs_path() -> PathBuf {
    if let Some(p) = std::env::var_os("OFFICINA_CADUCEUS_PREFS_FILE") {
        return PathBuf::from(p);
    }
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("caduceus-prefs.json");
    p
}

pub fn parse_state(json: &str) -> Option<CaduceusState> {
    serde_json::from_str(json).ok()
}

pub fn load() -> Option<CaduceusState> {
    load_at(&state_path())
}

pub fn load_at(path: &Path) -> Option<CaduceusState> {
    parse_state(&fs::read_to_string(path).ok()?)
}

/// Load policy. Absent file → defaults, no note. Invalid/unsupported →
/// defaults + a note the page footer shows (I6 corrupt-safe).
/// HOME-path convenience wrapper (used by the /status snapshot).
pub fn load_prefs() -> (CaduceusPrefs, Option<String>) {
    load_prefs_at(&prefs_path())
}

pub fn load_prefs_at(path: &Path) -> (CaduceusPrefs, Option<String>) {
    if !path.exists() {
        return (CaduceusPrefs::default(), None);
    }
    match fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<CaduceusPrefs>(&raw) {
            Ok(prefs) if prefs.version == 1 => (prefs, None),
            Ok(other) => (
                CaduceusPrefs::default(),
                Some(format!(
                    "prefs ignored: unsupported version {}",
                    other.version
                )),
            ),
            Err(_) => (
                CaduceusPrefs::default(),
                Some("prefs ignored: invalid JSON".to_string()),
            ),
        },
        Err(_) => (
            CaduceusPrefs::default(),
            Some("prefs ignored: unreadable".to_string()),
        ),
    }
}

/// Atomic tmp+rename write (I6).
pub fn save_prefs_at(path: &Path, prefs: &CaduceusPrefs) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let body = serde_json::to_string_pretty(prefs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&tmp, format!("{body}\n"))?;
    fs::rename(&tmp, path)
}

// ── Page ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Providers,
    Models,
    Health,
    Policy,
    Vitriol,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Providers,
        Tab::Models,
        Tab::Health,
        Tab::Policy,
        Tab::Vitriol,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Providers => "Providers",
            Tab::Models => "Models",
            Tab::Health => "Health",
            Tab::Policy => "Policy",
            Tab::Vitriol => "Vitriol",
        }
    }

    pub fn next(self) -> Tab {
        match self {
            Tab::Providers => Tab::Models,
            Tab::Models => Tab::Health,
            Tab::Health => Tab::Policy,
            Tab::Policy => Tab::Vitriol,
            Tab::Vitriol => Tab::Providers,
        }
    }

    pub fn prev(self) -> Tab {
        match self {
            Tab::Providers => Tab::Vitriol,
            Tab::Models => Tab::Providers,
            Tab::Health => Tab::Models,
            Tab::Policy => Tab::Health,
            Tab::Vitriol => Tab::Policy,
        }
    }

    pub fn from_number(n: usize) -> Option<Tab> {
        Tab::ALL.get(n.wrapping_sub(1)).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyRow {
    FreeOnly,
    PerOwner,
    MaxCandidates,
    AllowPaid { provider: String, allow: bool },
}

/// Engine facts sampled from the systemd unit (Vitriol tab). Sampled on
/// page open, `r`, tab entry, and after a control action — never per frame.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EngineInfo {
    pub state: String,
    pub pid: String,
    pub since: String,
    pub last_log: String,
}

/// Pure parse of `systemctl show -p ActiveState -p MainPID
/// -p ExecMainStartTimestamp` output (key=value lines).
pub fn parse_engine_show(output: &str) -> (String, String, String) {
    let mut state = String::new();
    let mut pid = String::new();
    let mut since = String::new();
    for line in output.lines() {
        if let Some(v) = line.strip_prefix("ActiveState=") {
            state = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("MainPID=") {
            let v = v.trim();
            if v != "0" {
                pid = v.to_string();
            }
        } else if let Some(v) = line.strip_prefix("ExecMainStartTimestamp=") {
            since = v.trim().to_string();
        }
    }
    (state, pid, since)
}

pub fn probe_engine() -> EngineInfo {
    let show = std::process::Command::new("systemctl")
        .args([
            "show",
            "vitriol-server.service",
            "-p",
            "ActiveState",
            "-p",
            "MainPID",
            "-p",
            "ExecMainStartTimestamp",
            "--no-pager",
        ])
        .output();
    let (state, pid, since) = match show {
        Ok(o) => parse_engine_show(&String::from_utf8_lossy(&o.stdout)),
        Err(_) => (String::new(), String::new(), String::new()),
    };
    let last_log = match std::process::Command::new("journalctl")
        .args([
            "-u",
            "vitriol-server.service",
            "-n",
            "1",
            "--no-pager",
            "-o",
            "cat",
        ])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .trim()
            .chars()
            .take(160)
            .collect(),
        Err(_) => String::new(),
    };
    EngineInfo {
        state: if state.is_empty() {
            "unknown".into()
        } else {
            state
        },
        pid,
        since,
        last_log,
    }
}

/// The page state owned by AppState while `/caduceus` is open.
#[derive(Debug, Clone)]
pub struct Page {
    /// Policy file this page reads/writes. Held explicitly so tests never
    /// touch the operator's HOME file (env-based path resolved once at load).
    pub prefs_file: PathBuf,
    pub tab: Tab,
    pub sel: usize,
    pub filter: String,
    pub filtering: bool,
    /// Show hidden models (H). Hidden rows are display-only, never routing.
    pub show_hidden: bool,
    pub detail: bool,
    pub state: Option<CaduceusState>,
    pub prefs: CaduceusPrefs,
    /// Footer note: prefs save result or a corrupt-file warning.
    pub note: Option<String>,
    /// Vitriol tab: sampled engine facts + armed control (press-twice
    /// confirmation for stop/restart — they interrupt active turns).
    pub engine: Option<EngineInfo>,
    pub engine_arm: Option<&'static str>,
    /// Vitriol tab: the live config with one-line explanations (stage 6).
    /// Read-only; loaded on open/reload/tab entry.
    pub config: Option<Vec<crate::tui::engine_config::ConfigRow>>,
    pub config_scroll: usize,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            prefs_file: prefs_path(),
            tab: Tab::Providers,
            sel: 0,
            filter: String::new(),
            filtering: false,
            show_hidden: false,
            detail: false,
            state: None,
            prefs: CaduceusPrefs::default(),
            note: None,
            engine: None,
            engine_arm: None,
            config: None,
            config_scroll: 0,
        }
    }
}

impl Page {
    pub fn load() -> Self {
        let prefs_file = prefs_path();
        let (prefs, note) = load_prefs_at(&prefs_file);
        Self {
            prefs_file,
            state: load(),
            prefs,
            note,
            engine: Some(probe_engine()),
            config: crate::tui::engine_config::load(),
            ..Self::default()
        }
    }

    /// `r` — re-read both files (and the engine facts); selection clamps.
    pub fn reload(&mut self) {
        self.state = load();
        self.engine = Some(probe_engine());
        self.config = crate::tui::engine_config::load();
        self.config_scroll = 0;
        let (prefs, note) = load_prefs_at(&self.prefs_file);
        self.prefs = prefs;
        self.note = note;
        self.clamp_sel();
    }

    fn matches(&self, id: &str, name: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        let needle = self.filter.to_lowercase();
        id.to_lowercase().contains(&needle) || name.to_lowercase().contains(&needle)
    }

    /// Providers view: filter, then explicit rank (asc, unranked last), then id.
    pub fn provider_view(&self) -> Vec<&ProviderRow> {
        let mut rows: Vec<&ProviderRow> = self
            .state
            .as_ref()
            .map(|s| {
                s.providers
                    .iter()
                    .filter(|p| self.matches(&p.id, &p.name))
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| {
            let ra = self.prefs.rank(&a.id).unwrap_or(u32::MAX);
            let rb = self.prefs.rank(&b.id).unwrap_or(u32::MAX);
            ra.cmp(&rb).then_with(|| a.id.cmp(&b.id))
        });
        rows
    }

    pub fn candidate_view(&self) -> Vec<&CandidateRow> {
        self.state
            .as_ref()
            .map(|s| {
                s.candidates
                    .iter()
                    .filter(|c| self.matches(&c.id, &c.owner))
                    .filter(|c| self.show_hidden || !self.prefs.is_hidden(&c.id))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn policy_rows(&self) -> Vec<PolicyRow> {
        let mut rows = vec![
            PolicyRow::FreeOnly,
            PolicyRow::PerOwner,
            PolicyRow::MaxCandidates,
        ];
        if let Some(s) = self.state.as_ref() {
            let mut providers: Vec<&ProviderRow> = s
                .providers
                .iter()
                .filter(|p| p.enabled || p.key_present || self.prefs.providers.contains_key(&p.id))
                .collect();
            providers.sort_by(|a, b| a.id.cmp(&b.id));
            for p in providers {
                rows.push(PolicyRow::AllowPaid {
                    allow: self.prefs.allow_paid(&p.id),
                    provider: p.id.clone(),
                });
            }
        }
        rows
    }

    pub fn row_count(&self) -> usize {
        match self.tab {
            Tab::Providers => self.provider_view().len(),
            Tab::Models => self.candidate_view().len(),
            Tab::Health => 0,
            Tab::Policy => self.policy_rows().len(),
            Tab::Vitriol => 0, // static facts, no selectable rows
        }
    }

    pub fn clamp_sel(&mut self) {
        let n = self.row_count();
        if n == 0 {
            self.sel = 0;
        } else if self.sel >= n {
            self.sel = n - 1;
        }
    }

    pub fn set_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            self.sel = 0;
            self.detail = false;
            self.engine_arm = None;
            if tab == Tab::Vitriol {
                if self.engine.is_none() {
                    self.engine = Some(probe_engine());
                }
                if self.config.is_none() {
                    self.config = crate::tui::engine_config::load();
                }
            }
        }
    }

    pub fn move_sel(&mut self, delta: i32) {
        let n = self.row_count();
        if n == 0 {
            self.sel = 0;
            return;
        }
        let next = self.sel as i32 + delta;
        self.sel = next.clamp(0, n as i32 - 1) as usize;
    }

    /// Space — per-tab toggle. Saves prefs when something changed.
    pub fn toggle(&mut self) -> bool {
        let changed = match self.tab {
            Tab::Providers => {
                let id = self.provider_view().get(self.sel).map(|p| p.id.clone());
                match id {
                    Some(id) => {
                        let entry = self.prefs.providers.entry(id.clone()).or_default();
                        entry.routing = Some(match entry.routing.as_deref() {
                            Some("off") => "on".to_string(),
                            _ => "off".to_string(),
                        });
                        true
                    }
                    None => false,
                }
            }
            Tab::Models => {
                let id = self.candidate_view().get(self.sel).map(|c| c.id.clone());
                match id {
                    Some(id) => {
                        match self.prefs.model_state(&id) {
                            ModelState::Neutral => {
                                self.prefs.models.pinned.push(id);
                            }
                            ModelState::Pinned => {
                                self.prefs.models.pinned.retain(|m| *m != id);
                                self.prefs.models.denied.push(id);
                            }
                            ModelState::Denied => {
                                self.prefs.models.denied.retain(|m| *m != id);
                            }
                        }
                        true
                    }
                    None => false,
                }
            }
            Tab::Health | Tab::Vitriol => false,
            Tab::Policy => {
                let row = self.policy_rows().get(self.sel).cloned();
                match row {
                    Some(PolicyRow::FreeOnly) => {
                        self.prefs.free_only = !self.prefs.free_only;
                        true
                    }
                    Some(PolicyRow::AllowPaid { provider, .. }) => {
                        let entry = self.prefs.providers.entry(provider).or_default();
                        entry.allow_paid = Some(!entry.allow_paid.unwrap_or(false));
                        true
                    }
                    // PerOwner/MaxCandidates use +/- instead of Space.
                    _ => false,
                }
            }
        };
        if changed {
            self.save();
        }
        changed
    }

    /// +/- — Providers: rank; Policy: caps.
    pub fn adjust(&mut self, delta: i32) -> bool {
        let changed = match self.tab {
            Tab::Providers => {
                let id = self.provider_view().get(self.sel).map(|p| p.id.clone());
                match id {
                    Some(id) => {
                        let entry = self.prefs.providers.entry(id).or_default();
                        // `+` promotes (lower rank number), `-` demotes.
                        // First promotion pins to the top, first demotion to
                        // the tail; existing ranks move linearly.
                        let next = match entry.rank {
                            None if delta > 0 => 0,
                            None => 99,
                            Some(current) => (current as i32 - delta).clamp(0, 99) as u32,
                        };
                        entry.rank = Some(next);
                        true
                    }
                    None => false,
                }
            }
            Tab::Policy => {
                let row = self.policy_rows().get(self.sel).cloned();
                match row {
                    Some(PolicyRow::PerOwner) => {
                        self.prefs.per_owner =
                            (self.prefs.per_owner as i32 + delta).clamp(1, 32) as u32;
                        true
                    }
                    Some(PolicyRow::MaxCandidates) => {
                        self.prefs.max_candidates =
                            (self.prefs.max_candidates as i32 + delta).clamp(1, 256) as u32;
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        };
        if changed {
            self.save();
        }
        changed
    }

    /// Remove the selected model from both pin/deny lists (Backspace/Delete).
    pub fn clear_model_state(&mut self) -> bool {
        if self.tab != Tab::Models {
            return false;
        }
        let id = self.candidate_view().get(self.sel).map(|c| c.id.clone());
        match id {
            Some(id) => {
                let before = (
                    self.prefs.models.pinned.len(),
                    self.prefs.models.denied.len(),
                );
                self.prefs.models.pinned.retain(|m| *m != id);
                self.prefs.models.denied.retain(|m| *m != id);
                let changed = before
                    != (
                        self.prefs.models.pinned.len(),
                        self.prefs.models.denied.len(),
                    );
                if changed {
                    self.save();
                }
                changed
            }
            None => false,
        }
    }

    /// `h` — hide/unhide the selected model (display only, no routing effect).
    pub fn toggle_hidden(&mut self) -> bool {
        if self.tab != Tab::Models {
            return false;
        }
        let id = self.candidate_view().get(self.sel).map(|c| c.id.clone());
        match id {
            Some(id) => {
                if let Some(pos) = self.prefs.models.hidden.iter().position(|m| *m == id) {
                    self.prefs.models.hidden.remove(pos);
                } else {
                    self.prefs.models.hidden.push(id);
                }
                self.clamp_sel();
                self.save();
                true
            }
            None => false,
        }
    }

    /// `H` — show or hide the hidden rows in the Models tab.
    pub fn toggle_show_hidden(&mut self) {
        if self.tab == Tab::Models {
            self.show_hidden = !self.show_hidden;
            self.clamp_sel();
        }
    }

    fn save(&mut self) {
        self.note = Some(match save_prefs_at(&self.prefs_file, &self.prefs) {
            Ok(()) => "prefs saved".to_string(),
            Err(e) => format!("prefs save failed: {e}"),
        });
    }

    /// Effective policy summary for the footer.
    pub fn policy_summary(&self) -> String {
        format!(
            "free_only={} · per_owner={} · max={}",
            if self.prefs.free_only { "on" } else { "off" },
            self.prefs.per_owner,
            self.prefs.max_candidates
        )
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 999 → "999" · 1234 → "1.2k" · 1_234_000 → "1.2M"
pub fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        trim_dot_zero(format!("{:.1}", n as f64 / 1_000_000.0)) + "M"
    } else if n >= 1_000 {
        trim_dot_zero(format!("{:.1}", n as f64 / 1_000.0)) + "k"
    } else {
        n.to_string()
    }
}

fn trim_dot_zero(mut s: String) -> String {
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    s
}

/// 5_000 ms → "5s" · 120_000 → "2m" · 7_200_000 → "2h" · 172_800_000 → "2d"
pub fn fmt_age_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        format!("{s}s")
    } else if s < 3_600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3_600)
    } else {
        format!("{}d", s / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "ts": 1731000000000,
        "base_url": "http://127.0.0.1:20128/v1",
        "gateway_up": true,
        "catalog_count": 123,
        "catalog_fetched_at": "2026-11-11T10:00:00.000Z",
        "connections": {"active": ["a","b"], "backoff": ["c"], "hard_down": ["d"]},
        "cooling": 2,
        "inference_key": "...a1b2",
        "keys": [{"provider":"zai","env":"ZAI_KEY","value":"...c3d4","source":"env-file"}],
        "budget": {"daily_tokens":1200000,"daily_cap":2000000,"monthly_tokens":8000000,
                   "monthly_cap":40000000,"allowed":true,"paid_allowed":false}
    }"#;

    const SAMPLE_V2: &str = r#"{
        "version": 2,
        "ts": 1731000000000,
        "base_url": "http://127.0.0.1:20128/v1",
        "gateway_up": true,
        "catalog_count": 5,
        "catalog_fetched_at": "",
        "connections": {"active": ["glm"], "backoff": [], "hard_down": []},
        "cooling": 0,
        "inference_key": "",
        "keys": [],
        "budget": {"daily_tokens":0,"daily_cap":0,"monthly_tokens":0,"monthly_cap":0,
                   "allowed":true,"paid_allowed":false},
        "providers": [
          {"id":"glm","name":"GLM","website":"https://glm","category":"api-key",
           "has_free":true,"free_note":"flash tier","auth_hint":"key",
           "credential_env":"GLM_API_KEY","key_present":true,"enabled":true,
           "signup_status":"keyed","gateway":"active","free_models":3,"method":"api-key"},
          {"id":"dahl","name":"Dahl","key_present":false,"enabled":false,"gateway":"unknown",
           "free_models":0,"method":"api-key"}
        ],
        "candidates": [
          {"id":"glm/glm-4.7-flash","owner":"glm","context_window":200000,
           "max_tokens":16384,"free":true}
        ]
    }"#;

    fn tmp_path(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("officina-caduceus-{}-{tag}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p.join("caduceus-prefs.json")
    }

    #[test]
    fn parses_full_state() {
        let s = parse_state(SAMPLE).expect("sample parses");
        assert!(s.gateway_up);
        assert_eq!(s.catalog_count, 123);
        assert_eq!(s.connections.active, vec!["a", "b"]);
        assert_eq!(s.connections.hard_down, vec!["d"]);
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.keys[0].provider, "zai");
        assert_eq!(s.budget.daily_cap, 2_000_000);
        assert!(s.budget.allowed);
        assert!(!s.budget.paid_allowed);
        assert_eq!(s.inference_key, "...a1b2");
        // v1 snapshot: no provider facts (I5)
        assert_eq!(s.version, 0);
        assert!(s.providers.is_empty());
        assert!(s.candidates.is_empty());
    }

    #[test]
    fn parses_v2_provider_and_candidate_rows() {
        let s = parse_state(SAMPLE_V2).expect("v2 parses");
        assert_eq!(s.version, 2);
        assert_eq!(s.providers.len(), 2);
        let glm = &s.providers[0];
        assert_eq!(glm.free_models, 3);
        assert!(glm.key_present);
        assert!(glm.has_free);
        assert_eq!(glm.method, "api-key");
        assert_eq!(s.candidates.len(), 1);
        assert!(s.candidates[0].free);
        assert_eq!(s.free_candidate_count(), 1);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let s = parse_state("{}").expect("empty object parses");
        assert!(!s.gateway_up);
        assert_eq!(s.catalog_count, 0);
        assert!(s.connections.active.is_empty());
        assert_eq!(s.budget.daily_cap, 0);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse_state("not json").is_none());
        assert!(parse_state(r#"{"ts":"oops"}"#).is_none());
    }

    #[test]
    fn load_at_missing_file_is_none() {
        assert!(load_at(Path::new("/nonexistent/caduceus-state.json")).is_none());
    }

    #[test]
    fn age_is_saturating_and_skew_safe() {
        let s = parse_state(SAMPLE).unwrap();
        assert_eq!(s.age_ms(1_731_000_010_000), 10_000);
        assert_eq!(s.age_ms(1_730_999_990_000), 0); // future stamp → 0
        assert!(!s.is_stale(1_731_000_000_000));
        assert!(s.is_stale(1_731_000_181_000));
    }

    #[test]
    fn fmt_count_buckets() {
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_234), "1.2k");
        assert_eq!(fmt_count(20_000), "20k");
        assert_eq!(fmt_count(1_234_000), "1.2M");
        assert_eq!(fmt_count(40_000_000), "40M");
    }

    #[test]
    fn fmt_age_buckets() {
        assert_eq!(fmt_age_ms(5_000), "5s");
        assert_eq!(fmt_age_ms(59_000), "59s");
        assert_eq!(fmt_age_ms(120_000), "2m");
        assert_eq!(fmt_age_ms(7_200_000), "2h");
        assert_eq!(fmt_age_ms(172_800_000), "2d");
    }

    #[test]
    fn prefs_parse_hidden_list() {
        let prefs: CaduceusPrefs =
            serde_json::from_str(r#"{"version":1,"models":{"hidden":["a/b"]}}"#).unwrap();
        assert!(prefs.is_hidden("a/b"));
        assert!(!prefs.is_hidden("a/c"));
        // hidden is not routing: neither denied nor pinned
        assert_eq!(prefs.model_state("a/b"), ModelState::Neutral);
    }

    #[test]
    fn prefs_absent_file_is_defaults_without_note() {
        let path = tmp_path("absent");
        let _ = fs::remove_file(&path);
        let (prefs, note) = load_prefs_at(&path);
        assert_eq!(prefs, CaduceusPrefs::default());
        assert!(note.is_none());
    }

    #[test]
    fn prefs_round_trip_and_invalid_is_ignored() {
        let path = tmp_path("roundtrip");
        let mut prefs = CaduceusPrefs::default();
        prefs.free_only = false;
        prefs.per_owner = 3;
        prefs.providers.insert(
            "glm".into(),
            ProviderPrefs {
                routing: Some("off".into()),
                rank: Some(2),
                allow_paid: Some(true),
            },
        );
        prefs.models.pinned.push("glm/glm-4.7-flash".into());
        prefs.models.hidden.push("cohere/command-r-08-2024".into());
        save_prefs_at(&path, &prefs).expect("save");
        let (back, note) = load_prefs_at(&path);
        assert_eq!(back, prefs);
        assert!(note.is_none());

        fs::write(&path, "{corrupt").unwrap();
        let (fallback, note) = load_prefs_at(&path);
        assert_eq!(fallback, CaduceusPrefs::default());
        assert!(note.is_some());

        fs::write(&path, r#"{"version": 9}"#).unwrap();
        let (fallback, note) = load_prefs_at(&path);
        assert_eq!(fallback, CaduceusPrefs::default());
        assert!(note.unwrap().contains("version 9"));
    }

    fn sample_page() -> Page {
        Page {
            prefs_file: tmp_path("page"),
            state: parse_state(SAMPLE_V2),
            prefs: CaduceusPrefs::default(),
            ..Page::default()
        }
    }

    #[test]
    fn parse_engine_show_extracts_unit_facts() {
        let out = "ActiveState=active\nMainPID=12345\nExecMainStartTimestamp=Thu 2026-11-12 21:58:02 CET\n";
        let (state, pid, since) = parse_engine_show(out);
        assert_eq!(state, "active");
        assert_eq!(pid, "12345");
        assert_eq!(since, "Thu 2026-11-12 21:58:02 CET");
        // inactive engines have MainPID=0 → no pid shown
        let (state, pid, _) = parse_engine_show("ActiveState=inactive\nMainPID=0\n");
        assert_eq!(state, "inactive");
        assert_eq!(pid, "");
    }

    #[test]
    fn views_filter_and_order_by_rank() {
        let mut page = sample_page();
        assert_eq!(
            page.provider_view()
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["dahl", "glm"]
        );
        page.prefs.providers.insert(
            "glm".into(),
            ProviderPrefs {
                rank: Some(0),
                ..Default::default()
            },
        );
        assert_eq!(
            page.provider_view()
                .iter()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>(),
            vec!["glm", "dahl"]
        );
        page.filter = "da".into();
        assert_eq!(page.provider_view().len(), 1);
        assert_eq!(page.provider_view()[0].id, "dahl");
    }

    #[test]
    fn toggles_change_policy_and_persist() {
        let path = tmp_path("toggle");
        let mut page = sample_page();
        page.prefs_file = path.clone();
        page.set_tab(Tab::Providers);
        assert!(page.toggle());
        assert_eq!(page.prefs.routing("dahl"), "off");
        let saved = fs::read_to_string(&path).expect("saved");
        assert!(saved.contains("\"routing\": \"off\""));

        page.set_tab(Tab::Models);
        page.toggle(); // pin
        assert_eq!(
            page.prefs.model_state("glm/glm-4.7-flash"),
            ModelState::Pinned
        );
        page.toggle(); // deny
        assert_eq!(
            page.prefs.model_state("glm/glm-4.7-flash"),
            ModelState::Denied
        );
        page.toggle(); // neutral
        assert_eq!(
            page.prefs.model_state("glm/glm-4.7-flash"),
            ModelState::Neutral
        );

        page.set_tab(Tab::Policy);
        page.sel = 0;
        page.toggle();
        assert!(!page.prefs.free_only);
        page.sel = 1;
        page.adjust(-1);
        assert_eq!(page.prefs.per_owner, 7);
        page.sel = 2;
        page.adjust(1000);
        assert_eq!(page.prefs.max_candidates, 256);
    }

    #[test]
    fn rank_adjust_and_clamps() {
        let path = tmp_path("rank");
        let mut page = sample_page();
        page.prefs_file = path;
        page.set_tab(Tab::Providers);
        page.adjust(1);
        assert_eq!(page.prefs.rank("dahl"), Some(0));
        page.adjust(-1);
        assert_eq!(page.prefs.rank("dahl"), Some(1));
        page.adjust(-1000);
        assert_eq!(page.prefs.rank("dahl"), Some(99));
    }

    #[test]
    fn hidden_models_are_display_only() {
        let mut page = sample_page();
        page.set_tab(Tab::Models);
        let id = "glm/glm-4.7-flash";
        assert_eq!(page.candidate_view().len(), 1);
        assert!(page.toggle_hidden());
        assert!(page.prefs.is_hidden(id));
        assert_eq!(page.candidate_view().len(), 0);
        // routing lists are untouched: hidden is not denied/pinned
        assert_eq!(page.prefs.model_state(id), ModelState::Neutral);
        assert!(page.prefs.models.denied.is_empty());
        // H reveals hidden rows again
        page.toggle_show_hidden();
        assert_eq!(page.candidate_view().len(), 1);
        assert!(page.candidate_view()[0].id == id);
        // unhide from the reveal view
        assert!(page.toggle_hidden());
        assert!(!page.prefs.is_hidden(id));
        // H remains a pure view toggle
        assert!(page.show_hidden);
        page.toggle_show_hidden();
        assert!(!page.show_hidden);
    }

    #[test]
    fn selection_clamps_and_tabs_reset() {
        let mut page = sample_page();
        page.set_tab(Tab::Providers);
        page.sel = 99;
        page.clamp_sel();
        assert_eq!(page.sel, 1);
        page.set_tab(Tab::Models);
        assert_eq!(page.sel, 0);
        page.move_sel(10);
        assert_eq!(page.sel, 0); // one candidate
        assert_eq!(Tab::from_number(3), Some(Tab::Health));
        assert_eq!(Tab::from_number(5), Some(Tab::Vitriol));
        assert_eq!(Tab::from_number(0), None);
        assert_eq!(Tab::Policy.next(), Tab::Vitriol);
        assert_eq!(Tab::Vitriol.next(), Tab::Providers);
        assert_eq!(Tab::Providers.prev(), Tab::Vitriol);
        assert_eq!(Tab::Vitriol.prev(), Tab::Policy);
    }

    #[test]
    fn policy_rows_include_enabled_and_pref_owners() {
        let mut page = sample_page();
        page.prefs.providers.insert(
            "dahl".into(),
            ProviderPrefs {
                allow_paid: Some(true),
                ..Default::default()
            },
        );
        let rows = page.policy_rows();
        assert!(matches!(rows[0], PolicyRow::FreeOnly));
        assert!(matches!(rows[1], PolicyRow::PerOwner));
        assert!(matches!(rows[2], PolicyRow::MaxCandidates));
        assert!(rows.iter().any(
            |r| matches!(r, PolicyRow::AllowPaid { provider, allow: true } if provider == "dahl")
        ));
        assert!(rows
            .iter()
            .any(|r| matches!(r, PolicyRow::AllowPaid { provider, .. } if provider == "glm")));
    }
}
