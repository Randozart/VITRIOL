// Application state — chat entries, widgets, agent state, markdown cache.

use std::collections::HashMap;

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;

use crate::rpc::protocol::{Model, RpcCommand};
use crate::theme;

// ── Tool verbosity (owner request 2026-09-03) ────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolVerbosity {
    Line = 0,
    Block = 1,
    Full = 2,
}

impl ToolVerbosity {
    pub const ALL: [ToolVerbosity; 3] = [
        ToolVerbosity::Line,
        ToolVerbosity::Block,
        ToolVerbosity::Full,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolVerbosity::Line => "line",
            ToolVerbosity::Block => "block",
            ToolVerbosity::Full => "full",
        }
    }

    pub fn parse(s: &str) -> Option<ToolVerbosity> {
        Self::ALL.iter().copied().find(|m| m.label() == s)
    }

    pub fn next(self) -> ToolVerbosity {
        let i = Self::ALL.iter().position(|m| *m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn max(self, other: ToolVerbosity) -> ToolVerbosity {
        if self > other {
            self
        } else {
            other
        }
    }

    pub fn min(self, other: ToolVerbosity) -> ToolVerbosity {
        if self < other {
            self
        } else {
            other
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// Always this mode, ignores global.
    Pinned,
    /// At-least: effective = max(global, mode).
    AtLeast,
    /// At-most: effective = min(global, mode).
    AtMost,
}

#[derive(Debug, Clone, Copy)]
pub struct ToolOverride {
    pub mode: ToolVerbosity,
    pub strictness: Strictness,
}

impl ToolOverride {
    /// Encode as "block!" / "block+" / "block-" for persistence.
    pub fn encode(self) -> String {
        let suffix = match self.strictness {
            Strictness::Pinned => "!",
            Strictness::AtLeast => "+",
            Strictness::AtMost => "-",
        };
        format!("{}{}", self.mode.label(), suffix)
    }

    /// Parse "block!" / "block+" / "block-"
    pub fn parse(s: &str) -> Option<ToolOverride> {
        let (mode_s, suffix) = if s.ends_with('!') {
            (&s[..s.len() - 1], '!')
        } else if s.ends_with('+') {
            (&s[..s.len() - 1], '+')
        } else if s.ends_with('-') {
            (&s[..s.len() - 1], '-')
        } else {
            (s, '+') // bare mode = at-least (the user's default)
        };
        let mode = ToolVerbosity::parse(mode_s)?;
        let strictness = match suffix {
            '!' => Strictness::Pinned,
            '+' => Strictness::AtLeast,
            '-' => Strictness::AtMost,
            _ => Strictness::AtLeast,
        };
        Some(ToolOverride { mode, strictness })
    }
}

/// Resolve effective tool verbosity given global default + per-tool overrides.
pub fn effective_mode(
    tool_name: &str,
    global: ToolVerbosity,
    overrides: &HashMap<String, ToolOverride>,
) -> ToolVerbosity {
    match overrides.get(tool_name) {
        None => global,
        Some(o) => match o.strictness {
            Strictness::Pinned => o.mode,
            Strictness::AtLeast => global.max(o.mode),
            Strictness::AtMost => global.min(o.mode),
        },
    }
}

/// Known pi tool names for the modal picker.
pub const KNOWN_TOOLS: &[&str] = &[
    "bash",
    "read",
    "write",
    "edit",
    "find",
    "grep",
    "ls",
    "scratchpad_write",
    "update_tasks",
];

// ── /help modal (owner request 2026-09-03) ───────────────────────────

/// Sidebar glossary — every row explained. DRIFT GUARD (standing rule,
/// plan officina-sidebar-declutter-help-2026-09-03): keep in sync with
/// officina/.pi/extensions/session-panel/index.ts — any sidebar content
/// change must touch this up.
pub const SIDEBAR_GLOSSARY: &[(&str, &str)] = &[
    (
        "◈ <name>",
        "coupling — the engine endpoint identity (gold anchor)",
    ),
    (
        "ctx",
        "context window fill: tokens used / window size + percent",
    ),
    (
        "eng",
        "live engine rate (tok/s / idle); 'N decoded this boot' = engine decode total",
    ),
    (
        "[>] / [ ] / ✓ / ✗",
        "Todo: in-progress / pending / done / cancelled counts",
    ),
    (
        "[>] <item>",
        "Todo: one row per task (open first, then history)",
    ),
    (
        "facts / context / leads / dead / lines",
        "Notebook: one count per row + every open line",
    ),
    (
        "file +n −n",
        "recently touched files: lines added / deleted",
    ),
    (
        "in / out / turns",
        "session tokens in, generated out, prompts",
    ),
    ("cache read / write / hit", "prompt-cache activity"),
    ("tools", "recently used tools"),
    (
        "ref · n injected",
        "knowledge documents injected into context",
    ),
    ("header center", "the model id behind this session"),
    ("header right", "session name or id @ working folder"),
    (
        "silver column",
        "quicksilver scroll gauge — fills as you scroll up",
    ),
    (
        "gauge colors",
        "teal = ctx fill · silver = idle · green/cyan = decoding",
    ),
];

/// /help key reference.
pub const HELP_KEYS: &[(&str, &str)] = &[
    (
        "enter",
        "send the message (steers while the agent is running)",
    ),
    ("alt+enter", "queue a follow-up instead of steering"),
    ("ctrl+q", "toggle the plan agent (also /plan)"),
    ("F2", "toggle deep research (also /deep-research)"),
    ("tab", "autocomplete / cycle agent"),
    ("esc", "abort streaming · close overlays — NEVER quits"),
    (
        "drag",
        "select transcript text (left button across the chat)",
    ),
    ("^c", "copy — the selection, or the last reply"),
    ("^esc", "quit (the only quit chord — /quit also works)"),
    ("pgup/pgdn", "scroll back through history (wheel = 3 rows)"),
    (
        "^↑ / ^↓",
        "scroll the sidebar independently (wheel over the panel)",
    ),
    (
        "home/end",
        "oldest line / live tail (when the input is empty)",
    ),
    ("^v", "cycle tool verbosity (line/block/full)"),
    ("^t", "tool verbosity picker"),
    ("F9", "stderr diagnostics overlay"),
    ("F1", "this help"),
];

/// One /help modal line. Header rows render as gold section titles;
/// item rows as `left` (cyan, fixed width) + `right` (text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpRowKind {
    Header,
    Item,
}

#[derive(Debug, Clone)]
pub struct HelpRow {
    pub kind: HelpRowKind,
    pub left: String,
    pub right: String,
}

/// The full /help content: sidebar glossary, keys, then every local
/// command (rendered FROM LOCAL_COMMANDS so it cannot drift).
pub fn help_rows() -> Vec<HelpRow> {
    let mut rows: Vec<HelpRow> = Vec::new();
    let mut push = |kind: HelpRowKind, left: &str, right: &str| {
        rows.push(HelpRow {
            kind,
            left: left.to_string(),
            right: right.to_string(),
        });
    };
    push(HelpRowKind::Header, "SIDEBAR", "");
    for (l, r) in SIDEBAR_GLOSSARY {
        push(HelpRowKind::Item, l, r);
    }
    push(HelpRowKind::Header, "KEYS", "");
    for (l, r) in HELP_KEYS {
        push(HelpRowKind::Item, l, r);
    }
    push(HelpRowKind::Header, "COMMANDS", "");
    for (name, _short, long) in AppState::LOCAL_COMMANDS {
        push(HelpRowKind::Item, &format!("/{name}"), long);
    }
    rows
}

/// Output cap at ingest (sliced at render for Block/Line).
const TOOL_OUTPUT_CAP: usize = 2000;
/// Block preview lines.
const TOOL_OUTPUT_PREVIEW: usize = 30;

/// Seconds of zero-output before a running tool gets a pipe-buffer hint.
/// Commands like `cargo test 2>&1 | tail -30` buffer everything until EOF —
/// from the chair that reads as a hang, so after this window we say so.
const NO_OUTPUT_HINT_SECS: u64 = 30;

/// One rendered chat entry.
#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    Thinking(String),
    Tool {
        tool_call_id: Option<String>,
        name: String,
        summary: String,
        args: Option<serde_json::Value>,
        output: Vec<String>,
        output_truncated: bool,
        running: bool,
        error: bool,
        /// Wall-clock start — drives the running elapsed timer + pipe-buffer
        /// hint (owner request 2026-09-07: a 30-min bash with buffered
        /// output reads as a hang).
        started_at: Option<std::time::Instant>,
        /// Declared tool timeout from args (`bash` `timeout: 1800` → "30m").
        timeout_s: Option<u64>,
        /// Cached Block/Full rendering: (width, tools_gen, output_len, tick, lines).
        /// Invalidated when any component changes — streaming output grows,
        /// config toggles bump tools_gen, and the elapsed tick advances
        /// once per second while the tool is running.
        #[allow(clippy::type_complexity)]
        render_cache: Option<(usize, u64, usize, u64, Vec<Line<'static>>)>,
    },
    /// Diagnostic notice (extension errors, retries, stderr surfacing).
    Diag(String),
}

#[derive(Debug, Clone, Default)]
pub struct WidgetBlock {
    pub key: String,
    pub lines: Vec<String>, // raw ANSI — parsed at render time
    /// pi `widgetPlacement`: "aboveEditor" (sidebar content) or
    /// "belowEditor" (composer-adjacent status strip).
    pub placement: Option<String>,
}

/// Pending interactive dialog requested by an extension (RPC
/// `extension_ui_request`). The Rust TUI used to auto-cancel these; now they
/// render as modals and reply with the documented response payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogKind {
    Confirm,
    Select,
    Input,
    Editor,
}

#[derive(Debug, Clone)]
pub struct Dialog {
    pub id: String,
    pub kind: DialogKind,
    pub title: String,
    pub message: Option<String>,
    pub options: Vec<String>,
    pub selected: usize,
    pub input: String,
    pub cursor: usize,
}

impl Dialog {
    pub fn confirm(id: String, title: String, message: Option<String>) -> Self {
        Self {
            id,
            kind: DialogKind::Confirm,
            title,
            message,
            options: vec!["Yes".to_string(), "No".to_string()],
            selected: 0,
            input: String::new(),
            cursor: 0,
        }
    }

    pub fn select(id: String, title: String, options: Vec<String>) -> Self {
        Self {
            id,
            kind: DialogKind::Select,
            title,
            message: None,
            options,
            selected: 0,
            input: String::new(),
            cursor: 0,
        }
    }

    pub fn input(id: String, kind: DialogKind, title: String, prefill: String) -> Self {
        let cursor = prefill.chars().count();
        Self {
            id,
            kind,
            title,
            message: None,
            options: Vec::new(),
            selected: 0,
            input: prefill,
            cursor,
        }
    }

    pub fn cursor_byte(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    pub fn insert_char(&mut self, c: char) {
        let at = self.cursor_byte();
        self.input.insert(at, c);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.cursor_byte();
        let start = self
            .input
            .char_indices()
            .nth(self.cursor - 1)
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.input.chars().count() {
            return;
        }
        let start = self.cursor_byte();
        let end = self
            .input
            .char_indices()
            .nth(self.cursor + 1)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len());
        self.input.replace_range(start..end, "");
    }

    pub fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.input.chars().count());
    }
}

#[allow(dead_code)]
pub struct AppState {
    pub entries: Vec<ChatEntry>,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub model: Option<Model>,
    pub thinking_level: Option<String>,
    pub session_id: String,
    pub session_name: Option<String>,
    pub message_count: u64,

    // Widget blocks from extension_ui_request (setWidget) — session-panel,
    // decode gauge, scratchpad note, etc. Rendered in arrival order.
    pub widgets: Vec<WidgetBlock>,

    // Terminal title (setTitle)
    pub title: String,

    // Notifications (notify)
    pub notice: Option<(String, String)>, // (message, type)

    // Slash commands (get_commands) — autocomplete source
    pub commands: Vec<crate::rpc::protocol::SlashCommand>,

    // /resume picker modal
    pub resume_open: bool,
    pub resume_entries: Vec<crate::rpc::sessions::SessionEntry>,
    pub resume_sel: usize,

    // /model picker modal (owner request 2026-11-10 — parity with the JS
    // TUI's selector instead of blind cycle).
    pub models_open: bool,
    pub models: Vec<Model>,
    pub models_sel: usize,
    pub models_filter: String,

    // /cache manager modal (owner request 2026-11-10): list the engine KV
    // disk cache with ages/sizes, delete one entry, or prune older than X.
    /// Extension dialogs awaiting a response, oldest first.
    pub dialogs: std::collections::VecDeque<Dialog>,

    /// /permissions modal + active mode (`ask|auto|yolo`).
    pub perm_modal_open: bool,
    pub perm_modal_sel: usize,
    pub perm_mode: String,

    pub cache_open: bool,
    pub cache_entries: Vec<crate::tui::cache::CacheEntry>,
    pub cache_sel: usize,
    pub cache_dir: String,
    pub cache_total: u64,

    // Diagnostics overlay (F9)
    pub show_diag: bool,
    /// Snapshot of the bridge stderr ring, refreshed each frame by run_loop.
    pub diag_view: Vec<String>,

    // Input
    pub input: String,
    pub cursor: usize,
    /// Selected index into command_candidates() (↑↓ navigation).
    pub cand_sel: usize,

    /// Working directory — session scan root for /resume.
    pub cwd: std::path::PathBuf,

    /// Agent mode tracking — the widget's own "/mode <next>" hint (Tab).
    pub agent_mode: Option<String>,
    /// Current mode label (e.g. "BUILD") + badge glyph, parsed from the same
    /// widget — drives the header mode chip (owner request 2026-09-02).
    pub agent_mode_label: Option<String>,
    pub agent_mode_glyph: Option<String>,
    #[allow(dead_code)]
    pub agent_modes: Vec<String>,

    // UI state
    pub should_quit: bool,

    /// Watermark glimmer animation mode (owner request 2026-09-02) —
    /// /glimmer cycles it; persistence lives in the run loop.
    pub glimmer: crate::watermark::GlimmerMode,
    /// Composer fire (owner request 2026-09-02): GPU-load-driven braille
    /// flames above the prompt box. `fire_target` is the last `engine-fire`
    /// widget reading; `fire_level` is the low-passed display value;
    /// `fire_on` is the /fire toggle; `fire_style` the color voice
    /// (default emerald — owner request, same session).
    pub fire_on: bool,
    pub fire_style: crate::fire::FireStyle,
    /// Whether standing text catches the flame color (owner request
    /// 2026-09-03: "a config setting whether to discolor the text, or
    /// leave text their original color"). Default on — the discolor
    /// effect is the original beloved behavior.
    pub fire_tint: bool,
    pub fire_target: f64,
    pub fire_level: f64,

    // Tool verbosity (owner request 2026-09-03)
    pub tool_default: ToolVerbosity,
    pub tool_overrides: HashMap<String, ToolOverride>,
    /// Generation counter — bumped on any config change; invalidates render caches.
    pub tools_gen: u64,
    // /tools modal picker
    pub tools_modal_open: bool,
    pub tools_modal_sel: usize,
    // /help modal (F1)
    pub help_open: bool,
    pub help_sel: usize,

    /// Compaction auto-continue guard (owner choice B, 2026-09-03): armed
    /// on compaction_start, consumed by ONE threshold compaction_end.
    pub compaction_auto_armed: bool,

    /// Tool-call watchdog (2026-11-10): a tool that never emits
    /// `tool_execution_end` used to leave the TUI "streaming" forever, so
    /// later prompts became RPC `Steer` messages queued behind the stuck
    /// turn and were never persisted. 0 disables the watchdog.
    pub tool_watchdog_secs: u64,
    /// Tool call ids already watched/aborted once this turn.
    pub watchdog_fired: std::collections::HashSet<String>,
    /// Set after a watchdog abort; if the agent has not settled by this
    /// instant the TUI surfaces a restart hint instead of looking hung.
    pub abort_deadline: Option<std::time::Instant>,
    /// Prompts typed while a tool is still running. They are delivered after
    /// the turn settles instead of being sent as RPC `Steer` behind a tool
    /// that may never finish (and would then never be persisted).
    pub queued_prompts: Vec<String>,
    /// Tool calls announced by the assistant turn but not yet observed via
    /// `tool_execution_start`. Covers the failure where pi persists the
    /// assistant tool call and then never starts the tool (the TUI otherwise
    /// has nothing to watch).
    pub expected_tool_calls: std::collections::HashMap<String, std::time::Instant>,

    // Text selection (owner request 2026-09-03): drag with the left
    // button across the transcript; ^c copies the selected rows. Screen
    // coords; clamped against last_chat_area at render/extract time.
    pub sel_anchor: Option<(u16, u16)>,
    pub sel_head: Option<(u16, u16)>,
    /// Chat viewport of the last rendered frame — lets handle_key map
    /// selection rows to transcript lines without a Frame.
    pub last_chat_area: Option<ratatui::layout::Rect>,
    /// Sidebar viewport of the last rendered frame — wheel events over this
    /// rect scroll the sidebar independently of the chat (2026-11-10).
    pub last_sidebar_area: Option<ratatui::layout::Rect>,
    /// Sidebar scroll offset in rows, and the clamp for the last render.
    pub sidebar_scroll: u16,
    pub sidebar_scroll_max: u16,
    /// Sidebar visibility override: None = auto (hide on narrow terminals),
    /// Some(true) = force show, Some(false) = force hide.
    pub sidebar_force: Option<bool>,
    /// Session clock — drives the glimmer phase (elapsed ms).
    pub started: std::time::Instant,

    /// Scrollback: rows scrolled back from the live tail (owner request
    /// 2026-09-02 — PgUp/PgDn/wheel through session history). 0 = pinned
    /// to the newest output. Clamped to `scroll_max` by the chat renderer.
    pub scroll: u16,
    pub scroll_max: u16,

    // Markdown render cache: (entry_index, text_len, width) → lines.
    // Finished entries parse once; the growing tail re-renders per frame.
    md_cache: HashMap<(usize, usize, usize), Vec<Line<'static>>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            is_streaming: false,
            is_compacting: false,
            model: None,
            thinking_level: None,
            session_id: String::new(),
            session_name: None,
            message_count: 0,
            widgets: Vec::new(),
            title: String::new(),
            notice: None,
            commands: Vec::new(),
            resume_open: false,
            resume_entries: Vec::new(),
            resume_sel: 0,
            models_open: false,
            models: Vec::new(),
            models_sel: 0,
            models_filter: String::new(),
            dialogs: std::collections::VecDeque::new(),
            perm_modal_open: false,
            perm_modal_sel: 0,
            perm_mode: "ask".to_string(),
            cache_open: false,
            cache_entries: Vec::new(),
            cache_sel: 0,
            cache_dir: String::new(),
            cache_total: 0,
            show_diag: false,
            diag_view: Vec::new(),
            input: String::new(),
            cursor: 0,
            cand_sel: 0,
            cwd: std::path::PathBuf::from("."),
            agent_mode: None,
            agent_mode_label: None,
            agent_mode_glyph: None,
            agent_modes: Vec::new(),
            should_quit: false,
            glimmer: crate::watermark::GlimmerMode::Shimmer,
            fire_on: true,
            fire_style: crate::fire::FireStyle::default(),
            fire_tint: true,
            fire_target: 0.0,
            fire_level: 0.0,
            tool_default: ToolVerbosity::Line,
            tool_overrides: HashMap::new(),
            tools_gen: 0,
            tools_modal_open: false,
            tools_modal_sel: 0,
            help_open: false,
            help_sel: 0,
            compaction_auto_armed: false,
            tool_watchdog_secs: 300,
            watchdog_fired: std::collections::HashSet::new(),
            abort_deadline: None,
            queued_prompts: Vec::new(),
            expected_tool_calls: std::collections::HashMap::new(),
            sel_anchor: None,
            sel_head: None,
            last_chat_area: None,
            last_sidebar_area: None,
            sidebar_scroll: 0,
            sidebar_scroll_max: 0,
            sidebar_force: None,
            started: std::time::Instant::now(),
            scroll: 0,
            scroll_max: 0,
            md_cache: HashMap::new(),
        }
    }
}

impl AppState {
    /// Models matching the picker filter (case-insensitive, AND of words
    /// across provider/id/name). Empty filter = all models.
    pub fn filtered_models(&self) -> Vec<&Model> {
        let needle = self.models_filter.trim().to_lowercase();
        self.models
            .iter()
            .filter(|m| {
                if needle.is_empty() {
                    return true;
                }
                let name = m.name.as_deref().unwrap_or("");
                let hay = format!("{} {} {}", m.provider, m.id, name).to_lowercase();
                needle.split_whitespace().all(|w| hay.contains(w))
            })
            .collect()
    }

    /// Keep the selection inside the filtered list after a filter edit.
    pub fn models_clamp_selection(&mut self) {
        let len = self.filtered_models().len();
        if len == 0 {
            self.models_sel = 0;
        } else if self.models_sel >= len {
            self.models_sel = len - 1;
        }
    }

    // ── /permissions ─────────────────────────────────────────────────────

    pub const PERM_MODES: &'static [(&'static str, &'static str)] = &[
        (
            "ask",
            "prompt for risky shell + outside-project writes (recommended)",
        ),
        ("auto", "allow risky shell; sensitive paths still prompt"),
        ("yolo", "allow everything EXCEPT sensitive roots/secrets"),
    ];

    pub fn permissions_mode_index(&self) -> usize {
        Self::PERM_MODES
            .iter()
            .position(|(mode, _)| *mode == self.perm_mode)
            .unwrap_or(0)
    }

    pub fn open_permissions_modal(&mut self) {
        self.perm_mode = crate::envfile::read_permissions_mode();
        self.perm_modal_sel = self.permissions_mode_index();
        self.perm_modal_open = true;
    }

    pub fn set_permissions_mode(&mut self, mode: &str) {
        match crate::envfile::write_permissions_mode(mode) {
            Ok(()) => {
                self.perm_mode = mode.to_string();
                self.notice = Some((format!("permissions: {mode}"), "info".to_string()));
            }
            Err(e) => self.entries.push(ChatEntry::Diag(format!(
                "permissions: could not persist mode: {e}"
            ))),
        }
    }

    pub fn toggle_yolo(&mut self) {
        let next = if self.perm_mode == "yolo" {
            "ask"
        } else {
            "yolo"
        };
        self.set_permissions_mode(next);
    }

    // ── /cache manager ───────────────────────────────────────────────────

    pub fn cache_refresh(&mut self) {
        let dir = crate::tui::cache::cache_dir();
        self.cache_dir = dir.to_string_lossy().to_string();
        self.cache_entries = crate::tui::cache::scan(&dir);
        self.cache_total = crate::tui::cache::total_size(&self.cache_entries);
        if self.cache_sel >= self.cache_entries.len() {
            self.cache_sel = self.cache_entries.len().saturating_sub(1);
        }
    }

    pub fn cache_open_modal(&mut self) {
        self.cache_refresh();
        self.cache_sel = 0;
        self.cache_open = true;
    }

    pub fn cache_delete_selected(&mut self) {
        let Some(entry) = self.cache_entries.get(self.cache_sel).cloned() else {
            return;
        };
        if crate::tui::cache::is_protected(&entry.name) {
            self.entries.push(ChatEntry::Diag(
                "cache: slot0.bin is the warm-boot checkpoint — protected".to_string(),
            ));
            return;
        }
        let size = crate::tui::cache::fmt_bytes(entry.size);
        if crate::tui::cache::delete(&entry) {
            self.entries.push(ChatEntry::Diag(format!(
                "cache: removed {} ({})",
                entry.name, size
            )));
            self.cache_refresh();
        } else {
            self.entries.push(ChatEntry::Diag(format!(
                "cache: could not remove {}",
                entry.name
            )));
        }
    }

    /// `wipe [age]` — no age wipes every non-protected entry. Never deletes
    /// slot0.bin (the warm-boot restore checkpoint).
    pub fn cache_wipe(&mut self, age_text: Option<&str>) {
        let age = match age_text {
            Some(text) => match crate::tui::cache::parse_age(text) {
                Some(age) => Some(age),
                None => {
                    self.entries.push(ChatEntry::Diag(
                        "cache: age must look like 30m, 12h, 7d, or 4w".to_string(),
                    ));
                    return;
                }
            },
            None => None,
        };
        let dir = crate::tui::cache::cache_dir();
        let (count, bytes) = crate::tui::cache::wipe(&dir, age, false);
        let scope = age_text
            .map(|t| format!(" older than {t}"))
            .unwrap_or_default();
        self.entries.push(ChatEntry::Diag(format!(
            "cache: removed {}{} ({}) — slot0.bin protected",
            count,
            scope,
            crate::tui::cache::fmt_bytes(bytes)
        )));
        self.cache_refresh();
    }

    /// `/cache` command surface. Bare opens the manager modal.
    pub fn cache_command(&mut self, args: &str) {
        let args = args.trim();
        if args.is_empty() {
            self.cache_open_modal();
            return;
        }
        let mut parts = args.split_whitespace();
        match parts.next().unwrap_or("") {
            "wipe" | "prune" | "clear" => self.cache_wipe(parts.next()),
            "list" => {
                self.cache_refresh();
                self.entries.push(ChatEntry::Diag(format!(
                    "cache: {} entries · {} · {}",
                    self.cache_entries.len(),
                    crate::tui::cache::fmt_bytes(self.cache_total),
                    self.cache_dir
                )));
            }
            other => self.entries.push(ChatEntry::Diag(format!(
                "cache: unknown action '{other}' — use /cache or /cache wipe [age]"
            ))),
        }
    }

    pub fn apply_state(
        &mut self,
        model: Option<Model>,
        thinking_level: Option<String>,
        is_streaming: bool,
        is_compacting: bool,
        session_id: Option<String>,
        session_name: Option<String>,
        message_count: Option<u64>,
    ) {
        if let Some(m) = model {
            self.model = Some(m);
        }
        self.thinking_level = thinking_level;
        self.is_streaming = is_streaming;
        self.is_compacting = is_compacting;
        self.session_id = session_id.unwrap_or_default();
        self.session_name = session_name;
        self.message_count = message_count.unwrap_or(0);
    }

    /// Append a text delta to the last assistant entry (or open a new one).
    pub fn push_text_delta(&mut self, delta: &str) {
        match self.entries.last_mut() {
            Some(ChatEntry::Assistant(t)) => t.push_str(delta),
            _ => self.entries.push(ChatEntry::Assistant(delta.to_string())),
        }
    }

    /// Append a thinking delta to the last thinking entry (or open a new one).
    pub fn push_thinking_delta(&mut self, delta: &str) {
        match self.entries.last_mut() {
            Some(ChatEntry::Thinking(t)) => t.push_str(delta),
            _ => self.entries.push(ChatEntry::Thinking(delta.to_string())),
        }
    }

    pub fn push_user(&mut self, text: String) {
        self.entries.push(ChatEntry::User(text));
    }

    /// Tool execution started — add a running tool entry with full args.
    pub fn tool_start(&mut self, tool_call_id: Option<&str>, name: &str, args: &serde_json::Value) {
        let summary = summarize_args(name, args);
        let timeout_s = args.get("timeout").and_then(|v| v.as_u64()).or_else(|| {
            args.get("timeout")
                .and_then(|v| v.as_i64())
                .and_then(|i| u64::try_from(i).ok())
        });
        self.entries.push(ChatEntry::Tool {
            tool_call_id: tool_call_id.map(String::from),
            name: name.to_string(),
            summary,
            args: Some(args.clone()),
            output: Vec::new(),
            output_truncated: false,
            running: true,
            error: false,
            started_at: Some(std::time::Instant::now()),
            timeout_s,
            render_cache: None,
        });
    }

    /// Live streaming update from tool_execution_update — appends partial
    /// output to the matching running entry (bash live output, etc.).
    pub fn tool_update(&mut self, tool_call_id: Option<&str>, name: &str, text: &str) {
        if text.is_empty() {
            return;
        }
        for e in self.entries.iter_mut().rev() {
            if let ChatEntry::Tool {
                tool_call_id: ref id,
                name: n,
                output,
                output_truncated,
                running,
                ..
            } = e
            {
                // Both sides have ids → ids must match (same-name tools run
                // concurrently). Legacy events without an id → name fallback.
                let matched = match (tool_call_id, id.as_deref()) {
                    (Some(cid), Some(eid)) => cid == eid,
                    (Some(_), None) => false,
                    (None, _) => n == name,
                };
                if matched && *running {
                    for line in text.lines() {
                        if output.len() < TOOL_OUTPUT_CAP {
                            output.push(line.to_string());
                        } else {
                            *output_truncated = true;
                        }
                    }
                    return;
                }
            }
        }
    }

    /// Tool execution finished — match by toolCallId first, name fallback.
    pub fn tool_end(
        &mut self,
        tool_call_id: Option<&str>,
        name: &str,
        result_text: &str,
        error: bool,
    ) {
        // Append result text to output.
        if !result_text.is_empty() {
            self.tool_update(tool_call_id, name, result_text);
        }
        // Mark complete.
        for e in self.entries.iter_mut().rev() {
            if let ChatEntry::Tool {
                tool_call_id: ref id,
                name: n,
                running,
                error: e_err,
                ..
            } = e
            {
                // Same matching rule as tool_update: ids when both sides
                // have them, name only as legacy fallback.
                let matched = match (tool_call_id, id.as_deref()) {
                    (Some(cid), Some(eid)) => cid == eid,
                    (Some(_), None) => false,
                    (None, _) => n == name,
                };
                if matched && *running {
                    *running = false;
                    *e_err = error;
                    return;
                }
            }
        }
    }

    /// Oldest still-running tool: stable key, display name, age, declared
    /// timeout. Used by the stall watchdog.
    pub fn oldest_running_tool(
        &self,
    ) -> Option<(String, String, std::time::Duration, Option<u64>)> {
        let now = std::time::Instant::now();
        self.entries
            .iter()
            .filter_map(|e| match e {
                ChatEntry::Tool {
                    tool_call_id,
                    name,
                    running: true,
                    started_at,
                    timeout_s,
                    ..
                } => {
                    let started = started_at.unwrap_or(now);
                    let key = tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("name:{name}"));
                    Some((key, name.clone(), now.duration_since(started), *timeout_s))
                }
                _ => None,
            })
            .max_by_key(|(_, _, age, _)| *age)
    }

    /// True while any tool call is still awaiting `tool_execution_end`.
    pub fn has_running_tool(&self) -> bool {
        self.entries
            .iter()
            .any(|e| matches!(e, ChatEntry::Tool { running: true, .. }))
    }

    /// Record tool calls announced by an assistant `message_end`. They move
    /// out of this map when their `tool_execution_start` arrives.
    pub fn note_expected_tool_calls(&mut self, content: &serde_json::Value) {
        let Some(parts) = content.as_array() else {
            return;
        };
        let now = std::time::Instant::now();
        for part in parts {
            if part.get("type").and_then(serde_json::Value::as_str) != Some("toolCall") {
                continue;
            }
            if let Some(id) = part.get("id").and_then(serde_json::Value::as_str) {
                self.expected_tool_calls
                    .entry(id.to_string())
                    .or_insert(now);
            }
        }
    }

    pub fn note_tool_started(&mut self, tool_call_id: Option<&str>) {
        if let Some(id) = tool_call_id {
            self.expected_tool_calls.remove(id);
        }
    }

    pub fn note_tool_finished(&mut self, tool_call_id: Option<&str>) {
        if let Some(id) = tool_call_id {
            self.expected_tool_calls.remove(id);
        }
    }

    /// Oldest announced-but-never-started tool call.
    pub fn oldest_expected_tool_call(&self) -> Option<(String, std::time::Duration)> {
        let now = std::time::Instant::now();
        self.expected_tool_calls
            .iter()
            .map(|(id, at)| (id.clone(), now.duration_since(*at)))
            .max_by_key(|(_, age)| *age)
    }

    /// Mark every still-running tool as failed. Called when the agent turn
    /// ends without a `tool_execution_end` — a dangling tool call must not
    /// leave a permanent spinner or a streaming flag.
    pub fn abandon_running_tools(&mut self, reason: &str) -> usize {
        let mut n = 0;
        for e in self.entries.iter_mut() {
            if let ChatEntry::Tool {
                output,
                running,
                error,
                ..
            } = e
            {
                if *running {
                    *running = false;
                    *error = true;
                    if output.last().map(String::as_str) != Some(reason) {
                        output.push(reason.to_string());
                    }
                    n += 1;
                }
            }
        }
        n
    }

    /// Insert or update a widget block by key.
    pub fn set_widget(&mut self, key: &str, lines: Vec<String>) {
        self.set_widget_placed(key, lines, None);
    }

    pub fn set_widget_placed(&mut self, key: &str, lines: Vec<String>, placement: Option<String>) {
        if key == "agent-mode" {
            self.parse_agent_mode(&lines);
        }
        if key == "engine-fire" {
            // Machine-readable load line from vitriol-decode: "FIRE 0.731".
            // Not sidebar content — the flames are the display.
            for line in &lines {
                let plain = strip_ansi(line);
                if let Some(rest) = plain.strip_prefix("FIRE ") {
                    if let Ok(v) = rest.trim().parse::<f64>() {
                        self.fire_target = v.clamp(0.0, 1.0);
                    }
                }
            }
            return;
        }
        if let Some(w) = self.widgets.iter_mut().find(|w| w.key == key) {
            w.lines = lines;
            if placement.is_some() {
                w.placement = placement;
            }
        } else {
            self.widgets.push(WidgetBlock {
                key: key.to_string(),
                lines,
                placement,
            });
        }
    }

    /// Parse the agent-mode widget. The line format (agent-mode.ts) is
    /// "▪ BUILD · hint · TAB / /agent <next>" (loud directive agents:
    /// "► PLAN AGENT — hint · TAB / /agent <next>") — the trailing hint names
    /// the NEXT mode directly, which is exactly what Tab needs, and the
    /// second whitespace token is the CURRENT mode label (owner request
    /// 2026-09-02: header mode chip). (There is no "agent mode: X" text in
    /// the widget; that string only appears in notify footers.)
    fn parse_agent_mode(&mut self, lines: &[String]) {
        for line in lines {
            let plain = strip_ansi(line);
            // Current mode: first token is the badge glyph, second the label.
            let mut tokens = plain.split_whitespace();
            if let (Some(glyph), Some(label)) = (tokens.next(), tokens.next()) {
                let label = label
                    .trim_end_matches("MODE")
                    .trim_end_matches("AGENT")
                    .trim();
                if !label.is_empty()
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    self.agent_mode_label = Some(label.to_string());
                    self.agent_mode_glyph = Some(glyph.to_string());
                }
            }
            // Next agent: the trailing bare word after "/agent " (legacy
            // "/mode " accepted for older agent-mode widget lines).
            let next = plain
                .rfind("/agent ")
                .map(|idx| plain[idx + "/agent ".len()..].trim().to_string())
                .or_else(|| {
                    plain
                        .rfind("/mode ")
                        .map(|idx| plain[idx + "/mode ".len()..].trim().to_string())
                });
            if let Some(next) = next {
                // Token must be a bare mode name (letters/dashes).
                if !next.is_empty()
                    && next
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    self.agent_mode = Some(next);
                    return;
                }
            }
        }
    }

    /// Next mode per the widget's own hint (Tab). None until the widget
    /// tells us.
    pub fn next_agent_mode(&self) -> Option<String> {
        self.agent_mode.clone()
    }

    /// Cycle the watermark glimmer (Shimmer → Breathe → Twinkle → Off).
    /// Returns the new mode's label for the notify line.
    pub fn cycle_glimmer(&mut self) -> &'static str {
        self.glimmer = self.glimmer.next();
        self.glimmer.label()
    }

    /// /glimmer handler: bare argument cycles, a named mode sets directly.
    /// Unknown names are ignored (returns current). Returns the new label.
    pub fn set_glimmer(&mut self, args: &str) -> &'static str {
        let args = args.trim();
        if args.is_empty() {
            return self.cycle_glimmer();
        }
        if let Some(m) = crate::watermark::GlimmerMode::parse(args) {
            self.glimmer = m;
        }
        self.glimmer.label()
    }

    /// /fire handler: bare argument toggles, "on"/"off" set directly, a
    /// style name (emerald|alchemy) switches the color voice, bare "style"
    /// cycles it. Returns the new state for the notify line.
    pub fn set_fire(&mut self, args: &str) -> String {
        let args = args.trim();
        match args {
            "" => self.fire_on = !self.fire_on,
            "on" | "1" | "true" => self.fire_on = true,
            "off" | "0" | "false" => {
                self.fire_on = false;
                self.fire_target = 0.0;
            }
            "style" => self.fire_style = self.fire_style.next(),
            // Text discoloration toggle (owner request 2026-09-03) —
            // independent of the flame itself.
            "tint" => {
                self.fire_tint = !self.fire_tint;
                return if self.fire_tint {
                    "tint on".to_string()
                } else {
                    "tint off".to_string()
                };
            }
            word => {
                if let Some(s) = crate::fire::FireStyle::parse(word) {
                    self.fire_style = s;
                    self.fire_on = true;
                }
                // unknown words ignored
            }
        }
        if !self.fire_on {
            "off".to_string()
        } else {
            self.fire_style.label().to_string()
        }
    }

    /// Advance the fire's low-pass — exponential smoothing with a ≈0.33 s
    /// time constant, so the flame breathes at the same tempo regardless of
    /// frame rate.
    pub fn step_fire(&mut self, dt_secs: f64) {
        if !self.fire_on {
            self.fire_target = 0.0;
        }
        let k = 1.0 - (-dt_secs / 0.33).exp();
        self.fire_level += (self.fire_target - self.fire_level) * k;
        if self.fire_level < crate::fire::MIN_LEVEL {
            self.fire_level = 0.0;
        }
    }

    // ── Scrollback (owner request 2026-09-02) ────────────────────────────

    pub fn scroll_up(&mut self, rows: u16) {
        self.scroll = self
            .scroll
            .saturating_add(rows)
            .min(self.scroll_max.max(self.scroll));
    }

    pub fn scroll_down(&mut self, rows: u16) {
        self.scroll = self.scroll.saturating_sub(rows);
    }

    // ── Text selection (owner request 2026-09-03) ────────────────────────

    /// Normalized selection row range (screen coords), anchor/head order-agnostic.
    pub fn selection_rows(&self) -> Option<(u16, u16)> {
        let (a, h) = self.sel_anchor.zip(self.sel_head)?;
        Some(if a.1 <= h.1 { (a.1, h.1) } else { (h.1, a.1) })
    }

    pub fn clear_selection(&mut self) {
        self.sel_anchor = None;
        self.sel_head = None;
    }

    /// Text of the selected transcript rows (whole rendered lines, ANSI
    /// stripped, leading/trailing blank rows dropped). Whole-line granularity:
    /// wrapped continuations copy intact rather than mangled mid-glyph.
    pub fn extract_selection(&mut self) -> Option<String> {
        let area = self.last_chat_area?;
        let (mut y0, mut y1) = self.selection_rows()?;
        y0 = y0.max(area.y);
        y1 = y1.min(area.y + area.height.saturating_sub(1));
        if y1 < y0 {
            return None;
        }
        let width = (area.width as usize).saturating_sub(1); // gauge col excluded
        let lines = self.chat_lines(width);
        let visible = area.height as usize;
        let start = lines.len().saturating_sub(visible + self.scroll as usize);
        let mut out: Vec<String> = Vec::new();
        for row in y0..=y1 {
            let li = start + (row - area.y) as usize;
            if let Some(l) = lines.get(li) {
                out.push(l.to_string().trim_end().to_string());
            }
        }
        while out.first().map(|s| s.is_empty()).unwrap_or(false) {
            out.remove(0);
        }
        while out.last().map(|s| s.is_empty()).unwrap_or(false) {
            out.pop();
        }
        if out.is_empty() {
            None
        } else {
            Some(out.join("\n"))
        }
    }

    /// Most recent non-empty assistant reply — ^c fallback with no selection.
    pub fn last_assistant(&self) -> Option<String> {
        self.entries.iter().rev().find_map(|e| match e {
            ChatEntry::Assistant(t) if !t.trim().is_empty() => Some(t.clone()),
            _ => None,
        })
    }

    /// Jump to the oldest visible content (renderer clamps to scroll_max).
    pub fn scroll_home(&mut self) {
        self.scroll = u16::MAX;
    }

    /// Back to the live tail.
    pub fn scroll_end(&mut self) {
        self.scroll = 0;
    }

    /// Sidebar has its own scrollback (owner request 2026-11-10): the panel
    /// can hold more rows than the screen, so wheel/^↑↓ move it separately
    /// from the transcript.
    pub fn sidebar_scroll_up(&mut self, rows: u16) {
        self.sidebar_scroll = self
            .sidebar_scroll
            .saturating_add(rows)
            .min(self.sidebar_scroll_max);
    }

    pub fn sidebar_scroll_down(&mut self, rows: u16) {
        self.sidebar_scroll = self.sidebar_scroll.saturating_sub(rows);
    }

    /// /sidebar — cycle auto (terminal width) → forced show → forced hide.
    pub fn cycle_sidebar_mode(&mut self) {
        self.sidebar_force = match self.sidebar_force {
            None => Some(true),
            Some(true) => Some(false),
            Some(false) => None,
        };
        let label = match self.sidebar_force {
            None => "auto (hide when narrow)",
            Some(true) => "shown",
            Some(false) => "hidden",
        };
        self.notice = Some((format!("sidebar: {label}"), "info".to_string()));
    }

    /// Drop the markdown render cache (on transcript clear/rehydrate).
    pub fn md_cache_clear(&mut self) {
        self.md_cache.clear();
    }

    /// All chat lines as styled ratatui Lines.
    /// Assistant entries go through the markdown renderer (cached).
    /// Tool entries render per their effective verbosity mode (cached per
    /// entry against width + tools_gen + output length).
    pub fn chat_lines(&mut self, width: usize) -> Vec<Line<'static>> {
        if self.md_cache.len() > 1024 {
            self.md_cache.clear();
        }
        let mut lines: Vec<Line<'static>> = Vec::new();
        let total = self.entries.len();
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            let before = lines.len();
            match entry {
                ChatEntry::User(text) => {
                    for (i, seg) in wrap_text(text, width.saturating_sub(2))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(
                                    format!("{} ", theme::GLYPH_USER),
                                    Style::default().fg(theme::GREEN),
                                ),
                                Span::raw(seg),
                            ]));
                        } else {
                            lines.push(Line::from(vec![Span::raw("  "), Span::raw(seg)]));
                        }
                    }
                }
                ChatEntry::Assistant(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    let key = (idx, text.chars().count(), width);
                    let md = if let Some(cached) = self.md_cache.get(&key) {
                        cached.clone()
                    } else {
                        let rendered = crate::markdown::render(text, width.saturating_sub(6));
                        self.md_cache.insert(key, rendered.clone());
                        rendered
                    };
                    let mut first = true;
                    for ml in md {
                        let mut spans: Vec<Span> = Vec::with_capacity(ml.spans.len() + 1);
                        if first {
                            first = false;
                            spans.push(Span::styled(
                                format!("{} ", theme::GLYPH_AI),
                                Style::default().fg(theme::GOLD),
                            ));
                        } else {
                            spans.push(Span::raw("  ".to_string()));
                        }
                        spans.extend(ml.spans);
                        lines.push(Line::from(spans));
                    }
                }
                ChatEntry::Thinking(text) => {
                    for seg in wrap_text(text, width.saturating_sub(8)) {
                        lines.push(Line::from(vec![
                            Span::styled("  ·    ", Style::default().fg(theme::MUTED)),
                            Span::styled(
                                seg,
                                Style::default()
                                    .fg(theme::MUTED)
                                    .add_modifier(ratatui::style::Modifier::ITALIC),
                            ),
                        ]));
                    }
                }
                ChatEntry::Tool {
                    name,
                    summary,
                    args,
                    output,
                    output_truncated,
                    running,
                    error,
                    started_at,
                    timeout_s,
                    render_cache,
                    ..
                } => {
                    let mode = effective_mode(name, self.tool_default, &self.tool_overrides);
                    if mode == ToolVerbosity::Line {
                        // Collapsed single line — the status-quo rendering,
                        // byte-identical to pre-verbosity builds.
                        let (icon, st) = if *running {
                            (theme::GLYPH_CRUCIBLE, theme::warn())
                        } else if *error {
                            ("✗", theme::crit())
                        } else {
                            ("✓", Style::default().fg(theme::GREEN).bg(theme::BG))
                        };
                        if let Some((title, cmd)) = bash_title_and_cmd(name, args.as_ref()) {
                            // The model's purpose is the primary scan target;
                            // command is secondary (owner request 2026-11-10:
                            // title and command must be different colors).
                            let budget = width.saturating_sub(8);
                            let title_s = trunc_str(title, 40);
                            let used = name.chars().count() + 3 + title_s.chars().count() + 3;
                            let cmd_s = trunc_str(&cmd, budget.saturating_sub(used));
                            let mut spans = vec![
                                Span::styled(format!("  {} ", icon), st),
                                Span::styled(name.clone(), st),
                                Span::styled(
                                    " ▸ ",
                                    Style::default().fg(theme::MUTED).bg(theme::BG),
                                ),
                                Span::styled(
                                    title_s,
                                    Style::default()
                                        .fg(theme::GOLD)
                                        .bg(theme::BG)
                                        .add_modifier(ratatui::style::Modifier::BOLD),
                                ),
                            ];
                            if !cmd_s.is_empty() {
                                spans.push(Span::styled(
                                    " · ",
                                    Style::default().fg(theme::MUTED).bg(theme::BG),
                                ));
                                spans.push(Span::styled(cmd_s, st));
                            }
                            lines.push(Line::from(spans));
                        } else {
                            let body = if summary.is_empty() {
                                name.clone()
                            } else {
                                format!("{} {}", name, summary)
                            };
                            for (i, seg) in wrap_text(&body, width.saturating_sub(8))
                                .into_iter()
                                .enumerate()
                            {
                                if i == 0 {
                                    lines.push(Line::from(vec![
                                        Span::styled(format!("  {} ", icon), st),
                                        Span::styled(seg, st),
                                    ]));
                                } else {
                                    lines.push(Line::from(vec![
                                        Span::raw("        "),
                                        Span::styled(seg, st),
                                    ]));
                                }
                            }
                        }
                    } else {
                        // Block / Full — bordered construction block, cached
                        // against (width, tools_gen, output length, elapsed
                        // tick for running tools → the timer advances once
                        // per second on the heartbeat redraw).
                        let gen = self.tools_gen;
                        let body_w = width.saturating_sub(2).max(20);
                        let olen = output.len();
                        let tick: u64 = if *running {
                            started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
                        } else {
                            0
                        };
                        let fresh = matches!(render_cache,
                            Some((w, g, o, tk, _)) if *w == body_w && *g == gen && *o == olen && *tk == tick);
                        if !fresh {
                            let elapsed = started_at.map(|t| t.elapsed().as_secs());
                            let no_output_hint = *running
                                && output.is_empty()
                                && elapsed.is_some_and(|s| s >= NO_OUTPUT_HINT_SECS);
                            let block = render_tool_block(
                                mode,
                                name,
                                summary,
                                args.as_ref(),
                                output,
                                *output_truncated,
                                *running,
                                *error,
                                body_w,
                                elapsed,
                                *timeout_s,
                                no_output_hint,
                            );
                            *render_cache = Some((body_w, gen, olen, tick, block));
                        }
                        if let Some((_, _, _, _, block)) = render_cache.as_ref() {
                            for bl in block {
                                lines.push(Line::from(
                                    std::iter::once(Span::raw("  "))
                                        .chain(bl.spans.iter().cloned())
                                        .collect::<Vec<Span>>(),
                                ));
                            }
                        }
                    }
                }
                ChatEntry::Diag(text) => {
                    let st = theme::crit();
                    for (i, seg) in wrap_text(text, width.saturating_sub(8))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(format!("  {} ", theme::GLYPH_SULFUR), st),
                                Span::styled(seg, st),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("        "),
                                Span::styled(seg, st),
                            ]));
                        }
                    }
                }
            }
            // Breathing room between transcript entries (owner request
            // 2026-11-10: everything was bunched). Exactly one blank row,
            // never doubled when a renderer already ends blank.
            if lines.len() > before && idx + 1 < total {
                let already_blank = lines
                    .last()
                    .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
                    .unwrap_or(true);
                if !already_blank {
                    lines.push(Line::from(""));
                }
            }
        }
        lines
    }

    /// Local (TUI-side) commands — dispatched to RPC directly, never sent
    /// to pi as prompt text. (name, description)
    /// (name, short — menu row, long — popup detail line + /help).
    /// Shorts stay ≤ ~40 cols; longs carry the full usage.
    pub const LOCAL_COMMANDS: &[(&str, &str, &str)] = &[
        ("cache", "manage the KV cache", "/cache — list engine KV cache entries with age/size; /cache wipe [age] — prune (slot0.bin protected)"),
        ("clear", "clear the transcript", "/clear — wipe the local transcript view (pi keeps the session history)"),
        ("compact", "compact the context", "/compact [hints] — summarize older history now; after it settles, auto-continue picks the work back up"),
        ("diag", "toggle diagnostics", "/diag or F9 — stderr overlay from the agent process"),
        ("fire", "composer flames", "/fire [on|off|style|tint|<voice>] — voices: prismatic, emerald, alchemy; tint toggles whether flames discolor standing text"),
        ("glimmer", "watermark glimmer", "/glimmer [shimmer|breathe|twinkle|off] — bare cycles"),
        ("help", "glossary + keys + commands", "/help or F1 — the reference modal"),
        ("clone", "duplicate the session", "/clone — duplicate the active branch into a fresh session"),
        ("export", "export session to HTML", "/export [path] — write a self-contained HTML transcript"),
        ("rename", "rename the session", "/rename <name> — set the session display name"),
        ("sidebar", "cycle sidebar visibility", "/sidebar — auto (hide when narrow) → show → hide"),
        ("permissions", "permission mode", "/permissions — ask | auto | yolo (sensitive roots always guarded)"),
        ("yolo", "toggle YOLO mode", "/yolo — toggle ask <-> yolo (sensitive roots still prompt)"),
        ("opencode", "import an OpenCode session", "/opencode [latest|<id>|list] [full] — convert an OpenCode session for this project and switch to it"),
        ("model", "select a model", "/model [provider/id] — picker from available models; provider/id switches directly"),
        ("new", "new session", "/new — fresh session (old one stays resumable)"),
        ("quit", "quit the TUI", "/quit — deliberate quit; the only other path is ^esc"),
        ("resume", "resume a session", "/resume — session picker; replays the full transcript including tool calls"),
        ("settings", "UI settings", "/settings [glimmer|fire] <args> — bare lists current values"),
        ("stats", "refresh stats", "/stats — refresh session statistics"),
        ("thinking", "thinking level", "/thinking [off..max] — bare cycles"),
        ("tools", "tool verbosity", "/tools [<tool> <mode>[!+|-] | <tool> clear | default <mode>] — modes: line/block/full; bare opens the picker; ctrl+t too"),
    ];

    /// Slash-command candidates for the current input (empty if not a
    /// command). Merges pi's extension commands with the local registry,
    /// sorted by name.
    pub fn command_candidates(&self) -> Vec<crate::rpc::protocol::SlashCommand> {
        self.command_candidates_for(&self.input)
    }

    /// Candidates for an explicit input string — Enter passes the taken
    /// message here because state.input is already cleared by then.
    pub fn command_candidates_for(&self, input: &str) -> Vec<crate::rpc::protocol::SlashCommand> {
        if !input.starts_with('/') {
            return Vec::new();
        }
        let word = input.split_whitespace().next().unwrap_or("");
        if word.contains(' ') {
            return Vec::new(); // already past the command word
        }
        let prefix = &word[1..]; // strip '/'
        let mut out: Vec<crate::rpc::protocol::SlashCommand> = Vec::new();
        for c in &self.commands {
            if prefix.is_empty() || c.name.starts_with(prefix) {
                out.push(c.clone());
            }
        }
        for (name, short, _long) in Self::LOCAL_COMMANDS {
            if prefix.is_empty() || name.starts_with(prefix) {
                out.push(crate::rpc::protocol::SlashCommand {
                    name: name.to_string(),
                    description: Some(short.to_string()),
                    source: Some("local".to_string()),
                });
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Dispatch a local RPC command. Returns None when the input is not a
    /// local command (or /resume, which the caller opens as a modal).
    pub fn local_dispatch(&mut self, msg: &str) -> Option<RpcCommand> {
        let (word, args) = match msg.split_once(' ') {
            Some((w, a)) => (w.trim_start_matches('/'), a.trim()),
            None => (msg.trim_start_matches('/'), ""),
        };
        match word {
            "new" => Some(RpcCommand::NewSession {
                id: None,
                parent_session: None,
            }),
            "compact" => Some(RpcCommand::Compact {
                id: None,
                custom_instructions: if args.is_empty() {
                    None
                } else {
                    Some(args.to_string())
                },
            }),
            "model" => {
                if args.is_empty() {
                    // Bare /model opens the picker: the request response fills
                    // the modal (parity with the JS TUI's selector).
                    Some(RpcCommand::GetAvailableModels { id: None })
                } else if let Some((provider, model_id)) = args.split_once('/') {
                    let (provider, model_id) = (provider.trim(), model_id.trim());
                    if provider.is_empty() || model_id.is_empty() {
                        self.entries.push(ChatEntry::Diag(
                            "model: use /model or /model <provider>/<model-id>".to_string(),
                        ));
                        None
                    } else {
                        Some(RpcCommand::SetModel {
                            id: None,
                            provider: provider.to_string(),
                            model_id: model_id.to_string(),
                        })
                    }
                } else {
                    self.entries.push(ChatEntry::Diag(
                        "model: use /model or /model <provider>/<model-id>".to_string(),
                    ));
                    None
                }
            }
            "thinking" => {
                if args.is_empty() {
                    Some(RpcCommand::CycleThinkingLevel { id: None })
                } else {
                    const LEVELS: [&str; 7] =
                        ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
                    if LEVELS.contains(&args) {
                        Some(RpcCommand::SetThinkingLevel {
                            id: None,
                            level: args.to_string(),
                        })
                    } else {
                        self.entries.push(ChatEntry::Diag(format!(
                            "thinking level: one of {}",
                            LEVELS.join(" | ")
                        )));
                        None
                    }
                }
            }
            "stats" => Some(RpcCommand::GetSessionStats { id: None }),
            _ => None,
        }
    }

    /// True when the input names a local command exactly.
    pub fn is_local_command(&self, msg: &str) -> bool {
        let word = msg.split_whitespace().next().unwrap_or("");
        let word = word.trim_start_matches('/');
        Self::LOCAL_COMMANDS.iter().any(|(n, _, _)| *n == word) || word == "config"
    }

    /// Longest common prefix completion for the current command word.
    pub fn complete_command(&mut self) {
        let cands = self.command_candidates();
        if cands.is_empty() {
            return;
        }
        let pick = cands
            .get(self.cand_sel.min(cands.len() - 1))
            .map(|c| c.name.clone());
        if let Some(name) = pick {
            // Complete to the selected candidate.
            self.input = format!("/{}", name);
            self.cursor = self.input.chars().count();
            return;
        }
        if cands.len() == 1 {
            let full = format!("/{} ", cands[0].name);
            self.input = full;
            self.cursor = self.input.chars().count();
            return;
        }
        let mut common = String::new();
        let first = cands[0].name.as_bytes();
        for (i, b) in first.iter().enumerate() {
            if cands.iter().all(|c| c.name.as_bytes().get(i) == Some(b)) {
                common.push(*b as char);
            } else {
                break;
            }
        }
        if common.len() > self.input.trim_start_matches('/').len() {
            self.input = format!("/{}", common);
            self.cursor = self.input.chars().count();
        }
    }

    /// Cycle the candidate selection (↑↓). Returns true when popup is open.
    pub fn cycle_candidate(&mut self, down: bool) -> bool {
        let n = self.command_candidates().len();
        if n == 0 {
            return false;
        }
        self.cand_sel = if down {
            (self.cand_sel + 1).min(n - 1)
        } else {
            self.cand_sel.saturating_sub(1)
        };
        true
    }

    /// True when the command popup is open (candidates available).
    pub fn cand_sel_logic_open(&self) -> bool {
        !self.command_candidates().is_empty()
    }
}

/// Strip ANSI escape sequences (CSI + OSC) from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(n) = chars.next() {
                match n {
                    '[' => {
                        for c in chars.by_ref() {
                            if c.is_ascii_alphabetic() {
                                break;
                            }
                        }
                    }
                    ']' | '_' | 'P' => {
                        let mut prev = n;
                        for c in chars.by_ref() {
                            if c == '\x07' || (prev == '\x1b' && c == '\\') {
                                break;
                            }
                            prev = c;
                        }
                    }
                    _ => {}
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// For shell tools: the model-supplied purpose + first command line.
fn bash_title_and_cmd<'a>(
    name: &str,
    args: Option<&'a serde_json::Value>,
) -> Option<(&'a str, String)> {
    if name != "bash" && name != "powershell" {
        return None;
    }
    let obj = args?.as_object()?;
    let title = obj.get("title")?.as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    let cmd = obj
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    Some((title, cmd))
}

/// Short args summary for tool entries — first meaningful value, plus
/// purpose-built summaries for the two extension tools whose args otherwise
/// render as a wall of JSON (owner request 2026-11-10).
fn summarize_args(name: &str, args: &serde_json::Value) -> String {
    let Some(obj) = args.as_object() else {
        return String::new();
    };

    if name == "bash" || name == "powershell" {
        let command = obj.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let first = command.lines().next().unwrap_or("").trim();
        let title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        return match title {
            Some(t) => trunc_str(&format!("{t} · {first}"), 72),
            None => trunc_str(first, 48),
        };
    }

    if name == "scratchpad_write" {
        if obj.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
            return "reset all sections".to_string();
        }
        let counts: Vec<String> = ["facts", "context", "leads", "dead"]
            .iter()
            .filter_map(|key| {
                obj.get(*key)
                    .and_then(|v| v.as_array())
                    .map(|a| format!("{key} {}", a.len()))
            })
            .collect();
        if !counts.is_empty() {
            return counts.join(" · ");
        }
        return "no sections".to_string();
    }

    if name == "update_tasks" {
        if let Some(tasks) = obj.get("tasks").and_then(|v| v.as_array()) {
            let done = tasks
                .iter()
                .filter(|t| t.get("status").and_then(|s| s.as_str()) == Some("completed"))
                .count();
            return format!("{done}/{} done", tasks.len());
        }
    }

    for key in [
        "file_path",
        "path",
        "command",
        "pattern",
        "query",
        "url",
        "name",
        "skill",
    ] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            return trunc_str(v, 48);
        }
    }
    String::new()
}

/// Purpose-built args rendering for tools whose JSON is noise in the chat
/// (scratchpad_write, update_tasks) and for shell calls, whose command should
/// read as `$ <cmd>` under the model's title. `None` = use the generic JSON
/// args section.
fn special_tool_args(
    name: &str,
    args: &serde_json::Value,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
) -> Option<Vec<Line<'static>>> {
    match name {
        "scratchpad_write" => Some(scratchpad_args_lines(args, full, width, surf)),
        "update_tasks" => Some(task_args_lines(args, full, width, surf)),
        "bash" | "powershell" => Some(shell_args_lines(args, full, width, surf)),
        _ => None,
    }
}

fn scratchpad_args_lines(
    args: &serde_json::Value,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme::BORDER_DIM).bg(surf);
    let muted = Style::default().fg(theme::MUTED).bg(surf);
    let text = Style::default().fg(theme::TEXT).bg(surf);
    let mut out = vec![Line::from(vec![
        Span::styled("│ ", border),
        Span::styled("notebook", muted),
    ])];
    let mut any = false;
    for section in ["facts", "context", "leads", "dead"] {
        let Some(items) = args.get(section).and_then(|v| v.as_array()) else {
            continue;
        };
        any = true;
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(
                format!("{section} ({})", items.len()),
                Style::default().fg(theme::GOLD).bg(surf),
            ),
        ]));
        let budget = if full { items.len() } else { 4 };
        for item in items.iter().take(budget) {
            let raw = item.as_str().unwrap_or("");
            for (i, seg) in wrap_text(raw, width.saturating_sub(8))
                .into_iter()
                .enumerate()
            {
                let prefix = if i == 0 { "│   · " } else { "│     " };
                out.push(Line::from(vec![
                    Span::styled(prefix, border),
                    Span::styled(seg, text),
                ]));
            }
        }
        if items.len() > budget {
            out.push(Line::from(vec![
                Span::styled("│   ", border),
                Span::styled(format!("… {} more", items.len() - budget), muted),
            ]));
        }
    }
    if args.get("reset").and_then(|v| v.as_bool()).unwrap_or(false) {
        any = true;
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(
                "reset all sections",
                Style::default().fg(theme::ORANGE).bg(surf),
            ),
        ]));
    }
    if !any {
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("(no sections named)", muted),
        ]));
    }
    out
}

fn task_args_lines(
    args: &serde_json::Value,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme::BORDER_DIM).bg(surf);
    let muted = Style::default().fg(theme::MUTED).bg(surf);
    let text = Style::default().fg(theme::TEXT).bg(surf);
    let mut out = vec![Line::from(vec![
        Span::styled("│ ", border),
        Span::styled("checklist", muted),
    ])];
    let Some(tasks) = args.get("tasks").and_then(|v| v.as_array()) else {
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("(no tasks)", muted),
        ]));
        return out;
    };
    if tasks.is_empty() {
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("(empty list)", muted),
        ]));
    }
    let budget = if full { tasks.len() } else { 8 };
    for task in tasks.iter().take(budget) {
        let status = task
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");
        let (glyph, color) = match status {
            "in_progress" => ("▶", theme::GOLD),
            "completed" => ("✓", theme::GREEN),
            "cancelled" => ("✗", theme::MUTED),
            _ => ("·", theme::MUTED),
        };
        let desc = task
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let id = task
            .get("id")
            .and_then(|v| v.as_i64())
            .map(|n| format!("{n}. "))
            .unwrap_or_default();
        let body = format!("{id}{desc}");
        for (i, seg) in wrap_text(&body, width.saturating_sub(8))
            .into_iter()
            .enumerate()
        {
            let mut spans = vec![Span::styled(if i == 0 { "│   " } else { "│     " }, border)];
            spans.push(Span::styled(
                if i == 0 {
                    format!("{glyph} ")
                } else {
                    "  ".to_string()
                },
                Style::default().fg(color).bg(surf),
            ));
            spans.push(Span::styled(seg, text));
            out.push(Line::from(spans));
        }
    }
    if tasks.len() > budget {
        out.push(Line::from(vec![
            Span::styled("│   ", border),
            Span::styled(format!("… {} more", tasks.len() - budget), muted),
        ]));
    }
    out
}

fn shell_args_lines(
    args: &serde_json::Value,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme::BORDER_DIM).bg(surf);
    let muted = Style::default().fg(theme::MUTED).bg(surf);
    let text = Style::default().fg(theme::TEXT).bg(surf);
    let mut out = vec![Line::from(vec![
        Span::styled("│ ", border),
        Span::styled("command", muted),
    ])];
    let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let total_lines = command.lines().count().max(1);
    let budget = if full { total_lines } else { 8 };
    for (i, logical) in command.lines().take(budget).enumerate() {
        for (j, seg) in wrap_text(logical, width.saturating_sub(8))
            .into_iter()
            .enumerate()
        {
            let prefix = if i == 0 && j == 0 {
                "│   $ "
            } else {
                "│     "
            };
            out.push(Line::from(vec![
                Span::styled(prefix, border),
                Span::styled(seg, text),
            ]));
        }
    }
    if total_lines > budget {
        out.push(Line::from(vec![
            Span::styled("│   ", border),
            Span::styled(format!("… {} more lines", total_lines - budget), muted),
        ]));
    }
    out
}

/// Render a Tool entry in Block or Full verbosity (owner request
/// 2026-09-03). Open-right box — no right border, so nothing needs
/// padding math and wrapped content reads naturally in the chat flow:
///
/// ```text
/// ╭─ ⌬ write ▸ src/main.rs
/// │ args  {"path": "src/main.rs"}
/// │ out
/// │   use tokio::sync::mpsc;
/// │   … 3 more lines
/// ╰─ ✓
/// ```
///
/// Block shows a one-line args summary and a preview slice of the output;
/// Full shows the complete pretty-printed args JSON and (near-)all output,
/// with a live tail while the tool is running. `width` is the box budget.
fn render_tool_block(
    mode: ToolVerbosity,
    name: &str,
    summary: &str,
    args: Option<&serde_json::Value>,
    output: &[String],
    truncated: bool,
    running: bool,
    error: bool,
    width: usize,
    elapsed: Option<u64>,
    timeout_s: Option<u64>,
    no_output_hint: bool,
) -> Vec<Line<'static>> {
    let full = mode == ToolVerbosity::Full;
    // The panel is an OPAQUE PLATE (owner request 2026-09-03: fire must
    // not interrupt it — "it stands apart from the background"). Every
    // span carries bg(PANEL) and every line pads to `width`, so flame
    // cells cannot show through gaps and the tint pass skips the surface.
    let surf = theme::PANEL;
    let border = Style::default().fg(theme::BORDER_DIM).bg(surf);
    let text_st = Style::default().fg(theme::TEXT).bg(surf);
    let muted_st = Style::default().fg(theme::MUTED).bg(surf);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(16 + output.len());

    // Header: ╭─ ⌬ name [▸ summary]
    {
        let mut spans = vec![
            Span::styled("╭─ ", border),
            Span::styled(
                format!("{} ", theme::GLYPH_CRUCIBLE),
                Style::default().fg(theme::GOLD).bg(surf),
            ),
            Span::styled(
                name.to_string(),
                Style::default()
                    .fg(theme::SILVER)
                    .bg(surf)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
        ];
        let head_w = width.saturating_sub(6 + name.chars().count());
        if let Some((title, cmd)) = bash_title_and_cmd(name, args) {
            if head_w > 4 {
                let budget = head_w.saturating_sub(3);
                let title_s = trunc_str(title, budget.min(40));
                let remain = budget.saturating_sub(title_s.chars().count() + 3);
                spans.push(Span::styled(" ▸ ", muted_st));
                spans.push(Span::styled(
                    title_s,
                    Style::default()
                        .fg(theme::GOLD)
                        .bg(surf)
                        .add_modifier(ratatui::style::Modifier::BOLD),
                ));
                if remain > 0 && !cmd.is_empty() {
                    spans.push(Span::styled(" · ", muted_st));
                    spans.push(Span::styled(trunc_str(&cmd, remain), muted_st));
                }
            }
        } else if !summary.is_empty() && head_w > 4 {
            spans.push(Span::styled(
                format!(" ▸ {}", trunc_str(summary, head_w - 3)),
                muted_st,
            ));
        }
        lines.push(Line::from(spans));
    }

    // Args section — purpose-built for the extension/shell tools, generic
    // JSON only for everything else.
    if let Some(a) = args.filter(|a| !a.is_null()) {
        if let Some(special) = special_tool_args(name, a, full, width, surf) {
            lines.extend(special);
        } else if full {
            lines.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("args", muted_st),
            ]));
            if let Ok(pretty) = serde_json::to_string_pretty(a) {
                for pl in pretty.lines() {
                    let mut spans = vec![Span::styled("│   ", border)];
                    spans.extend(json_line_spans(pl, width.saturating_sub(4)));
                    lines.push(Line::from(spans));
                }
            }
        } else {
            let one = serde_json::to_string(a).unwrap_or_default();
            let compact = if summary.is_empty() {
                one
            } else {
                format!("{} {}", summary, one)
            };
            lines.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("args  ", muted_st),
                Span::styled(trunc_str(compact.trim(), width.saturating_sub(9)), text_st),
            ]));
        }
    }

    // Output section.
    if !output.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("out", muted_st),
        ]));
        let budget = match (full, running) {
            (true, true) => 50, // live tail
            (true, false) => TOOL_OUTPUT_CAP,
            (false, true) => 8,
            (false, false) => TOOL_OUTPUT_PREVIEW,
        };
        // Completed views show the HEAD (the construction story); running
        // views show the TAIL (what's happening now).
        let slice: &[String] = if running && output.len() > budget {
            &output[output.len() - budget..]
        } else if output.len() > budget {
            &output[..budget]
        } else {
            output
        };
        for ol in slice {
            for (i, seg) in wrap_text(ol, width.saturating_sub(5))
                .into_iter()
                .enumerate()
            {
                let prefix = if i == 0 { "│   " } else { "│   " };
                lines.push(Line::from(vec![
                    Span::styled(prefix, border),
                    Span::styled(seg, text_st),
                ]));
            }
        }
        let hidden = output.len().saturating_sub(slice.len());
        if hidden > 0 {
            lines.push(Line::from(vec![
                Span::styled("│   ", border),
                Span::styled(format!("… {} more lines", hidden), muted_st),
            ]));
        }
        if truncated {
            lines.push(Line::from(vec![
                Span::styled("│   ", border),
                Span::styled("… output truncated (storage cap)", muted_st),
            ]));
        }
    } else if running {
        let mut spans = vec![
            Span::styled("│ ", border),
            Span::styled(
                "running…",
                Style::default().fg(theme::LIGHT_YELLOW).bg(surf),
            ),
        ];
        if let Some(secs) = elapsed {
            spans.push(Span::styled(
                format!(" {}", fmt_elapsed(secs)),
                Style::default().fg(theme::ORANGE).bg(surf),
            ));
        }
        lines.push(Line::from(spans));
        if no_output_hint {
            lines.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled(
                    trunc_str(
                        "no output yet — pipe-buffered? (e.g. `2>&1 | tail`)",
                        width.saturating_sub(4),
                    ),
                    muted_st.add_modifier(ratatui::style::Modifier::ITALIC),
                ),
            ]));
        }
    }

    // Bottom status.
    {
        let (icon, label, st) = if running {
            (
                theme::GLYPH_CRUCIBLE,
                " running",
                Style::default().fg(theme::ORANGE).bg(surf),
            )
        } else if error {
            ("✗", " failed", Style::default().fg(theme::RED).bg(surf))
        } else {
            ("✓", "", Style::default().fg(theme::GREEN).bg(surf))
        };
        let mut status = format!("{}{}", icon, label);
        if running {
            if let Some(secs) = elapsed {
                status.push_str(&format!(" · {}", fmt_elapsed(secs)));
            }
            if let Some(t) = timeout_s {
                status.push_str(&format!(" · timeout {}", fmt_elapsed(t)));
            }
        }
        lines.push(Line::from(vec![
            Span::styled("╰─ ", border),
            Span::styled(status, st),
        ]));
    }
    lines.into_iter().map(|l| pad_panel(l, width)).collect()
}

/// Pad a tool-block line to `width` with PANEL-surfaced spaces — the last
/// inch of every row is panel too, so nothing burns through the plate.
fn pad_panel(mut line: Line<'static>, width: usize) -> Line<'static> {
    let w: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    if w < width {
        line.spans.push(Span::styled(
            " ".repeat(width - w),
            Style::default().bg(theme::PANEL),
        ));
    }
    line
}

/// Compact wall-clock format: `45s`, `12m 03s`, `1h 02m`.
fn fmt_elapsed(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{}h {:02}m", h, m)
    } else if m > 0 {
        format!("{}m {:02}s", m, s)
    } else {
        format!("{}s", s)
    }
}

/// Pretty-JSON line colorized into spans: `"key":` cyan, braces muted,
/// scalar values text-colored. Truncated to `width` chars.
fn json_line_spans(line: &str, width: usize) -> Vec<Span<'static>> {
    let surf = theme::PANEL;
    let t = trunc_str(line, width);
    let trimmed = t.trim_start();
    let brace = trimmed == "{" || trimmed == "}" || trimmed == "},";
    if brace {
        return vec![Span::styled(t, Style::default().fg(theme::MUTED).bg(surf))];
    }
    if let Some(pos) = t.find("\":") {
        let split = pos + 2; // include the colon
        return vec![
            Span::styled(
                t[..split].to_string(),
                Style::default().fg(theme::CYAN).bg(surf),
            ),
            Span::styled(
                t[split..].to_string(),
                Style::default().fg(theme::TEXT).bg(surf),
            ),
        ];
    }
    vec![Span::styled(t, Style::default().fg(theme::TEXT).bg(surf))]
}

/// Char-count truncation with ellipsis.
fn trunc_str(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let cut: String = s.chars().take(w.saturating_sub(1)).collect();
        format!("{}…", cut)
    }
}

/// Greedy word wrap returning owned strings.
fn wrap_text(s: &str, width: usize) -> Vec<String> {
    let width = width.max(10);
    if s.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for para in s.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut cur = String::new();
        for word in para.split_whitespace() {
            let candidate_len = cur.chars().count() + 1 + word.chars().count();
            if !cur.is_empty() && candidate_len > width {
                lines.push(std::mem::take(&mut cur));
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            if word.chars().count() > width {
                let mut chunk = String::new();
                for ch in word.chars() {
                    if chunk.chars().count() >= width {
                        lines.push(std::mem::take(&mut chunk));
                    }
                    chunk.push(ch);
                }
                cur = chunk;
            } else {
                cur.push_str(word);
            }
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_agent_mode_captures_current_and_next() {
        let mut s = AppState::default();
        s.parse_agent_mode(&[
            "\x1b[1m\x1b[38;2;46;161m▪ BUILD\x1b[0m · writes unlocked · TAB / /agent plan\x1b[0m"
                .to_string(),
        ]);
        assert_eq!(s.agent_mode_label.as_deref(), Some("BUILD"));
        assert_eq!(s.agent_mode_glyph.as_deref(), Some("▪"));
        assert_eq!(s.agent_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn parse_agent_mode_handles_loud_directive_format() {
        let mut s = AppState::default();
        s.parse_agent_mode(&["► PLAN AGENT — research only · TAB / /agent build".to_string()]);
        assert_eq!(s.agent_mode_label.as_deref(), Some("PLAN"));
        assert_eq!(s.agent_mode_glyph.as_deref(), Some("►"));
        assert_eq!(s.agent_mode.as_deref(), Some("build"));
    }

    #[test]
    fn parse_agent_mode_accepts_legacy_mode_widget() {
        let mut s = AppState::default();
        s.parse_agent_mode(&["▪ BUILD · writes unlocked · TAB / /mode plan".to_string()]);
        assert_eq!(s.agent_mode_label.as_deref(), Some("BUILD"));
        assert_eq!(s.agent_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn glimmer_cycles_and_sets() {
        let mut s = AppState::default();
        assert_eq!(s.set_glimmer("off"), "off");
        assert_eq!(s.set_glimmer(""), "shimmer"); // cycles off → shimmer
        assert_eq!(s.set_glimmer("breathe"), "breathe");
        assert_eq!(s.set_glimmer("nonsense"), "breathe"); // unknown ignored
    }

    #[test]
    fn fire_widget_parses_and_low_passes() {
        let mut s = AppState::default();
        s.set_widget("engine-fire", vec!["FIRE 0.800".to_string()]);
        assert!((s.fire_target - 0.8).abs() < 1e-9);
        s.step_fire(1.0); // one full time-constant
        assert!(s.fire_level > 0.5 && s.fire_level < 0.8);
        s.set_widget("engine-fire", vec!["\x1b[31mFIRE 1.500\x1b[0m".to_string()]);
        assert!((s.fire_target - 1.0).abs() < 1e-9); // clamped
    }

    #[test]
    fn fire_toggle_and_kill() {
        let mut s = AppState::default();
        assert_eq!(s.set_fire(""), "off"); // default on → toggle off
        assert_eq!(s.set_fire("on"), "prismatic"); // default voice
        assert_eq!(s.set_fire("nonsense"), "prismatic"); // unknown ignored
        s.fire_target = 0.9;
        assert_eq!(s.set_fire("off"), "off");
        assert_eq!(s.fire_target, 0.0); // kill kills the target too
        assert_eq!(s.fire_level, 0.0); // below MIN after step target zero
        s.step_fire(1.0);
        assert_eq!(s.fire_level, 0.0);
    }

    #[test]
    fn fire_style_switch_and_cycle() {
        let mut s = AppState::default();
        assert_eq!(s.fire_style, crate::fire::FireStyle::Pulse); // prismatic default
        assert_eq!(s.set_fire("pulse"), "prismatic"); // legacy alias
        assert_eq!(s.set_fire("alchemy"), "alchemy");
        assert!(s.fire_on); // naming a style ignites
                            // Cycle order: prismatic → emerald → alchemy → prismatic.
        assert_eq!(s.set_fire("style"), "prismatic");
        assert_eq!(s.set_fire("style"), "emerald");
        assert_eq!(s.set_fire("emerald"), "emerald");
        assert_eq!(s.set_fire("style"), "alchemy");
    }

    #[test]
    fn fire_tint_toggles_independently() {
        let mut s = AppState::default();
        assert!(s.fire_tint, "tint on by default — the beloved behavior");
        assert_eq!(s.set_fire("tint"), "tint off");
        assert!(!s.fire_tint);
        assert!(s.fire_on, "tint toggle must not touch the flame itself");
        assert_eq!(s.set_fire("tint"), "tint on");
        assert!(s.fire_tint);
        // The flame can die while tint stays configured — config, not state.
        s.set_fire("off");
        assert_eq!(s.set_fire("tint"), "tint off");
    }

    // ── Tool verbosity (owner request 2026-09-03) ────────────────────────

    #[test]
    fn tool_verbosity_order_and_cycle() {
        assert!(ToolVerbosity::Line < ToolVerbosity::Block);
        assert!(ToolVerbosity::Block < ToolVerbosity::Full);
        assert_eq!(ToolVerbosity::Line.next(), ToolVerbosity::Block);
        assert_eq!(ToolVerbosity::Full.next(), ToolVerbosity::Line);
        assert_eq!(ToolVerbosity::parse("block"), Some(ToolVerbosity::Block));
        assert_eq!(ToolVerbosity::parse("nope"), None);
        assert_eq!(
            ToolVerbosity::Line.max(ToolVerbosity::Full),
            ToolVerbosity::Full
        );
        assert_eq!(
            ToolVerbosity::Full.min(ToolVerbosity::Line),
            ToolVerbosity::Line
        );
    }

    #[test]
    fn tool_override_parse_encode_roundtrip() {
        for (s, mode, strict) in [
            ("block!", ToolVerbosity::Block, Strictness::Pinned),
            ("full+", ToolVerbosity::Full, Strictness::AtLeast),
            ("line-", ToolVerbosity::Line, Strictness::AtMost),
            ("block", ToolVerbosity::Block, Strictness::AtLeast), // bare = at-least
        ] {
            let o = ToolOverride::parse(s).unwrap();
            assert_eq!(o.mode, mode);
            assert_eq!(o.strictness, strict);
        }
        assert!(ToolOverride::parse("wat").is_none());
        // Round-trip.
        for s in ["block!", "full+", "line-"] {
            let o = ToolOverride::parse(s).unwrap();
            assert_eq!(o.encode(), s);
        }
    }

    #[test]
    fn effective_mode_resolution_matrix() {
        let mut ov: HashMap<String, ToolOverride> = HashMap::new();
        ov.insert(
            "write".into(),
            ToolOverride {
                mode: ToolVerbosity::Block,
                strictness: Strictness::AtLeast,
            },
        );
        ov.insert(
            "bash".into(),
            ToolOverride {
                mode: ToolVerbosity::Block,
                strictness: Strictness::AtMost,
            },
        );
        ov.insert(
            "read".into(),
            ToolOverride {
                mode: ToolVerbosity::Full,
                strictness: Strictness::Pinned,
            },
        );
        let g = ToolVerbosity::Line;
        // AtLeast bumps up when global less verbose.
        assert_eq!(effective_mode("write", g, &ov), ToolVerbosity::Block);
        // AtMost caps when global more verbose.
        assert_eq!(
            effective_mode("bash", ToolVerbosity::Full, &ov),
            ToolVerbosity::Block
        );
        // Pinned ignores global both ways.
        assert_eq!(effective_mode("read", g, &ov), ToolVerbosity::Full);
        assert_eq!(
            effective_mode("read", ToolVerbosity::Full, &ov),
            ToolVerbosity::Full
        );
        // No override → global.
        assert_eq!(effective_mode("grep", g, &ov), ToolVerbosity::Line);
    }

    #[test]
    fn oldest_running_tool_and_abandon() {
        let mut s = AppState::default();
        s.tool_start(
            Some("call-1"),
            "bash",
            &serde_json::json!({"command": "sleep 999"}),
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        s.tool_start(
            Some("call-2"),
            "bash",
            &serde_json::json!({"command": "ls"}),
        );
        let (key, name, _, _) = s.oldest_running_tool().expect("running tool");
        assert_eq!(key, "call-1");
        assert_eq!(name, "bash");

        s.tool_end(Some("call-1"), "bash", "done", false);
        let (key, _, _, _) = s.oldest_running_tool().expect("still running");
        assert_eq!(key, "call-2");

        assert_eq!(s.abandon_running_tools("no result"), 1);
        assert!(s.oldest_running_tool().is_none());
        let failed = s.entries.iter().any(|e| {
            matches!(
                e,
                ChatEntry::Tool {
                    running: false,
                    error: true,
                    ..
                }
            )
        });
        assert!(failed);
    }

    #[test]
    fn tool_start_update_end_by_id() {
        let mut s = AppState::default();
        s.tool_start(
            Some("call-1"),
            "write",
            &serde_json::json!({"path": "a.rs"}),
        );
        s.tool_start(
            Some("call-2"),
            "write",
            &serde_json::json!({"path": "b.rs"}),
        );
        // Two same-named tools running — id must disambiguate.
        s.tool_update(Some("call-1"), "write", "line one\nline two\n");
        s.tool_update(Some("call-2"), "write", "other\n");
        s.tool_end(Some("call-2"), "write", "final\n", false);
        // call-2 complete with output; call-1 still running with its own.
        let states: Vec<(bool, Vec<String>)> = s
            .entries
            .iter()
            .filter_map(|e| match e {
                ChatEntry::Tool {
                    output, running, ..
                } => Some((*running, output.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(states.len(), 2);
        assert!(states[0].0, "call-1 still running");
        assert_eq!(states[0].1, vec!["line one", "line two"]);
        assert!(!states[1].0, "call-2 done");
        assert_eq!(states[1].1, vec!["other", "final"]);
    }

    #[test]
    fn tool_end_falls_back_to_name_match() {
        let mut s = AppState::default();
        // No id available (legacy events) → name fallback.
        s.tool_start(None, "bash", &serde_json::json!({"command": "ls"}));
        s.tool_end(None, "bash", "", true);
        let done = s
            .entries
            .iter()
            .filter_map(|e| match e {
                ChatEntry::Tool { running, error, .. } => Some((*running, *error)),
                _ => None,
            })
            .next();
        assert_eq!(done, Some((false, true)));
    }

    #[test]
    fn tool_output_cap_marks_truncated() {
        let mut s = AppState::default();
        s.tool_start(Some("c"), "bash", &serde_json::json!({}));
        let flood = (0..(TOOL_OUTPUT_CAP + 50))
            .map(|i| format!("l{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        s.tool_update(Some("c"), "bash", &flood);
        if let Some(ChatEntry::Tool {
            output,
            output_truncated,
            ..
        }) = s.entries.last()
        {
            assert_eq!(output.len(), TOOL_OUTPUT_CAP);
            assert!(output_truncated);
        } else {
            panic!("expected tool entry");
        }
    }

    #[test]
    fn line_mode_renders_like_legacy() {
        let mut s = AppState::default();
        assert_eq!(s.tool_default, ToolVerbosity::Line);
        s.tool_start(
            Some("c"),
            "write",
            &serde_json::json!({"path": "src/main.rs"}),
        );
        s.tool_end(Some("c"), "write", "", false);
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("✓"), "done icon present: {}", text);
        assert!(text.contains("write"), "name present: {}", text);
        assert!(text.contains("src/main.rs"), "summary present: {}", text);
        // No block furniture in Line mode.
        assert!(!text.contains('╭'), "no block border: {}", text);
    }

    #[test]
    fn slash_completion_uses_hovered_candidate() {
        let mut s = AppState::default();
        s.commands = vec![crate::rpc::protocol::SlashCommand {
            name: "helper".to_string(),
            description: None,
            source: None,
        }];
        s.input = "/hel".to_string();
        // Candidates: /help (local) and /helper (pi), sorted.
        s.cand_sel = 1;
        s.complete_command();
        assert_eq!(s.input, "/helper");
    }

    #[test]
    fn sidebar_visibility_cycles_auto_show_hide() {
        let mut s = AppState::default();
        assert_eq!(s.sidebar_force, None);
        s.cycle_sidebar_mode();
        assert_eq!(s.sidebar_force, Some(true));
        s.cycle_sidebar_mode();
        assert_eq!(s.sidebar_force, Some(false));
        s.cycle_sidebar_mode();
        assert_eq!(s.sidebar_force, None);
    }

    #[test]
    fn sidebar_scroll_clamps_to_content() {
        let mut s = AppState::default();
        s.sidebar_scroll_max = 10;
        s.sidebar_scroll_up(100);
        assert_eq!(s.sidebar_scroll, 10);
        s.sidebar_scroll_down(4);
        assert_eq!(s.sidebar_scroll, 6);
        s.sidebar_scroll_down(100);
        assert_eq!(s.sidebar_scroll, 0);
    }

    #[test]
    fn chat_entries_have_exactly_one_blank_separator() {
        let mut s = AppState::default();
        s.push_user("one".into());
        s.entries.push(ChatEntry::Assistant("two".into()));
        s.tool_start(Some("c"), "bash", &serde_json::json!({"command": "true"}));
        s.tool_end(Some("c"), "bash", "", false);
        let lines = s.chat_lines(80);
        let blank = |l: &ratatui::text::Line| l.spans.iter().all(|sp| sp.content.trim().is_empty());
        let blanks = lines.iter().filter(|l| blank(l)).count();
        assert!(blanks >= 2, "one gap between each of 3 entries");
        assert!(
            !lines.windows(2).any(|w| blank(&w[0]) && blank(&w[1])),
            "no double blank lines"
        );
    }

    #[test]
    fn at_least_override_promotes_to_block() {
        let mut s = AppState::default();
        s.tool_overrides.insert(
            "write".into(),
            ToolOverride {
                mode: ToolVerbosity::Block,
                strictness: Strictness::AtLeast,
            },
        );
        s.tool_start(
            Some("c"),
            "write",
            &serde_json::json!({"path": "src/main.rs"}),
        );
        s.tool_end(Some("c"), "write", "out line\n", false);
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('╭'), "block header present: {}", text);
        assert!(text.contains('╰'), "block footer present: {}", text);
        assert!(text.contains("args"), "args section present: {}", text);
        assert!(text.contains("out line"), "output present: {}", text);
    }

    #[test]
    fn full_mode_shows_pretty_args() {
        let mut s = AppState::default();
        s.tool_default = ToolVerbosity::Full;
        s.tool_start(
            Some("c"),
            "edit",
            &serde_json::json!({"path": "a.rs", "old": "x", "new": "y"}),
        );
        s.tool_end(Some("c"), "edit", "", false);
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("args"), "args header: {}", text);
        assert!(text.contains("\"path\""), "pretty json key: {}", text);
        assert!(text.contains("\"old\""), "second key: {}", text);
    }

    #[test]
    fn cache_invalidates_on_gen_and_stream() {
        let mut s = AppState::default();
        s.tool_default = ToolVerbosity::Block;
        s.tool_start(Some("c"), "bash", &serde_json::json!({}));
        s.tool_update(Some("c"), "bash", "one\n");
        let g0 = s.tools_gen;
        let _ = s.chat_lines(60);
        // Same inputs → cache hit (gen/output unchanged): mutate width only
        // to force rebuild and confirm width is part of the key.
        let _ = s.chat_lines(60);
        assert_eq!(s.tools_gen, g0);
        // Streaming grows output → cache must rebuild (output_len mismatch).
        s.tool_update(Some("c"), "bash", "two\n");
        let _ = s.chat_lines(60);
        // Config change → cache must rebuild for every entry.
        s.tools_gen += 1;
        let text = s
            .chat_lines(60)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("one"), "streamed lines visible: {}", text);
        assert!(text.contains("two"), "streamed lines visible: {}", text);
    }

    #[test]
    fn scratchpad_summary_is_counts_not_json() {
        let s = summarize_args(
            "scratchpad_write",
            &serde_json::json!({"facts": ["a", "b"], "leads": ["x"]}),
        );
        assert_eq!(s, "facts 2 · leads 1");
    }

    #[test]
    fn task_summary_is_done_count() {
        let s = summarize_args(
            "update_tasks",
            &serde_json::json!({"tasks": [
                {"id": 1, "description": "a", "status": "completed"},
                {"id": 2, "description": "b", "status": "in_progress"}
            ]}),
        );
        assert_eq!(s, "1/2 done");
    }

    #[test]
    fn bash_line_title_and_command_use_different_colors() {
        let mut s = AppState::default();
        s.tool_start(
            Some("c"),
            "bash",
            &serde_json::json!({"command": "cargo test --lib", "title": "Run tests"}),
        );
        s.tool_end(Some("c"), "bash", "", false);
        let line = s
            .chat_lines(100)
            .into_iter()
            .find(|l| l.to_string().contains("Run tests"))
            .expect("bash line");
        assert!(
            line.spans
                .iter()
                .any(|sp| sp.content.contains("Run tests")
                    && sp.style.fg == Some(crate::theme::GOLD)),
            "title gold: {:?}",
            line.spans
        );
        assert!(
            line.spans
                .iter()
                .any(|sp| sp.content.contains("cargo test")
                    && sp.style.fg != Some(crate::theme::GOLD)),
            "command not gold: {:?}",
            line.spans
        );
    }

    #[test]
    fn bash_title_and_cmd_extracts_purpose() {
        let args = serde_json::json!({"command": "cargo test --lib\nmore", "title": "Run tests"});
        let (title, cmd) = bash_title_and_cmd("bash", Some(&args)).unwrap();
        assert_eq!(title, "Run tests");
        assert_eq!(cmd, "cargo test --lib");
        assert!(bash_title_and_cmd("read", Some(&args)).is_none());
        assert!(bash_title_and_cmd("bash", Some(&serde_json::json!({"command": "ls"}))).is_none());
    }

    #[test]
    fn bash_summary_prepends_title_to_first_command_line() {
        let s = summarize_args(
            "bash",
            &serde_json::json!({"command": "cargo test --lib\ncargo build", "title": "Run tests"}),
        );
        assert_eq!(s, "Run tests · cargo test --lib");
    }

    #[test]
    fn scratchpad_block_renders_sections_not_raw_json() {
        let mut s = AppState::default();
        s.tool_default = ToolVerbosity::Block;
        s.tool_start(
            Some("c"),
            "scratchpad_write",
            &serde_json::json!({"facts": ["evidence one", "evidence two"]}),
        );
        s.tool_end(Some("c"), "scratchpad_write", "", false);
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("facts (2)"), "section header: {}", text);
        assert!(text.contains("evidence one"), "fact line: {}", text);
        assert!(!text.contains("\"facts\""), "no raw json: {}", text);
    }

    #[test]
    fn task_block_renders_checklist_not_raw_json() {
        let mut s = AppState::default();
        s.tool_default = ToolVerbosity::Block;
        s.tool_start(
            Some("c"),
            "update_tasks",
            &serde_json::json!({"tasks": [
                {"id": 1, "description": "first item", "status": "completed"},
                {"id": 2, "description": "second item", "status": "in_progress"}
            ]}),
        );
        s.tool_end(Some("c"), "update_tasks", "", false);
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("checklist"), "checklist header: {}", text);
        assert!(text.contains("✓ 1. first item"), "done glyph: {}", text);
        assert!(text.contains("▶ 2. second item"), "active glyph: {}", text);
        assert!(!text.contains("\"tasks\""), "no raw json: {}", text);
    }
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    /// Owner request 2026-09-03: the tool panel is an opaque plate — fire
    /// renders AROUND it, never through it. Every line fills its width and
    /// every span carries the PANEL surface.
    #[test]
    fn tool_block_is_an_opaque_panel() {
        let lines = render_tool_block(
            ToolVerbosity::Block,
            "edit",
            "/home/x/docs/GUARDS.md",
            Some(&serde_json::json!({ "path": "/home/x/docs/GUARDS.md", "edits": [1] })),
            &["Successfully replaced 1 block(s).".to_string()],
            false,
            false,
            false,
            40,
            Some(7),
            Some(30),
            false,
        );
        assert!(
            lines.len() >= 4,
            "header + args + out + status: {}",
            lines.len()
        );
        for l in &lines {
            let w: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(w, 40, "panel line fills its width: {:?}", l.to_string());
            for s in &l.spans {
                assert_eq!(s.style.bg, Some(theme::PANEL), "span bg: {:?}", s.content);
            }
        }
    }

    #[test]
    fn running_tool_block_is_also_opaque() {
        let lines = render_tool_block(
            ToolVerbosity::Full,
            "bash",
            "",
            Some(&serde_json::json!({ "command": "cargo test" })),
            &[],
            false,
            true,
            false,
            40,
            Some(90),
            Some(1800),
            true,
        );
        for l in &lines {
            let w: usize = l.spans.iter().map(|s| s.content.chars().count()).sum();
            assert_eq!(w, 40);
            assert!(l.spans.iter().all(|s| s.style.bg == Some(theme::PANEL)));
        }
    }

    /// Owner request 2026-09-07: a long bash with pipe-buffered output reads
    /// as a hang — the running block must show elapsed time, the declared
    /// timeout, and a pipe-buffer hint once output stays empty.
    #[test]
    fn running_tool_shows_elapsed_timeout_and_buffer_hint() {
        let lines = render_tool_block(
            ToolVerbosity::Full,
            "bash",
            "",
            Some(&serde_json::json!({ "command": "cargo test", "timeout": 1800 })),
            &[],
            false,
            true,
            false,
            60,
            Some(93),
            Some(1800),
            true,
        );
        let text = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("1m 33s"), "elapsed shown: {}", text);
        assert!(text.contains("timeout 30m"), "timeout shown: {}", text);
        assert!(
            text.contains("pipe-buffered"),
            "buffer hint shown: {}",
            text
        );
    }

    #[test]
    fn fmt_elapsed_compact() {
        assert_eq!(fmt_elapsed(0), "0s");
        assert_eq!(fmt_elapsed(45), "45s");
        assert_eq!(fmt_elapsed(723), "12m 03s");
        assert_eq!(fmt_elapsed(3725), "1h 02m");
    }
}
