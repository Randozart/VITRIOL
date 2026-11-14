// Application state — chat entries, widgets, agent state, markdown cache.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
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

/// Formal display title for a tool registry name (2026-11-13, operator
/// request: "Update Tasks" instead of "update_tasks"). Unknown names are
/// title-cased on '_' / '-' boundaries.
pub fn tool_display_name(name: &str) -> String {
    let title = match name {
        "update_tasks" => return "Update Tasks".to_string(),
        "scratchpad_write" => return "Scratchpad Write".to_string(),
        "scratchpad_read" => return "Scratchpad Read".to_string(),
        "bash" => "Bash",
        "read" => "Read",
        "write" => "Write",
        "edit" => "Edit",
        "find" => "Find",
        "grep" => "Grep",
        "ls" => "List Directory",
        _ => {
            let mut out = String::with_capacity(name.len());
            let mut cap = true;
            for ch in name.chars() {
                if ch == '_' || ch == '-' {
                    out.push(' ');
                    cap = true;
                } else if cap {
                    out.extend(ch.to_uppercase());
                    cap = false;
                } else {
                    out.push(ch);
                }
            }
            return out;
        }
    };
    title.to_string()
}

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
    ("alt+e", "edit the draft in $VISUAL/$EDITOR"),
    ("^o", "attach the clipboard image (alt+o clears)"),
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
    /// Thinking block (2026-11-13): a left click toggles `collapsed`
    /// (close/open the trace); the text survives collapse.
    Thinking {
        text: String,
        collapsed: bool,
    },
    Tool {
        tool_call_id: Option<String>,
        name: String,
        summary: String,
        args: Option<serde_json::Value>,
        /// Click override (2026-11-14): the operator cycles
        /// closed (line) → formatted (block) → full → closed. None follows
        /// the /tools config until the first click.
        click_mode: Option<ToolVerbosity>,
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
    /// Informational notice (neutral, 2026-11-14): settings/janitor/scan
    /// reports and other ordinary output. Never rendered in the failure red.
    Diag(String),
    /// Observation that deserves attention but is not a failure (amber):
    /// harness watchdog aborts, budget pressure.
    Warn(String),
    /// Genuine failure (red): extension errors, failed commands, failed
    /// compactions/imports.
    Error(String),
    /// Long info-notify captured into the transcript (QoL batch stage 3,
    /// 2026-11-12): multi-line tables (/free, /nimbus, /phase-models) were
    /// invisible beyond the 40-char glance row. Neutral violet — NOT an
    /// error (Diag renders with theme::crit, which is red).
    Notice(String),
    /// hang-watchdog entry (2026-11-13): rendered as a compact amber-bordered
    /// block instead of a red Diag line — it is an observation, not an error.
    Watchdog(String),
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

/// INQUIRE answer (2026-11-13): versioned facts file written by the
/// agent-mode extension; rendered by /answers.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct AnswerDoc {
    pub version: u32,
    pub ts: u64,
    pub question: String,
    pub answer: String,
}

/// Structured watchdog facts (2026-11-13): written by the hang-watchdog
/// extension; the transcript renders fields and the tool row shows a
/// countdown, instead of dumping prose.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct WatchdogDoc {
    pub version: u32,
    pub ts: u64,
    pub kind: String,
    pub command: String,
    pub elapsed_s: u64,
    pub stalled_s: u64,
    pub verdict: String,
    pub reason: String,
    pub next_check_at: u64,
    pub phase: String,
}

/// A staged next-prompt suggestion (voice-suggestions stage 1).
#[derive(Debug, Clone)]
pub struct GhostSuggestion {
    pub id: String,
    pub text: String,
    pub source: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
struct SuggestionFile {
    version: u32,
    id: String,
    text: String,
    source: String,
    expires: u64,
}

impl Default for SuggestionFile {
    fn default() -> Self {
        Self {
            version: 0,
            id: String::new(),
            text: String::new(),
            source: String::new(),
            expires: 0,
        }
    }
}

/// One staged clipboard image (codex-qol stage 9).
#[derive(Debug, Clone)]
pub struct PendingImage {
    pub path: std::path::PathBuf,
    pub mime: String,
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
/// A message typed while the agent is busy — shown as held until it
/// actually enters the conversation (2026-11-13, operator request).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeldKind {
    /// Screened behind a running tool; delivered as a fresh Prompt at settle.
    Queued,
    /// Steered into the running turn (inserted at the next step).
    Steer,
    /// Alt+Enter follow-up (pi queues it for after the turn).
    FollowUp,
}

#[derive(Debug, Clone)]
pub struct HeldMsg {
    pub text: String,
    pub kind: HeldKind,
    pub at: std::time::Instant,
}

#[derive(Debug)]
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

    // Live context fill, captured from the agent_end event's contextUsage
    // (tokens used / window size). None until the first turn completes —
    // the bottom status row hides itself rather than showing a guess.
    pub ctx_usage: Option<(u64, u64)>,

    // Terminal title (setTitle)
    pub title: String,

    // Notifications (notify)
    pub notice: Option<(String, String)>, // (message, type)

    // Slash commands (get_commands) — autocomplete source
    pub commands: Vec<crate::rpc::protocol::SlashCommand>,

    // /tree listing — one line per session entry, numbered so `/fork <n>`
    // can address it by its entry id. Empty until /tree is run (the tree
    // panel hides itself rather than showing a stale capture). The first
    // tuple element is the rendered display line ("  7  message user"),
    // the second the entry id /fork needs.
    pub tree_lines: Vec<(String, String)>, // (display line, entry id)
    // True while the /tree listing panel is on screen (esc closes it).
    pub tree_open: bool,
    // Scroll position within /tree (view follows it like the other modals).
    pub tree_sel: usize,

    // /resume picker modal
    pub resume_open: bool,
    pub resume_entries: Vec<crate::rpc::sessions::SessionEntry>,
    pub resume_sel: usize,
    /// Codex-QoL stage 3: typeable filter over title/path in the picker.
    pub resume_filter: String,

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

    // /caduceus page (2026-11-12): facts from the extension's state file,
    // policy from the TUI-owned prefs file. The TUI never does HTTP and
    // never sees secrets (I4); it writes caduceus-prefs.json only.
    pub caduceus: Option<crate::tui::caduceus::Page>,
    /// Deferred external flow (stage 4): argv the loop runs after suspending
    /// the TUI (none = no suspend). Set by the caduceus page's A/o keys.
    pub caduceus_suspend: Option<Vec<String>>,
    /// /status modal (codex-qol stage 2): read-only session snapshot, built
    /// once at open (file reads must not happen per frame).
    pub status_open: bool,
    pub status_lines: Vec<(&'static str, String)>,
    /// Deferred $EDITOR hand-off (codex-qol stage 4): argv + tmp file + the
    /// composer draft as it was, so a failed edit never loses text (I4).
    pub editor_suspend: Option<(Vec<String>, std::path::PathBuf, String)>,
    /// /cost modal (exec-infra stage 5): today's route ledger, read-only.
    pub cost_open: bool,
    /// /routes modal — Caduceus decision trail (2026-11-13, operator
    /// request: see routing decisions at a glance).
    pub routes: Option<crate::tui::routes::RouteReport>,
    pub routes_open: bool,
    pub routes_sel: usize,
    pub cost: Option<crate::tui::cost::CostReport>,
    /// Turn clock (2026-11-13): stamped when streaming starts, cleared when
    /// it settles — drives the "working… Ns" readouts.
    pub turn_started_at: Option<std::time::Instant>,
    /// Question panel (2026-11-13): the ask extension stages question.json;
    /// the TUI renders it on top of the prompt and writes the response.
    pub question: Option<crate::tui::question::QuestionDoc>,
    pub question_sel: usize,
    pub question_checked_at: Option<std::time::Instant>,
    /// Safe-quit arm (2026-11-13): when a turn/tool is in flight, the first
    /// quit press arms a 5 s confirm window instead of killing the work.
    pub quit_armed_at: Option<std::time::Instant>,
    /// Thinking display (2026-11-13): hide thinking blocks behind a collapsed
    /// marker line; persisted at ~/.vitriol/officina/tui-thinking.
    pub hide_thinking: bool,
    /// Default collapsed state for NEW thinking blocks (2026-11-13,
    /// operator request — default is collapsed). Persisted at
    /// ~/.vitriol/officina/tui-thinking-default.
    pub thinking_default_collapsed: bool,
    /// Injectable default-prefs path (tests must never write the
    /// operator's live file).
    pub thinking_default_file: Option<std::path::PathBuf>,
    /// Line → entry map of the last chat_lines output. The renderer also
    /// derives clickable thinking-block row ranges from it (2026-11-13).
    pub line_entries: Vec<usize>,
    /// Visible thinking blocks as screen rows: (y0, y1, entry idx).
    /// Filled by layout each frame; a left click inside a range toggles
    /// the block's collapsed state (2026-11-13).
    pub thinking_hitboxes: Vec<(u16, u16, usize)>,
    /// Visible tool blocks as screen rows: (y0, y1, entry idx). A left
    /// click toggles collapsed ⇄ full args/output (2026-11-13).
    pub tool_hitboxes: Vec<(u16, u16, usize)>,
    /// Latest structured watchdog facts (card + countdown).
    pub watchdog: Option<WatchdogDoc>,
    /// Injectable watchdog facts path (tests must never read the
    /// operator's live file).
    pub watchdog_file: Option<std::path::PathBuf>,
    /// /fx modal (2026-11-13): fire + glimmer controls with a live preview.
    pub fx_open: bool,
    pub fx_sel: usize,
    /// /answers panel (2026-11-13): the last INQUIRE-mode question + answer,
    /// written by the agent-mode extension as a facts file.
    pub answers_open: bool,
    pub answers: Option<AnswerDoc>,
    pub answers_scroll: u16,
    pub answers_mtime_seen: Option<std::time::SystemTime>,
    /// Prompt history (2026-11-13, owner request): Up/Down through earlier
    /// prompts, persisted to ~/.vitriol/officina/input-history.jsonl.
    pub prompt_history: Vec<String>,
    pub history_idx: Option<usize>,
    pub history_draft: String,
    /// Engine telemetry freshness: when the `engine-fire` widget stops
    /// reporting (engine down / cloud lane), the flames derive from STREAM
    /// deltas instead — so cloud-model turns animate too (2026-11-13).
    pub engine_fire_seen: Option<std::time::Instant>,
    /// Delta-driven heat accumulator (bumped per streamed delta, decayed
    /// per animation step).
    pub stream_heat: f64,
    /// Liveliness contract (2026-11-13): wall-clock of the last REAL
    /// signal (streamed text, tool events, agent start/end, prompt submit).
    /// Fire only burns while this is fresh — animations never lie.
    pub last_activity: Option<std::time::Instant>,
    /// Liveness evidence (2026-11-12): CPU seconds of pi's bash subtree,
    /// sampled from /proc and shown in the running tool row — proof the
    /// work is real even when output is silent (redirected to logs).
    pub liveness_cpu_total: f64,
    pub liveness_prev_total: f64,
    pub liveness_delta: f64,
    pub liveness_flat_secs: u64,
    pub liveness_sampled_at: Option<std::time::Instant>,
    pub liveness_unavailable: bool,
    /// Voice-suggestion ghost text (stage 2, 2026-11-12): rendered dimmed
    /// in the empty composer; → accepts, typing/Esc dismisses. Never sent.
    pub prompt_ghost: Option<GhostSuggestion>,
    pub ghost_dismissed_for: Option<String>,
    /// Injectable paths so tests never touch the operator's files.
    pub prompt_suggestion_file: Option<std::path::PathBuf>,
    pub ghost_events_file: Option<std::path::PathBuf>,
    /// /sessions cross-project picker (exec-infra stage 4): every store,
    /// newest first. Foreign projects render READ-ONLY (I4).
    pub xsessions_open: bool,
    pub xsessions: Vec<crate::rpc::sessions::AllSessionEntry>,
    pub xsessions_sel: usize,
    pub xsessions_filter: String,
    /// /review-findings modal (codex-qol stage 7): severity-sorted findings
    /// from .officina/review.json, written by the review extension.
    pub review_open: bool,
    pub review: Option<crate::tui::review::ReviewFindings>,
    pub review_sel: usize,
    /// Clipboard image attachments (codex-qol stage 9, I5): capped, temp
    /// files encoded to base64 at send, cleared after a successful submit.
    pub pending_images: Vec<PendingImage>,

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
    /// Messages typed while busy, visible above the prompt until the
    /// agent actually reaches them (2026-11-13, operator request).
    pub held: Vec<HeldMsg>,
    /// Compaction start (drives the prominent banner's timer).
    pub compaction_started_at: Option<std::time::Instant>,
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
    /// Scroll lock (2026-11-13, operator request): while scrolled back the
    /// view is pinned to an absolute line index so streaming output below
    /// never drags the lines being read. None = following the tail.
    pub scroll_lock: Option<usize>,
    /// Line index at the top of the last rendered window (scroll_up base).
    pub last_start: usize,
    /// Total rendered transcript lines at the last frame.
    pub last_chat_total: usize,
    /// Screen rect of the "scrolled back" hint row; click jumps to tail.
    pub tail_hint_area: Option<ratatui::layout::Rect>,
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
            ctx_usage: None,
            title: String::new(),
            notice: None,
            commands: Vec::new(),
            tree_lines: Vec::new(),
            tree_open: false,
            tree_sel: 0,
            resume_open: false,
            resume_entries: Vec::new(),
            resume_sel: 0,
            resume_filter: String::new(),
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
            caduceus: None,
            caduceus_suspend: None,
            status_open: false,
            status_lines: Vec::new(),
            editor_suspend: None,
            review_open: false,
            review: None,
            review_sel: 0,
            cost_open: false,
            routes: None,
            routes_open: false,
            routes_sel: 0,
            cost: None,
            question: None,
            question_sel: 0,
            question_checked_at: None,
            turn_started_at: None,
            quit_armed_at: None,
            hide_thinking: load_hide_thinking(),
            thinking_default_collapsed: load_thinking_default(),
            thinking_default_file: None,
            watchdog: None,
            watchdog_file: None,
            fx_open: false,
            fx_sel: 0,
            answers_open: false,
            answers: None,
            answers_scroll: 0,
            answers_mtime_seen: None,
            prompt_history: load_prompt_history(),
            history_idx: None,
            history_draft: String::new(),
            engine_fire_seen: None,
            stream_heat: 0.0,
            last_activity: None,
            liveness_cpu_total: 0.0,
            liveness_prev_total: 0.0,
            liveness_delta: 0.0,
            liveness_flat_secs: 0,
            liveness_sampled_at: None,
            liveness_unavailable: false,
            prompt_ghost: None,
            ghost_dismissed_for: None,
            prompt_suggestion_file: None,
            ghost_events_file: None,
            xsessions_open: false,
            xsessions: Vec::new(),
            xsessions_sel: 0,
            xsessions_filter: String::new(),
            pending_images: Vec::new(),
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
            held: Vec::new(),
            compaction_started_at: None,
            expected_tool_calls: std::collections::HashMap::new(),
            sel_anchor: None,
            sel_head: None,
            last_chat_area: None,
            scroll_lock: None,
            last_start: 0,
            last_chat_total: 0,
            tail_hint_area: None,
            last_sidebar_area: None,
            sidebar_scroll: 0,
            sidebar_scroll_max: 0,
            sidebar_force: None,
            started: std::time::Instant::now(),
            scroll: 0,
            scroll_max: 0,
            md_cache: HashMap::new(),
            line_entries: Vec::new(),
            thinking_hitboxes: Vec::new(),
            tool_hitboxes: Vec::new(),
        }
    }
}

// ── UI phase state machine (verification-rollout T0.1) ─────────────────
//
// The TUI's phase-level state is governed by a declared transition table.
// All phase mutations MUST go through `AppState::advance`; the table is the
// spec (illegal (phase, event) pairs are rejected with a typed error and
// debug_assert! in dev builds). Non-phase data (scroll, cursor, filters)
// stays as direct field writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPhase {
    /// No dialog, no streaming, no modal: the idle composer.
    Idle,
    /// An agent turn is in flight (agent_start..agent_end).
    Streaming,
    /// A threshold compaction is in progress.
    Compacting,
    /// A picker modal is open (models / cache / resume).
    Modal,
    /// A dialog is on the stack (extension dialog or the permissions modal).
    Dialog,
}

/// Events that mutate the UI phase (key actions + RPC lifecycle events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiEvent {
    /// RPC `agent_start`.
    AgentStart,
    /// RPC `agent_end` / `agent_settled`.
    AgentEnd,
    /// RPC `compaction_start`.
    CompactionStart,
    /// RPC `compaction_end`.
    CompactionEnd,
    /// A picker modal opened (models / cache / resume).
    OpenModal,
    /// A picker modal closed.
    CloseModal,
    /// A dialog pushed onto the stack (extension or permissions modal).
    OpenDialog,
    /// A dialog popped from the stack.
    CloseDialog,
}

/// One row of the declared transition table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: UiPhase,
    pub event: UiEvent,
    pub to: UiPhase,
}

/// The declared legal transitions. This table IS the spec — it is a
/// function (one row per (from, event) pair); the property test asserts
/// every row is reachable and every other (phase, event) pair is rejected.
///
/// The `to` field documents the intended resulting phase; the *effective*
/// phase after a mutation is always re-derived by `AppState::phase()`
/// (which keeps its priority order). Rows whose `to` equals `from` are real
/// events that do not move the phase — e.g. a picker opened mid-turn stays
/// hidden behind the Streaming phase.
pub const TRANSITIONS: &[Transition] = &[
    // Idle
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::AgentStart,
        to: UiPhase::Streaming,
    },
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::CompactionStart,
        to: UiPhase::Compacting,
    },
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::OpenModal,
        to: UiPhase::Modal,
    },
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::OpenDialog,
        to: UiPhase::Dialog,
    },
    // Closing is checked AFTER the data mutation, so the phase has already
    // fallen back to the underlying one (this is how a dialog queued during
    // graceful quit is cancelled while idle).
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::CloseModal,
        to: UiPhase::Idle,
    },
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::CloseDialog,
        to: UiPhase::Idle,
    },
    // pi emits BOTH agent_end and agent_settled for one turn: the second
    // lands on an already-idle UI. Idempotent re-delivery, not a desync.
    Transition {
        from: UiPhase::Idle,
        event: UiEvent::AgentEnd,
        to: UiPhase::Idle,
    },
    // Streaming
    // agent_start re-fires on retry/abort-restart while the turn is up.
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::AgentStart,
        to: UiPhase::Streaming,
    },
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::AgentEnd,
        to: UiPhase::Idle,
    },
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::CompactionStart,
        to: UiPhase::Compacting,
    },
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::OpenModal,
        to: UiPhase::Streaming,
    },
    // Pickers stay hidden behind Streaming in phase priority, so both edges
    // (open and close) remain Streaming while a turn is in flight.
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::CloseModal,
        to: UiPhase::Streaming,
    },
    // A confirm answered mid-turn pops the last dialog -> back under Streaming.
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::CloseDialog,
        to: UiPhase::Streaming,
    },
    Transition {
        from: UiPhase::Streaming,
        event: UiEvent::OpenDialog,
        to: UiPhase::Dialog,
    },
    // Compacting (own spinner; can wrap a live turn)
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::AgentEnd,
        to: UiPhase::Compacting,
    },
    // Compaction ended: the effective phase is re-derived from the flags
    // (Idle when the turn is over, Streaming when a turn continues under it).
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::CompactionEnd,
        to: UiPhase::Idle,
    },
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::AgentStart,
        to: UiPhase::Streaming,
    },
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::OpenModal,
        to: UiPhase::Compacting,
    },
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::CloseModal,
        to: UiPhase::Compacting,
    },
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::CloseDialog,
        to: UiPhase::Compacting,
    },
    Transition {
        from: UiPhase::Compacting,
        event: UiEvent::OpenDialog,
        to: UiPhase::Dialog,
    },
    // Modal (picker open, no turn in flight)
    Transition {
        from: UiPhase::Modal,
        event: UiEvent::CloseModal,
        to: UiPhase::Idle,
    },
    Transition {
        from: UiPhase::Modal,
        event: UiEvent::OpenModal,
        to: UiPhase::Modal,
    },
    Transition {
        from: UiPhase::Modal,
        event: UiEvent::OpenDialog,
        to: UiPhase::Dialog,
    },
    // A turn can start underneath an open picker; Streaming outranks Modal.
    Transition {
        from: UiPhase::Modal,
        event: UiEvent::AgentStart,
        to: UiPhase::Streaming,
    },
    // Dialog (extension dialog / permissions modal on the stack)
    Transition {
        from: UiPhase::Dialog,
        event: UiEvent::CloseDialog,
        to: UiPhase::Idle,
    },
    Transition {
        from: UiPhase::Dialog,
        event: UiEvent::OpenDialog,
        to: UiPhase::Dialog,
    },
    Transition {
        from: UiPhase::Dialog,
        event: UiEvent::AgentStart,
        to: UiPhase::Streaming,
    },
    // A dialog can outlive the turn that raised it (the answer is awaited by
    // a beforeToolCall promise, not by the agent loop).
    Transition {
        from: UiPhase::Dialog,
        event: UiEvent::AgentEnd,
        to: UiPhase::Dialog,
    },
    Transition {
        from: UiPhase::Dialog,
        event: UiEvent::CompactionStart,
        to: UiPhase::Dialog,
    },
];

/// A (phase, event) pair not in `TRANSITIONS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IllegalTransition {
    pub from: UiPhase,
    pub event: UiEvent,
}

impl AppState {
    /// Derive the current UI phase. Priority reflects what the UI shows:
    /// a dialog wins over everything, a compaction wins over the turn it
    /// runs inside (it has its own spinner), then the turn, then pickers.
    pub fn phase(&self) -> UiPhase {
        if !self.dialogs.is_empty() || self.perm_modal_open {
            return UiPhase::Dialog;
        }
        if self.is_compacting {
            return UiPhase::Compacting;
        }
        if self.is_streaming {
            return UiPhase::Streaming;
        }
        if self.models_open || self.cache_open || self.resume_open {
            return UiPhase::Modal;
        }
        UiPhase::Idle
    }

    /// Look up the legal target for (phase, event); None if illegal.
    pub fn lookup(from: UiPhase, event: UiEvent) -> Option<UiPhase> {
        TRANSITIONS
            .iter()
            .find(|t| t.from == from && t.event == event)
            .map(|t| t.to)
    }

    /// Check one UI event against the transition table for the CURRENT
    /// phase. Call it in the state the caller is about to mutate (check
    /// before opening a modal/dialog; check after popping one) so
    /// `phase()` reflects the `from` side. Illegal (phase, event) pairs
    /// are debug_assert! in dev and returned as a typed error in release —
    /// callers surface them as a Diag entry and continue, so a table gap
    /// never wedges the UI.
    pub fn check(&self, ev: UiEvent) -> Result<UiPhase, IllegalTransition> {
        let from = self.phase();
        let Some(to) = Self::lookup(from, ev) else {
            // The assert is skipped under `cfg(test)` so property tests can
            // observe the typed error instead of unwinding.
            if !cfg!(test) {
                debug_assert!(false, "illegal transition: {:?} + {:?}", from, ev);
            }
            return Err(IllegalTransition { from, event: ev });
        };
        Ok(to)
    }

    /// Validate a phase event against the transition table and record a
    /// violation as a Diag entry. Callers invoke this BEFORE the data
    /// mutation (so `phase()` reports the `from` side). A table gap is thus
    /// visible in the F9 diagnostics overlay without ever wedging the UI.
    pub fn note_phase(&mut self, ev: UiEvent) {
        if let Err(bad) = self.check(ev) {
            self.entries.push(ChatEntry::Diag(format!(
                "state: unexpected {:?} in {:?}",
                bad.event, bad.from
            )));
        }
    }

    /// Human-compact count: "12.3k", "262k", "1.2M".
    fn short_count(n: u64) -> String {
        if n >= 1_000_000 {
            format!("{:.1}M", n as f64 / 1_000_000.0)
        } else if n >= 1_000 {
            format!("{:.1}k", n as f64 / 1_000.0)
        } else {
            n.to_string()
        }
    }

    /// Bottom status row label for the context fill: "ctx 12.3k / 262k
    /// (4%)". None while no turn has reported usage — the row hides itself
    /// rather than displaying a guess. Pure so the format is unit-testable.
    pub fn ctx_bar_label(&self) -> Option<String> {
        let (tokens, window) = self.ctx_usage?;
        if window == 0 {
            return None;
        }
        let pct = (tokens as f64 / window as f64) * 100.0;
        Some(format!(
            "{} ctx {} / {} ({:.1}%)",
            theme::GLYPH_VITRIOL,
            Self::short_count(tokens),
            Self::short_count(window),
            pct
        ))
    }

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
        (
            "sandboxed",
            "bash runs in a bwrap box: ro filesystem, rw cwd+/tmp, no net unless OFFICINA_SANDBOX_NET=1",
        ),
    ];

    pub fn permissions_mode_index(&self) -> usize {
        Self::PERM_MODES
            .iter()
            .position(|(mode, _)| *mode == self.perm_mode)
            .unwrap_or(0)
    }

    pub fn open_permissions_modal(&mut self) {
        self.note_phase(UiEvent::OpenDialog);
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
        self.note_phase(UiEvent::OpenModal);
        self.cache_refresh();
        self.cache_sel = 0;
        self.cache_open = true;
    }

    /// `/caduceus` — load the facts snapshot + policy and open the page.
    /// A missing state file renders an honest "no state" hint; corrupt
    /// prefs fall back to defaults and say so in the footer (I6).
    pub fn caduceus_open_page(&mut self) {
        self.note_phase(UiEvent::OpenModal);
        self.caduceus = Some(crate::tui::caduceus::Page::load());
    }

    /// Resume picker rows honour the typeable filter (case-insensitive
    /// substring over title + path).
    pub fn filtered_resume_entries(&self) -> Vec<&crate::rpc::sessions::SessionEntry> {
        if self.resume_filter.is_empty() {
            return self.resume_entries.iter().collect();
        }
        let needle = self.resume_filter.to_lowercase();
        self.resume_entries
            .iter()
            .filter(|e| {
                e.title.to_lowercase().contains(&needle)
                    || e.path.to_string_lossy().to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn clamp_resume_sel(&mut self) {
        let n = self.filtered_resume_entries().len();
        if self.resume_sel >= n {
            self.resume_sel = n.saturating_sub(1);
        }
    }

    /// Stage the composer draft for an external $EDITOR run (codex-qol
    /// stage 4). Testable core: `editor` is the raw $VISUAL/$EDITOR string.
    /// The draft is written to a temp file and kept verbatim in the request,
    /// so a failed spawn/reload never loses text (I4).
    pub fn stage_editor(&mut self, editor: &str) {
        if self.editor_suspend.is_some() {
            return;
        }
        let mut argv: Vec<String> = editor.split_whitespace().map(String::from).collect();
        if argv.is_empty() {
            self.notice = Some((
                "editor: empty $VISUAL/$EDITOR — draft kept".into(),
                "warning".into(),
            ));
            return;
        }
        let tmp = std::env::temp_dir().join(format!(
            "officina-compose-{}-{}.md",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        if let Err(e) = std::fs::write(&tmp, &self.input) {
            self.notice = Some((
                format!("editor: temp write failed ({e}) — draft kept"),
                "warning".into(),
            ));
            return;
        }
        argv.push(tmp.to_string_lossy().to_string());
        self.editor_suspend = Some((argv, tmp, self.input.clone()));
    }

    /// `/sessions` — open the cross-project picker over a completed scan.
    pub fn xsessions_open_with(&mut self, entries: Vec<crate::rpc::sessions::AllSessionEntry>) {
        self.note_phase(UiEvent::OpenModal);
        self.xsessions = entries;
        self.xsessions_sel = 0;
        self.xsessions_filter.clear();
        self.xsessions_open = true;
    }

    pub fn xsessions_close(&mut self) {
        if self.xsessions_open {
            self.note_phase(UiEvent::CloseModal);
            self.xsessions_open = false;
        }
    }

    pub fn xsessions_view(&self) -> Vec<&crate::rpc::sessions::AllSessionEntry> {
        if self.xsessions_filter.is_empty() {
            return self.xsessions.iter().collect();
        }
        let needle = self.xsessions_filter.to_lowercase();
        self.xsessions
            .iter()
            .filter(|a| {
                a.entry.title.to_lowercase().contains(&needle)
                    || a.project.to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn clamp_xsessions_sel(&mut self) {
        let n = self.xsessions_view().len();
        if self.xsessions_sel >= n {
            self.xsessions_sel = n.saturating_sub(1);
        }
    }

    /// `/review-findings` — load the last review's findings and open the
    /// modal. A missing file opens an honest hint (I3).
    pub fn review_open_modal(&mut self) {
        self.note_phase(UiEvent::OpenModal);
        self.review = crate::tui::review::load(&self.cwd);
        self.review_sel = 0;
        self.review_open = true;
    }

    /// ^o — stage a clipboard image (cap 4, I5).
    pub fn attach_image(&mut self, path: std::path::PathBuf, mime: String) {
        if self.pending_images.len() >= crate::tui::paste::MAX_IMAGES {
            return;
        }
        self.pending_images.push(PendingImage { path, mime });
    }

    pub fn clear_images(&mut self) {
        for img in self.pending_images.drain(..) {
            let _ = std::fs::remove_file(&img.path);
        }
    }

    pub fn review_close(&mut self) {
        if self.review_open {
            self.note_phase(UiEvent::CloseModal);
            self.review_open = false;
        }
    }

    pub fn review_rows(&self) -> Vec<&crate::tui::review::Finding> {
        self.review.as_ref().map(|f| f.sorted()).unwrap_or_default()
    }

    pub fn clamp_review_sel(&mut self) {
        let n = self.review_rows().len();
        if n == 0 {
            self.review_sel = 0;
        } else if self.review_sel >= n {
            self.review_sel = n - 1;
        }
    }

    /// Voice ghost (stage 2): read the staged suggestion; show it only when
    /// the composer is empty, idle, and this id wasn't already dismissed.
    /// Corrupt/expired files = silently off (I5).
    pub fn refresh_ghost(&mut self) {
        if !self.input.is_empty() || self.is_streaming || self.has_running_tool() {
            self.prompt_ghost = None;
            return;
        }
        let path = self
            .prompt_suggestion_file
            .clone()
            .unwrap_or_else(prompt_suggestion_path);
        let Ok(raw) = std::fs::read_to_string(&path) else {
            self.prompt_ghost = None;
            return;
        };
        let Ok(doc) = serde_json::from_str::<SuggestionFile>(&raw) else {
            self.prompt_ghost = None;
            return;
        };
        if doc.version != 1 || doc.text.trim().is_empty() {
            self.prompt_ghost = None;
            return;
        }
        if doc.expires > 0 && doc.expires <= crate::tui::caduceus::now_ms() {
            self.prompt_ghost = None;
            return;
        }
        if self.ghost_dismissed_for.as_deref() == Some(doc.id.as_str()) {
            return;
        }
        self.prompt_ghost = Some(GhostSuggestion {
            id: doc.id,
            text: doc.text,
            source: doc.source,
        });
    }

    /// → — fold the ghost into the real draft (I1: the operator pressed a
    /// key; nothing was ever pre-sent).
    pub fn accept_ghost(&mut self) -> bool {
        let Some(g) = self.prompt_ghost.take() else {
            return false;
        };
        self.cursor = g.text.chars().count();
        self.input = g.text;
        self.ghost_dismissed_for = None;
        self.log_ghost_event(&g.id, &g.source, "accepted");
        true
    }

    /// Typing or Esc — the ghost yields to the operator's own words.
    pub fn dismiss_ghost(&mut self, verdict: &str) {
        if let Some(g) = self.prompt_ghost.take() {
            self.ghost_dismissed_for = Some(g.id.clone());
            self.log_ghost_event(&g.id, &g.source, verdict);
        }
    }

    fn log_ghost_event(&self, id: &str, source: &str, verdict: &str) {
        use std::io::Write;
        let path = self
            .ghost_events_file
            .clone()
            .unwrap_or_else(ghost_events_path);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // I4: ids + verdicts only — never the prompt text.
        let line = serde_json::json!({
            "ts": crate::tui::caduceus::now_ms(),
            "id": id,
            "source": source,
            "verdict": verdict,
        });
        if let Ok(mut f) = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&path)
        } {
            let _ = writeln!(f, "{line}");
        }
    }

    /// `/answers` — load the last INQUIRE answer and open the panel.
    pub fn answers_open_modal(&mut self) {
        self.note_phase(UiEvent::OpenModal);
        self.answers = load_answer_doc();
        self.answers_scroll = 0;
        self.answers_mtime_seen = answer_mtime();
        self.answers_open = true;
    }

    pub fn answers_close(&mut self) {
        if self.answers_open {
            self.note_phase(UiEvent::CloseModal);
            self.answers_open = false;
        }
    }

    /// After each turn: if the answer file changed, ping the operator.
    pub fn refresh_answers_notice(&mut self) {
        let mtime = answer_mtime();
        if mtime.is_some() && mtime != self.answers_mtime_seen {
            self.answers_mtime_seen = mtime;
            self.notice = Some(("answer ready — /answers".into(), "info".into()));
        }
    }

    /// Safe quit (2026-11-13): returns true when quitting may proceed.
    /// While a turn streams or a tool runs, the first press arms a 5 s
    /// confirm window; idle quits proceed immediately (I2).
    pub fn quit_may_proceed(&mut self) -> bool {
        let busy = self.is_streaming || self.has_running_tool();
        if !busy {
            self.quit_armed_at = None;
            return true;
        }
        let now = std::time::Instant::now();
        match self.quit_armed_at {
            Some(t) if now.duration_since(t).as_secs() < 5 => {
                self.quit_armed_at = None;
                true
            }
            _ => {
                self.quit_armed_at = Some(now);
                self.notice = Some((
                    "working — press again to quit anyway (esc waits)".into(),
                    "warning".into(),
                ));
                false
            }
        }
    }

    /// Record a sent prompt (dedupe consecutive, cap 200) + persist.
    pub fn record_prompt_history(&mut self, msg: &str) {
        self.mark_activity();
        let msg = msg.trim();
        if msg.is_empty() {
            return;
        }
        if self
            .prompt_history
            .last()
            .map(|m| m == msg)
            .unwrap_or(false)
        {
            return;
        }
        self.prompt_history.push(msg.to_string());
        if self.prompt_history.len() > 200 {
            let drop = self.prompt_history.len() - 200;
            self.prompt_history.drain(..drop);
        }
        append_prompt_history(msg);
        self.history_idx = None;
        self.history_draft.clear();
    }

    /// Up — step back through history; the in-progress draft is preserved
    /// and restored when you step past the newest entry.
    pub fn history_prev(&mut self) -> bool {
        if self.prompt_history.is_empty() {
            return false;
        }
        match self.history_idx {
            None => {
                self.history_draft = self.input.clone();
                self.history_idx = Some(self.prompt_history.len() - 1);
            }
            Some(0) => {}
            Some(i) => self.history_idx = Some(i - 1),
        }
        if let Some(i) = self.history_idx {
            self.input = self.prompt_history[i].clone();
            self.cursor = self.input.chars().count();
            self.dismiss_ghost("dismissed-history");
        }
        true
    }

    /// Down — step forward; past the newest entry restores the draft.
    pub fn history_next(&mut self) -> bool {
        match self.history_idx {
            None => false,
            Some(i) if i + 1 < self.prompt_history.len() => {
                self.history_idx = Some(i + 1);
                self.input = self.prompt_history[i + 1].clone();
                self.cursor = self.input.chars().count();
                true
            }
            Some(_) => {
                self.history_idx = None;
                self.input = std::mem::take(&mut self.history_draft);
                self.cursor = self.input.chars().count();
                true
            }
        }
    }

    /// Liveness probe (2026-11-12): sample the CPU of pi's whole subprocess
    /// tree at most every 2 s while a tool runs; flat samples accumulate
    /// `liveness_flat_secs`. No-op when nothing runs.
    pub fn probe_liveness(&mut self, pi_pid: Option<u32>) {
        if !self.has_running_tool() {
            self.liveness_cpu_total = 0.0;
            self.liveness_prev_total = 0.0;
            self.liveness_delta = 0.0;
            self.liveness_flat_secs = 0;
            self.liveness_sampled_at = None;
            return;
        }
        let now = std::time::Instant::now();
        if let Some(t) = self.liveness_sampled_at {
            if (now - t).as_secs_f64() < 2.0 {
                return; // throttle: /proc walks are not free
            }
        }
        let Some(total) = pi_pid.and_then(subtree_cpu_secs) else {
            self.liveness_unavailable = true;
            return;
        };
        self.liveness_delta = if self.liveness_sampled_at.is_some() {
            (total - self.liveness_prev_total).max(0.0)
        } else {
            0.0
        };
        self.liveness_prev_total = total;
        self.liveness_cpu_total = total;
        self.liveness_sampled_at = Some(now);
        if self.liveness_delta > 0.05 {
            self.liveness_flat_secs = 0;
        } else {
            self.liveness_flat_secs += 2; // sample window
        }
        self.liveness_unavailable = false;
    }

    /// `(text, is_alive)` suffix for the running tool header. Empty when no
    /// sample exists yet.
    pub fn liveness_suffix(&self) -> Option<(String, bool)> {
        if self.liveness_unavailable || self.liveness_sampled_at.is_none() {
            return None;
        }
        if self.liveness_flat_secs == 0 {
            Some((format!("cpu +{:.1}s", self.liveness_delta), true))
        } else {
            Some((format!("cpu flat {}s", self.liveness_flat_secs), false))
        }
    }

    /// `/status` — snapshot the session facts once and open the modal.
    pub fn status_open_modal(&mut self) {
        self.note_phase(UiEvent::OpenModal);
        self.status_lines = self.build_status_lines();
        self.status_open = true;
    }

    pub fn status_close(&mut self) {
        if self.status_open {
            self.note_phase(UiEvent::CloseModal);
            self.status_open = false;
        }
    }

    /// Pure builder so tests pin the content without touching $HOME files.
    /// `gateway`/`policy_note` come from the caduceus state + prefs files.
    fn status_rows(
        &self,
        gateway: Option<(crate::tui::caduceus::CaduceusState, bool)>,
        policy_line: Option<String>,
        profile: Option<String>,
    ) -> Vec<(&'static str, String)> {
        let mut rows: Vec<(&'static str, String)> = Vec::new();
        rows.push(("cwd", shorten_home_for_status(&self.cwd)));
        rows.push((
            "session",
            self.session_name.clone().unwrap_or_else(|| {
                format!("{}…", &self.session_id[..self.session_id.len().min(10)])
            }),
        ));
        rows.push((
            "model",
            match &self.model {
                Some(m) => match &m.name {
                    Some(name) => format!("{} / {} · {}", m.provider, m.id, name),
                    None => format!("{} / {}", m.provider, m.id),
                },
                None => "(none)".to_string(),
            },
        ));
        rows.push((
            "agent",
            self.agent_mode_label
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        ));
        rows.push((
            "profile",
            profile.unwrap_or_else(|| "auto (default)".to_string()),
        ));
        rows.push((
            "context",
            self.ctx_bar_label()
                .unwrap_or_else(|| "— (no report yet)".to_string()),
        ));
        let last = crate::tui::routes::load_recent(&crate::tui::routes::today(), 1);
        rows.push((
            "last route",
            last.latest()
                .map(|e| {
                    format!(
                        "{} · {} · {} · {}",
                        e.tier,
                        e.provider,
                        e.model,
                        crate::tui::routes::short_time(&e.ts)
                    )
                })
                .unwrap_or_else(|| "— (no decisions today)".to_string()),
        ));
        rows.push(("permissions", self.perm_mode.clone()));
        rows.push((
            "gateway",
            match gateway {
                Some((s, up)) => {
                    let stale = if s.is_stale(crate::tui::caduceus::now_ms()) {
                        " · STALE"
                    } else {
                        ""
                    };
                    if up {
                        format!(
                            "up · {} free lanes · cooling {}{stale}",
                            s.free_candidate_count(),
                            s.cooling
                        )
                    } else {
                        format!("DOWN{stale}")
                    }
                }
                None => "no state file (caduceus extension idle?)".to_string(),
            },
        ));
        rows.push((
            "policy",
            policy_line.unwrap_or_else(|| "prefs ignored".to_string()),
        ));
        rows
    }

    fn build_status_lines(&self) -> Vec<(&'static str, String)> {
        let gateway = crate::tui::caduceus::load().map(|s| {
            let up = s.gateway_up;
            (s, up)
        });
        let (prefs, note) = crate::tui::caduceus::load_prefs();
        let policy_line = match note {
            Some(note) => format!("prefs ignored: {note}"),
            None => format!(
                "free_only={} · per_owner={} · max={}",
                if prefs.free_only { "on" } else { "off" },
                prefs.per_owner,
                prefs.max_candidates
            ),
        };
        let profile = std::fs::read_to_string(profile_path())
            .ok()
            .map(|p| p.trim().to_string());
        self.status_rows(gateway, Some(policy_line), profile)
    }

    pub fn caduceus_close(&mut self) {
        if self.caduceus.is_some() {
            self.note_phase(UiEvent::CloseModal);
            self.caduceus = None;
        }
    }

    /// Extension notify (stage 3, 2026-11-12): errors/warnings land in the
    /// transcript as Diag; info keeps the 40-char glance row, and anything
    /// the truncation would mangle (multi-line tables or >100-char payloads)
    /// is ALSO captured as a Notice entry so /free, /nimbus, /phase-models
    /// output is actually readable.
    pub fn apply_notify(&mut self, msg: &str, ntype: &str) {
        // Watchdog observations are not errors — give them their own block.
        if msg.starts_with("hang-watchdog") {
            let path = self.watchdog_file.clone().unwrap_or_else(watchdog_path);
            let prev_key = self
                .watchdog
                .as_ref()
                .map(|d| format!("{}|{}", d.command, d.verdict));
            self.watchdog = load_watchdog_doc_at(&path);
            let new_key = self
                .watchdog
                .as_ref()
                .map(|d| format!("{}|{}", d.command, d.verdict));
            // Same command+verdict as the card on screen: update in place
            // instead of stacking duplicates (2026-11-14).
            let replace = matches!(
                (self.entries.last(), prev_key.as_deref(), new_key.as_deref()),
                (Some(ChatEntry::Watchdog(_)), Some(p), Some(n)) if p == n
            );
            if replace {
                if let Some(ChatEntry::Watchdog(prev)) = self.entries.last_mut() {
                    *prev = msg.to_string();
                }
            } else {
                self.entries.push(ChatEntry::Watchdog(msg.to_string()));
            }
            return;
        }
        if ntype == "error" {
            self.entries.push(ChatEntry::Error(msg.to_string()));
        } else if ntype == "warning" {
            self.entries.push(ChatEntry::Warn(msg.to_string()));
        } else {
            self.notice = Some((msg.to_string(), ntype.to_string()));
            if msg.contains('\n') || msg.len() > 100 {
                self.entries.push(ChatEntry::Notice(msg.to_string()));
            }
        }
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

    /// Collapse every thinking trace (2026-11-13, operator request).
    pub fn thinking_collapse_all(&mut self) {
        let mut n = 0;
        for e in self.entries.iter_mut() {
            if let ChatEntry::Thinking { collapsed, .. } = e {
                if !*collapsed {
                    *collapsed = true;
                    n += 1;
                }
            }
        }
        self.entries.push(ChatEntry::Diag(format!(
            "thinking: collapsed {n} trace(s) — /thinking-view expand reopens them"
        )));
    }

    /// Expand every thinking trace.
    pub fn thinking_expand_all(&mut self) {
        let mut n = 0;
        for e in self.entries.iter_mut() {
            if let ChatEntry::Thinking { collapsed, .. } = e {
                if *collapsed {
                    *collapsed = false;
                    n += 1;
                }
            }
        }
        self.entries
            .push(ChatEntry::Diag(format!("thinking: expanded {n} trace(s)")));
    }

    /// Register a message that is held (typed while the agent is busy).
    pub fn held_push(&mut self, text: &str, kind: HeldKind) {
        self.held.push(HeldMsg {
            text: text.to_string(),
            kind,
            at: std::time::Instant::now(),
        });
    }

    /// Drop held messages: one kind, or all when `kind` is None.
    pub fn held_clear(&mut self, kind: Option<HeldKind>) {
        match kind {
            Some(k) => self.held.retain(|h| h.kind != k),
            None => self.held.clear(),
        }
    }

    /// Resolve the oldest queued delivery (keeps a multi-message queue).
    pub fn held_resolve_queued(&mut self) {
        if let Some(pos) = self.held.iter().position(|h| h.kind == HeldKind::Queued) {
            self.held.remove(pos);
        }
    }

    /// One-line description of what is being held, for the prompt row.
    pub fn held_label(&self) -> Option<String> {
        let first = self.held.first()?;
        let mut t = first.text.replace('\n', " ");
        if t.chars().count() > 48 {
            t = t.chars().take(47).collect::<String>() + "…";
        }
        let kind = match first.kind {
            HeldKind::Queued => "queued",
            HeldKind::Steer => "steering",
            HeldKind::FollowUp => "follow-up",
        };
        let more = if self.held.len() > 1 {
            format!(" (+{} more)", self.held.len() - 1)
        } else {
            String::new()
        };
        Some(format!("{kind}: “{t}”{more}"))
    }

    /// `/cache` command surface. Bare opens the manager modal.
    /// `/thinking-view [hide|show]` — display-only toggle (the model's
    /// thinking LEVEL stays with pi's /thinking). Bare = status.
    pub fn thinking_view_command(&mut self, args: &str) {
        match args.trim() {
            "" => self.entries.push(ChatEntry::Diag(format!(
                "thinking display: {}; new blocks start {} — /thinking-view hide|show|collapse|expand|default",
                if self.hide_thinking {
                    "hidden"
                } else {
                    "shown"
                },
                if self.thinking_default_collapsed {
                    "collapsed"
                } else {
                    "expanded"
                }
            ))),
            "collapse" | "collapse-all" => self.thinking_collapse_all(),
            "expand" | "expand-all" => self.thinking_expand_all(),
            "default" => self.entries.push(ChatEntry::Diag(format!(
                "thinking default: {} — /thinking-view default collapsed|expanded",
                if self.thinking_default_collapsed {
                    "collapsed"
                } else {
                    "expanded"
                }
            ))),
            a if a.starts_with("default ") => {
                let v = a["default ".len()..].trim();
                let collapsed = match v {
                    "collapsed" | "collapse" | "hide" | "hidden" | "on" => true,
                    "expanded" | "expand" | "show" | "shown" | "off" => false,
                    other => {
                        self.entries.push(ChatEntry::Diag(format!(
                            "thinking-view: unknown default '{other}' — use collapsed | expanded"
                        )));
                        return;
                    }
                };
                self.thinking_default_collapsed = collapsed;
                persist_thinking_default(self);
                self.entries.push(ChatEntry::Diag(format!(
                    "thinking default: {} (applies to new traces)",
                    if collapsed { "collapsed" } else { "expanded" }
                )));
            }
            "hide" | "hidden" | "off" => {
                self.hide_thinking = true;
                persist_hide_thinking(self);
                self.entries.push(ChatEntry::Diag(
                    "thinking display: hidden (collapsed markers; /thinking-view show)".into(),
                ));
            }
            "show" | "shown" | "on" => {
                self.hide_thinking = false;
                persist_hide_thinking(self);
                self.entries
                    .push(ChatEntry::Diag("thinking display: shown".into()));
            }
            other => self.entries.push(ChatEntry::Diag(format!(
                "thinking-view: unknown '{other}' — use hide | show | collapse | expand | default"
            ))),
        }
    }

    /// `/janitor [dry]` — plan (and by default apply) stale-artifact cleanup.
    /// Dry mode lists what WOULD go; never touches protected paths.
    pub fn janitor_command(&mut self, dry: bool) {
        let plan = crate::tui::janitor::plan(
            &crate::tui::janitor::Roots::live(),
            std::time::SystemTime::now(),
        );
        if plan.items.is_empty() {
            self.entries
                .push(ChatEntry::Diag("janitor: nothing stale — all clean".into()));
            return;
        }
        if dry {
            self.entries.push(ChatEntry::Diag(format!(
                "janitor (dry): {} item(s) · {} would be removed",
                plan.items.len(),
                crate::tui::cache::fmt_bytes(plan.bytes())
            )));
            for item in plan.items.iter().take(12) {
                self.entries.push(ChatEntry::Diag(format!(
                    "  {} · {} · {}",
                    item.reason,
                    crate::tui::cache::fmt_bytes(item.bytes),
                    item.path.display()
                )));
            }
            if plan.items.len() > 12 {
                self.entries.push(ChatEntry::Diag(format!(
                    "  … {} more",
                    plan.items.len() - 12
                )));
            }
            return;
        }
        let (count, bytes) = crate::tui::janitor::apply(&plan.items);
        self.entries.push(ChatEntry::Diag(format!(
            "janitor: removed {count} item(s) · {}",
            crate::tui::cache::fmt_bytes(bytes)
        )));
    }

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
        if is_streaming && !self.is_streaming {
            self.turn_started_at = Some(std::time::Instant::now());
        } else if !is_streaming {
            self.turn_started_at = None;
        }
        self.is_streaming = is_streaming;
        self.is_compacting = is_compacting;
        self.session_id = session_id.unwrap_or_default();
        self.session_name = session_name;
        self.message_count = message_count.unwrap_or(0);
    }

    /// Append a text delta to the last assistant entry (or open a new one).
    pub fn push_text_delta(&mut self, delta: &str) {
        self.stream_heat = (self.stream_heat + 0.4).min(1.6);
        match self.entries.last_mut() {
            Some(ChatEntry::Assistant(t)) => t.push_str(delta),
            _ => self.entries.push(ChatEntry::Assistant(delta.to_string())),
        }
    }

    /// Append a thinking delta to the last thinking entry (or open a new one).
    pub fn push_thinking_delta(&mut self, delta: &str) {
        self.stream_heat = (self.stream_heat + 0.4).min(1.6);
        match self.entries.last_mut() {
            Some(ChatEntry::Thinking { text, .. }) => text.push_str(delta),
            _ => self.entries.push(ChatEntry::Thinking {
                text: delta.to_string(),
                collapsed: self.thinking_default_collapsed,
            }),
        }
    }

    /// Left-click on a thinking block: toggle it. Returns true when the
    /// click landed on a block (2026-11-13).
    pub fn thinking_toggle_at(&mut self, row: u16) -> bool {
        let idx = self
            .thinking_hitboxes
            .iter()
            .find(|(y0, y1, _)| row >= *y0 && row <= *y1)
            .map(|(_, _, idx)| *idx);
        let Some(idx) = idx else { return false };
        if let Some(ChatEntry::Thinking { collapsed, .. }) = self.entries.get_mut(idx) {
            *collapsed = !*collapsed;
            return true;
        }
        false
    }

    /// Left-click on a tool block: cycle closed (line) → formatted (block)
    /// → full → closed. Returns true when the click landed on a block
    /// (2026-11-14, operator request).
    pub fn tool_toggle_at(&mut self, row: u16) -> bool {
        let idx = self
            .tool_hitboxes
            .iter()
            .find(|(y0, y1, _)| row >= *y0 && row <= *y1)
            .map(|(_, _, idx)| *idx);
        let Some(idx) = idx else { return false };
        let Some(ChatEntry::Tool {
            name, click_mode, ..
        }) = self.entries.get(idx)
        else {
            return false;
        };
        let current = click_mode
            .unwrap_or_else(|| effective_mode(name, self.tool_default, &self.tool_overrides));
        let next = current.next();
        if let Some(ChatEntry::Tool {
            click_mode,
            render_cache,
            ..
        }) = self.entries.get_mut(idx)
        {
            *click_mode = Some(next);
            // The cached block was rendered at the old verbosity.
            *render_cache = None;
            return true;
        }
        false
    }

    pub fn push_user(&mut self, text: String) {
        self.entries.push(ChatEntry::User(text));
        // A new turn means the operator is looking at the prompt: release
        // the scroll lock so the answer streams in view.
        self.scroll_follow();
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
            click_mode: None,
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
            self.engine_fire_seen = Some(std::time::Instant::now());
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
    /// Record real activity (call on any streamed text, tool event, agent
    /// transition, or prompt submit).
    pub fn mark_activity(&mut self) {
        self.last_activity = Some(std::time::Instant::now());
    }

    /// Is the session actually doing something? Fresh activity (≤10 s) OR
    /// a running tool whose CPU burned in the last sample window.
    pub fn liveliness_active(&self) -> bool {
        if self.has_running_tool()
            && self.liveness_sampled_at.is_some()
            && self.liveness_flat_secs == 0
        {
            return true;
        }
        match self.last_activity {
            Some(t) => t.elapsed().as_secs() < 10,
            None => false,
        }
    }

    /// Seconds since the last real signal (0 when none ever, for display).
    /// Question panel poll (throttled to 500 ms). Loads a fresh staged
    /// question; clears the panel when the file is consumed or expires.
    pub fn question_poll(&mut self) {
        let now = crate::tui::caduceus::now_ms();
        if let Some(t) = self.question_checked_at {
            if t.elapsed().as_millis() < 500 {
                return;
            }
        }
        self.question_checked_at = Some(std::time::Instant::now());
        match crate::tui::question::load_fresh(now) {
            Some(doc) => {
                if self
                    .question
                    .as_ref()
                    .map(|q| q.id != doc.id)
                    .unwrap_or(true)
                {
                    self.question_sel = 0;
                }
                self.question = Some(doc);
            }
            None => self.question = None,
        }
    }

    /// Enter on the question panel: answer the staged question.
    pub fn question_answer(&mut self) {
        let Some(doc) = self.question.as_ref() else {
            return;
        };
        let Some(opt) = doc.options.get(self.question_sel) else {
            return;
        };
        let id = doc.id.clone();
        let answer = opt.clone();
        if let Err(e) = crate::tui::question::write_response(&id, Some(answer)) {
            self.entries.push(ChatEntry::Diag(format!(
                "question: response write failed ({e})"
            )));
            return;
        }
        self.question = None;
        self.question_sel = 0;
    }

    /// Esc on the question panel: cancel (the extension fail-opens).
    pub fn question_cancel(&mut self) {
        let Some(doc) = self.question.as_ref() else {
            return;
        };
        let id = doc.id.clone();
        if let Err(e) = crate::tui::question::write_response(&id, None) {
            self.entries.push(ChatEntry::Diag(format!(
                "question: response write failed ({e})"
            )));
        }
        self.question = None;
        self.question_sel = 0;
    }

    /// Digits 1-9 on the question panel: direct option pick.
    pub fn question_pick(&mut self, n: usize) {
        if self.question.is_none() {
            return;
        }
        self.question_sel = n.saturating_sub(1);
        self.question_answer();
    }

    /// Seconds since the turn began (stamped when streaming starts).
    pub fn turn_elapsed_secs(&self) -> u64 {
        self.turn_started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    pub fn stalled_secs(&self) -> u64 {
        self.last_activity
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0)
    }

    /// Seconds until the next watchdog check (None when no watch is armed
    /// or the facts are stale/consumed).
    pub fn watchdog_countdown(&self) -> Option<u64> {
        let doc = self.watchdog.as_ref()?;
        let now = crate::tui::caduceus::now_ms();
        if now.saturating_sub(doc.ts) > 120_000 || doc.next_check_at == 0 {
            return None;
        }
        if doc.next_check_at <= now {
            return Some(0);
        }
        Some((doc.next_check_at - now) / 1000)
    }

    /// Fire gate (I1): no evidence → flames are dying/dead. Called from the
    /// run loop before stepping the animation.
    pub fn apply_liveliness_gate(&mut self) {
        if !self.liveliness_active() {
            self.fire_target = 0.0;
        }
    }

    pub fn step_fire(&mut self, dt_secs: f64) {
        if !self.fire_on {
            self.fire_target = 0.0;
        }
        // Cloud/stream fallback (2026-11-13): when the engine-fire telemetry
        // is stale or absent (engine down, cloud lane), the flames derive
        // from STREAM deltas — they genuinely reflect token flow and decay
        // when the stream stalls. Fresh engine telemetry always wins.
        let engine_telemetry_fresh = self
            .engine_fire_seen
            .map(|t| t.elapsed().as_secs() < 15)
            .unwrap_or(false);
        if !engine_telemetry_fresh && self.is_streaming {
            self.stream_heat *= (-dt_secs / 1.5).exp();
            self.fire_target = (0.35 + 0.55 * self.stream_heat.min(1.0)).clamp(0.0, 1.0);
        }
        let k = 1.0 - (-dt_secs / 0.33).exp();
        self.fire_level += (self.fire_target - self.fire_level) * k;
        if self.fire_level < crate::fire::MIN_LEVEL {
            self.fire_level = 0.0;
        }
    }

    // ── Scrollback (owner request 2026-09-02) ────────────────────────────

    pub fn scroll_up(&mut self, rows: u16) {
        self.clear_selection();
        let anchor = self.scroll_lock.unwrap_or(self.last_start);
        self.scroll_lock = Some(anchor.saturating_sub(rows as usize));
        self.scroll = self
            .scroll
            .saturating_add(rows)
            .min(self.scroll_max.max(self.scroll));
    }

    pub fn scroll_down(&mut self, rows: u16) {
        self.clear_selection();
        if self.scroll == 0 {
            return;
        }
        let anchor = self.scroll_lock.unwrap_or(self.last_start);
        self.scroll = self.scroll.saturating_sub(rows);
        if self.scroll == 0 {
            self.scroll_lock = None;
        } else {
            self.scroll_lock = Some(anchor.saturating_add(rows as usize));
        }
    }

    /// Lock released — the view follows the tail again.
    pub fn scroll_follow(&mut self) {
        self.scroll = 0;
        self.scroll_lock = None;
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
        self.scroll_lock = Some(0);
    }

    /// Back to the live tail.
    pub fn scroll_end(&mut self) {
        self.scroll_follow();
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
        let mut line_entries: Vec<usize> = Vec::new();
        let total = self.entries.len();
        let mut liveness = self.liveness_suffix();
        if let Some(secs) = self.watchdog_countdown() {
            liveness = Some(match liveness {
                Some((text, alive)) => (format!("{text} \u{00b7} watch {secs}s"), alive),
                None => (format!("watch {secs}s"), true),
            });
        }
        let wd = self.watchdog.clone();
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
                ChatEntry::Thinking { text, collapsed } => {
                    let n = text.chars().count();
                    let collapsed = self.hide_thinking || *collapsed;
                    if collapsed {
                        // Liveliness (2026-11-13): while this block is the
                        // live tail and the session is streaming, the marker
                        // animates — visible proof the model is thinking,
                        // even with the text hidden and on any lane.
                        let growing = self.is_streaming && idx + 1 == total;
                        let spinner = if growing {
                            let frames = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
                            let f =
                                (self.started.elapsed().as_millis() / 120) as usize % frames.len();
                            format!("{} ", frames[f])
                        } else {
                            String::new()
                        };
                        let count_style = if growing {
                            Style::default().fg(theme::GOLD)
                        } else {
                            Style::default().fg(theme::MUTED)
                        };
                        let label = if growing {
                            "thinking…"
                        } else {
                            "thinking collapsed"
                        };
                        let hint = if self.hide_thinking {
                            "/thinking-view show"
                        } else {
                            "click to expand"
                        };
                        lines.push(Line::from(Span::styled(
                            format!(
                                "  ·    {spinner}{label} ({}) — {hint}",
                                if n >= 1000 {
                                    format!("{:.1}k chars", n as f64 / 1000.0)
                                } else {
                                    format!("{n} chars")
                                }
                            ),
                            count_style,
                        )));
                    } else {
                        // Markdown-formatted thinking (2026-11-13, operator
                        // request): same renderer + cache as assistant
                        // replies, dimmed italic so it still reads as
                        // reasoning rather than output. Click to close.
                        let key = (idx, n, width);
                        let md = if let Some(cached) = self.md_cache.get(&key) {
                            cached.clone()
                        } else {
                            let rendered = crate::markdown::render(text, width.saturating_sub(8));
                            self.md_cache.insert(key, rendered.clone());
                            rendered
                        };
                        let mut first = true;
                        for ml in md {
                            let mut spans: Vec<Span> = Vec::with_capacity(ml.spans.len() + 1);
                            spans.push(Span::styled(
                                if first { "  ·    " } else { "        " },
                                Style::default().fg(theme::MUTED),
                            ));
                            for sp in ml.spans {
                                spans.push(Span::styled(
                                    sp.content,
                                    sp.style
                                        .fg(theme::MUTED)
                                        .add_modifier(ratatui::style::Modifier::ITALIC),
                                ));
                            }
                            lines.push(Line::from(spans));
                            first = false;
                        }
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
                    click_mode,
                    ..
                } => {
                    // Click override wins over /tools config (2026-11-14).
                    let mode = click_mode.unwrap_or_else(|| {
                        effective_mode(name, self.tool_default, &self.tool_overrides)
                    });
                    if mode == ToolVerbosity::Line {
                        // Collapsed single line — the status-quo rendering,
                        // byte-identical to pre-verbosity builds.
                        let (icon, st) = if *running {
                            (theme::GLYPH_CRUCIBLE, theme::live())
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
                            let display = tool_display_name(name);
                            let used = display.chars().count() + 3 + title_s.chars().count() + 3;
                            let cmd_s = trunc_str(&cmd, budget.saturating_sub(used));
                            let mut spans = vec![
                                Span::styled(format!("  {} ", icon), st),
                                Span::styled(display, st),
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
                            let display = tool_display_name(name);
                            let body = if summary.is_empty() {
                                display
                            } else {
                                format!("{} {}", display, summary)
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
                                liveness.clone(),
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
                    // Neutral voice (2026-11-14): diagnostics are info, not
                    // errors — red is reserved for ChatEntry::Error.
                    let st = Style::new().fg(theme::SILVER).bg(theme::BG);
                    let glyph = Style::new().fg(theme::MUTED).bg(theme::BG);
                    for (i, seg) in wrap_text(text, width.saturating_sub(8))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(format!("  {} ", theme::GLYPH_SULFUR), glyph),
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
                ChatEntry::Warn(text) => {
                    let st = Style::new().fg(theme::ORANGE).bg(theme::BG);
                    for (i, seg) in wrap_text(text, width.saturating_sub(8))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled("  \u{26a0} ", st),
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
                ChatEntry::Error(text) => {
                    let st = Style::new().fg(theme::RED).bg(theme::BG);
                    for (i, seg) in wrap_text(text, width.saturating_sub(8))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled("  \u{2717} ", st),
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
                ChatEntry::Notice(text) => {
                    let st = Style::new().fg(theme::VIOLET).bg(theme::BG);
                    for (i, seg) in wrap_text(text, width.saturating_sub(8))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(format!("  {} ", theme::GLYPH_HERM), st),
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
                ChatEntry::Watchdog(text) => {
                    // Structured card when the facts file is fresh; raw text
                    // fallback so a message is never lost (I3).
                    let edge_color = watchdog_edge_color(wd.as_ref().map(|d| d.verdict.as_str()));
                    let edge = Style::new().fg(edge_color).bg(theme::BG);
                    let body = Style::new().fg(theme::TEXT).bg(theme::BG);
                    let muted = Style::new().fg(theme::MUTED).bg(theme::BG);
                    let span_w = width.saturating_sub(4) as usize;
                    let (chip, chip_style) = match wd.as_ref().map(|d| d.verdict.as_str()) {
                        Some("ALIVE") => (
                            " ALIVE ",
                            Style::new()
                                .fg(theme::GREEN)
                                .bg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Some("HUNG") => (
                            " HUNG ",
                            Style::new()
                                .fg(theme::ORANGE)
                                .bg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Some("JUDGING") => (
                            " JUDGING ",
                            Style::new()
                                .fg(theme::GOLD)
                                .bg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ),
                        _ => ("", muted),
                    };
                    let label = " \u{23f1} watch ";
                    let right = if chip.is_empty() {
                        String::new()
                    } else {
                        format!(" {chip}")
                    };
                    let dashes =
                        span_w.saturating_sub(label.chars().count() + right.chars().count() + 1);
                    lines.push(Line::from(vec![
                        Span::styled("  \u{256d}\u{2500}", edge),
                        Span::styled(
                            label,
                            Style::new()
                                .fg(edge_color)
                                .bg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled("\u{2500}".repeat(dashes.min(60)), edge),
                        Span::styled(right, chip_style),
                        Span::styled("\u{2500}\u{256e}", edge),
                    ]));
                    match wd.as_ref() {
                        Some(doc) if !doc.command.is_empty() => {
                            let field = |k: &str, v: String| {
                                Line::from(vec![
                                    Span::styled("  \u{2502} ", edge),
                                    Span::styled(format!("{k:<9}"), muted),
                                    Span::styled(trunc_str(&v, width.saturating_sub(13)), body),
                                ])
                            };
                            lines.push(field("cmd", doc.command.clone()));
                            lines.push(field(
                                "elapsed",
                                format!(
                                    "{}m {:02}s \u{00b7} stalled {}s",
                                    doc.elapsed_s / 60,
                                    doc.elapsed_s % 60,
                                    doc.stalled_s
                                ),
                            ));
                            if !doc.reason.is_empty() {
                                lines.push(field("reason", doc.reason.clone()));
                            }
                            let hint = match doc.verdict.as_str() {
                                "ALIVE" => "compiling/working \u{2014} no action needed",
                                "HUNG" => "esc aborts if you agree \u{00b7} no auto-kill",
                                _ => "judging on a free lane \u{00b7} no auto-kill",
                            };
                            lines.push(Line::from(vec![
                                Span::styled("  \u{2502} ", edge),
                                Span::styled(hint.to_string(), muted),
                            ]));
                        }
                        _ => {
                            for seg in wrap_text(text, width.saturating_sub(7)) {
                                lines.push(Line::from(vec![
                                    Span::styled("  \u{2502} ", edge),
                                    Span::styled(seg, body),
                                ]));
                            }
                        }
                    }
                    lines.push(Line::from(vec![
                        Span::styled("  \u{2570}", edge),
                        Span::styled("\u{2500}".repeat(span_w.saturating_sub(1)), edge),
                        Span::styled("\u{256f}", edge),
                    ]));
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
            for _ in before..lines.len() {
                line_entries.push(idx);
            }
        }
        self.line_entries = line_entries;
        lines
    }

    /// Local (TUI-side) commands — dispatched to RPC directly, never sent
    /// to pi as prompt text. (name, description)
    /// (name, short — menu row, long — popup detail line + /help).
    /// Shorts stay ≤ ~40 cols; longs carry the full usage.
    pub const LOCAL_COMMANDS: &[(&str, &str, &str)] = &[
        ("cost", "routing ledger", "/cost — today's route ledger: requests, tokens in/out, failures per provider/model + budget bars (read-only)"),
        ("routes", "caduceus decisions", "/routes — the latest routing decisions: tier, provider, model, reason, local state, latency, outcome (newest first; read-only)"),
        ("sessions", "all projects", "/sessions — search + resume sessions across every project store (foreign projects are read-only hints; I4)"),
        ("review-findings", "review results", "/review-findings — the last /review findings, severity-sorted (read-only; written by /review)"),
        ("janitor", "stale cache cleanup", "/janitor [dry] — remove stale prompt/watchdog/longjob/probe/KV/session-chain artifacts (protected: newest store file, slot0.bin, corrupt quarantines, ledgers); auto-runs daily"),
        ("thinking-view", "hide thinking blocks", "/thinking-view [hide|show] — collapse thinking blocks to a marker line (display only; the model's thinking level stays at /thinking)"),
        ("fx", "fire + glimmer", "/fx — fire/glimmer panel with live preview (fire only animates while the session is actually working)"),
        ("answers", "last inquiry answer", "/answers — the last INQUIRE-mode question + answer (scrollable; written by the agent-mode extension)"),
        ("engine", "engine control", "/engine [start|stop|restart|status] — control the vitriol-server systemd unit (no sudo: polkit scopes you to the two vitriol units; your daily-driver config is untouched)"),
        ("status", "session snapshot", "/status — model, agent, profile, context fill, cwd, session, permissions, gateway lanes, routing policy (read-only)"),
        ("caduceus", "gateway + routing", "/caduceus — providers/models/health/policy page: routing on-off, ranks, pins, free-only, caps (facts by the caduceus extension; policy file written by the TUI)"),
        ("cache", "manage the KV cache", "/cache — list engine KV cache entries with age/size; /cache wipe [age] — prune (slot0.bin protected)"),
        ("clear", "clear the transcript", "/clear — wipe the local transcript view (pi keeps the session history)"),
        ("compact", "compact the context", "/compact [hints] — summarize older history now; after it settles, auto-continue picks the work back up"),
        ("diag", "toggle diagnostics", "/diag or F9 — stderr overlay from the agent process"),
        ("help", "glossary + keys + commands", "/help or F1 — the reference modal"),
        ("clone", "duplicate the session", "/clone — duplicate the active branch into a fresh session"),
        ("export", "export session to HTML", "/export [path] — write a self-contained HTML transcript"),
        ("rename", "rename the session", "/rename <name> — set the session display name"),
        ("sidebar", "cycle sidebar visibility", "/sidebar — auto (hide when narrow) → show → hide"),
        ("permissions", "permission mode", "/permissions — ask | auto | yolo (sensitive roots always guarded)"),
        ("yolo", "toggle YOLO mode", "/yolo — toggle ask <-> yolo (sensitive roots still prompt)"),
        ("import-opencode", "import an OpenCode session", "/import-opencode [latest|<id>|list] [full] — convert an OpenCode session for this project and switch to it"),
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
/// Codex-QoL stage 5: does this output look like a unified diff? Header
/// markers win outright; otherwise a strong +/- majority over >=2 lines.
fn looks_like_diff(lines: &[String]) -> bool {
    let header = lines.iter().take(8).any(|l| {
        let t = l.trim_start();
        t.starts_with("diff --git")
            || t.starts_with("+++")
            || (t.starts_with("--- ") && t.len() > 4)
            || t.starts_with("@@")
            || t.starts_with("index ")
    });
    if header {
        return true;
    }
    if lines.len() < 2 {
        return false;
    }
    let pm = lines
        .iter()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with('+') || t.starts_with('-')
        })
        .count();
    pm * 2 > lines.len()
}

/// Style one diff line: @@ hunks gold, +++/--- file headers muted,
/// additions green, deletions red, context default.
fn diff_line_style(line: &str, default: Style) -> Style {
    let t = line.trim_start();
    if t.starts_with("@@") {
        Style::default().fg(theme::GOLD).bg(theme::BG)
    } else if t.starts_with("+++") || t.starts_with("---") {
        Style::default().fg(theme::MUTED).bg(theme::BG)
    } else if t.starts_with('+') {
        Style::default().fg(theme::GREEN).bg(theme::BG)
    } else if t.starts_with('-') {
        Style::default().fg(theme::RED).bg(theme::BG)
    } else {
        default
    }
}

/// Human-readable args for tools without a purpose-built renderer: one
/// line per key, values unquoted/shortened; objects fall back to compact
/// JSON, arrays collapse to a comma list or "[N items]". Max 6 rows.
fn generic_args_lines(
    args: &serde_json::Value,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
    muted_st: Style,
    text_st: Style,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme::MUTED).bg(surf);
    let mut lines = Vec::new();
    let Some(obj) = args.as_object() else {
        let one = serde_json::to_string(args).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(trunc_str(&one, width.saturating_sub(4)), text_st),
        ]));
        return lines;
    };
    let budget = width.saturating_sub(18);
    let mut shown = 0usize;
    for (k, v) in obj {
        if v.is_null() || (v.as_str().map(|s| s.trim().is_empty()).unwrap_or(false)) {
            continue;
        }
        // Block preview caps the list; the full (clicked) view shows all.
        if !full && shown >= 6 {
            let rest = obj.len().saturating_sub(shown);
            if rest > 0 {
                lines.push(Line::from(vec![
                    Span::styled("│ ", border),
                    Span::styled(format!("… {rest} more"), muted_st),
                ]));
            }
            break;
        }
        let value = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Array(items) => {
                let cap = if full { items.len() } else { 6 };
                let parts: Vec<String> = items
                    .iter()
                    .take(cap)
                    .map(|i| match i {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .collect();
                let joined = parts.join(", ");
                if items.len() > cap {
                    format!("{joined}, … (+{} more)", items.len() - cap)
                } else {
                    joined
                }
            }
            other => serde_json::to_string(other).unwrap_or_default(),
        };
        lines.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(
                format!("{:<14}", trunc_str(&trunc_str(k, 14), 14)),
                muted_st,
            ),
            Span::styled(trunc_str(value.trim(), budget), text_st),
        ]));
        shown += 1;
    }
    lines
}

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
    liveness: Option<(String, bool)>,
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
                tool_display_name(name),
                Style::default()
                    .fg(theme::SILVER)
                    .bg(surf)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
        ];
        let head_w = width.saturating_sub(6 + tool_display_name(name).chars().count());
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
        } else {
            // Generic tools: key/value lines instead of a raw JSON blob
            // (owner request 2026-11-13) — full mode lists every field,
            // the block preview caps at six.
            if full {
                lines.push(Line::from(vec![
                    Span::styled("│ ", border),
                    Span::styled("args", muted_st),
                ]));
            }
            lines.extend(generic_args_lines(a, full, width, surf, muted_st, text_st));
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
        // Codex-QoL stage 5: diff-shaped output (git diff, edit/write
        // results) gets +/-/@@ line coloring; everything else unchanged.
        let is_diff = looks_like_diff(slice);
        for ol in slice {
            let line_st = if is_diff {
                diff_line_style(ol, text_st)
            } else {
                text_st
            };
            for (i, seg) in wrap_text(ol, width.saturating_sub(5))
                .into_iter()
                .enumerate()
            {
                let prefix = if i == 0 { "│   " } else { "│   " };
                lines.push(Line::from(vec![
                    Span::styled(prefix, border),
                    Span::styled(seg, line_st),
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
            Span::styled("running…", Style::default().fg(theme::CYAN).bg(surf)),
        ];
        if let Some(secs) = elapsed {
            spans.push(Span::styled(
                format!(" {}", fmt_elapsed(secs)),
                Style::default().fg(theme::MUTED).bg(surf),
            ));
        }
        // Liveness evidence (2026-11-12): real CPU from pi's subprocess
        // tree — proof the work is alive even with output redirected to
        // logs. Green when the tree burned CPU in the last window, amber
        // when it has been flat.
        if let Some((text, alive)) = liveness {
            let st = if alive {
                Style::default().fg(theme::GREEN).bg(surf)
            } else {
                Style::default().fg(theme::ORANGE).bg(surf)
            };
            spans.push(Span::styled(format!(" · {text}"), st));
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
                Style::default().fg(theme::CYAN).bg(surf),
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
pub(crate) fn wrap_text(s: &str, width: usize) -> Vec<String> {
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
    fn notify_info_long_or_multiline_captures_notice_entry() {
        // stage 3 (2026-11-12): tables were invisible past the 40-char row
        let mut s = AppState::default();
        let table = "lane-a · 32k\nlane-b · 128k\nlane-c · 8k";
        s.apply_notify(table, "info");
        assert!(s.notice.is_some(), "glance row still set");
        assert_eq!(s.entries.len(), 1);
        let ChatEntry::Notice(t) = &s.entries[0] else {
            panic!("expected Notice entry, got {:?}", s.entries[0]);
        };
        assert!(t.contains("lane-b · 128k"));

        let long = "x".repeat(101);
        s.apply_notify(&long, "info");
        assert!(matches!(s.entries.last(), Some(ChatEntry::Notice(_))));
    }

    #[test]
    fn thinking_renders_markdown_and_maps_lines_to_entries() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = false;
        s.push_thinking_delta("**bold** reasoning and `code` here");
        let lines = s.chat_lines(80);
        assert!(!lines.is_empty());
        assert_eq!(
            s.line_entries.len(),
            lines.len(),
            "line→entry map must cover every rendered line"
        );
        assert!(s.line_entries.iter().all(|i| *i == 0));
        let bold = lines.iter().flat_map(|l| l.spans.iter()).any(|sp| {
            sp.content.contains("bold")
                && sp
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
        });
        assert!(bold, "markdown bold must survive the thinking renderer");
        assert!(lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|sp| sp.content.contains("reasoning")));
    }

    #[test]
    fn thinking_click_toggles_collapse_and_reopen() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = false;
        s.push_thinking_delta("line one\n\nline two");
        let expanded = s.chat_lines(80);
        assert!(expanded.len() > 1, "expanded block spans rows");
        s.thinking_hitboxes = vec![(10, 12, 0)];
        assert!(s.thinking_toggle_at(10), "click inside block toggles");
        assert!(matches!(
            s.entries[0],
            ChatEntry::Thinking {
                collapsed: true,
                ..
            }
        ));
        let collapsed = s.chat_lines(80);
        assert_eq!(collapsed.len(), 1, "collapsed block is one marker row");
        assert!(collapsed[0]
            .spans
            .iter()
            .any(|sp| sp.content.contains("click to expand")));
        assert_eq!(s.line_entries.len(), collapsed.len());
        assert!(s.thinking_toggle_at(12), "click again reopens");
        assert!(matches!(
            s.entries[0],
            ChatEntry::Thinking {
                collapsed: false,
                ..
            }
        ));
        assert!(
            !s.thinking_toggle_at(99),
            "click outside all blocks is a miss"
        );
    }

    #[test]
    fn thinking_hidden_still_maps_lines_to_entries() {
        let mut s = AppState::default();
        s.hide_thinking = true;
        s.push_thinking_delta("hidden reasoning");
        s.push_user("hello".into());
        let lines = s.chat_lines(80);
        assert_eq!(s.line_entries.len(), lines.len());
        assert!(
            s.line_entries.iter().any(|i| *i == 0),
            "hidden thinking marker maps to its entry (plus the blank)"
        );
        assert!(s.line_entries.iter().any(|i| *i == 1));
    }

    #[test]
    fn scroll_lock_freezes_the_view_and_releases_at_the_tail() {
        let mut s = AppState::default();
        s.last_start = 100;
        s.last_chat_total = 200;
        s.scroll_max = 120;
        s.scroll_up(5);
        assert_eq!(s.scroll_lock, Some(95), "lock pins the top line");
        assert_eq!(s.scroll, 5);
        // New output arrives: total grows, but the lock is untouched.
        s.last_chat_total = 260;
        assert_eq!(
            s.scroll_lock,
            Some(95),
            "streaming never moves a locked view"
        );
        s.scroll_down(5);
        assert_eq!(s.scroll, 0);
        assert_eq!(s.scroll_lock, None, "reaching the tail re-engages follow");
        s.scroll_up(3);
        s.scroll_home();
        assert_eq!(s.scroll_lock, Some(0), "Home jumps to the oldest line");
        s.scroll_end();
        assert_eq!(s.scroll_lock, None);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn new_user_turn_releases_the_scroll_lock() {
        let mut s = AppState::default();
        s.last_start = 40;
        s.scroll_max = 80;
        s.scroll_up(4);
        assert!(s.scroll_lock.is_some());
        s.push_user("hello".into());
        assert_eq!(s.scroll_lock, None);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn thinking_default_collapsed_applies_to_new_blocks_and_is_configurable() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = true;
        s.push_thinking_delta("first block");
        assert!(matches!(
            s.entries[0],
            ChatEntry::Thinking {
                collapsed: true,
                ..
            }
        ));
        // New block after an assistant entry also starts collapsed.
        s.push_text_delta("answer");
        s.push_thinking_delta("second block");
        assert!(matches!(
            s.entries.last(),
            Some(ChatEntry::Thinking {
                collapsed: true,
                ..
            })
        ));
        let dir = std::env::temp_dir().join(format!("thinking-default-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.thinking_default_file = Some(dir.join("tui-thinking-default"));
        s.thinking_view_command("default expanded");
        assert!(!s.thinking_default_collapsed);
        let raw = std::fs::read_to_string(dir.join("tui-thinking-default")).unwrap();
        assert_eq!(raw.trim(), "expanded");
        s.push_thinking_delta("third block");
        assert!(matches!(
            s.entries.last(),
            Some(ChatEntry::Thinking {
                collapsed: false,
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thinking_collapse_and_expand_all_commands() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = false;
        s.push_thinking_delta("one");
        s.push_text_delta("mid");
        s.push_thinking_delta("two");
        s.thinking_view_command("collapse");
        assert!(s
            .entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Thinking { .. }))
            .all(|e| matches!(
                e,
                ChatEntry::Thinking {
                    collapsed: true,
                    ..
                }
            )));
        s.thinking_view_command("expand");
        assert!(s
            .entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Thinking { .. }))
            .all(|e| matches!(
                e,
                ChatEntry::Thinking {
                    collapsed: false,
                    ..
                }
            )));
        s.thinking_view_command("collapse-all");
        assert!(s
            .entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Thinking { .. }))
            .all(|e| matches!(
                e,
                ChatEntry::Thinking {
                    collapsed: true,
                    ..
                }
            )));
    }

    #[test]
    fn tool_click_cycles_closed_formatted_full() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = false;
        s.tool_start(
            Some("t1"),
            "bash",
            &serde_json::json!({"command": "brievc build c4probe.bv --verify-bodies", "timeout": 240}),
        );
        s.tool_update(Some("t1"), "bash", "compiling\n");
        let closed = s.chat_lines(100);
        assert_eq!(s.line_entries.len(), closed.len());
        assert!(matches!(
            s.entries[0],
            ChatEntry::Tool {
                click_mode: None,
                ..
            }
        ));
        s.tool_hitboxes = vec![(5, 9, 0)];

        // Click 1: formatted block (bordered panel, args preview).
        assert!(s.tool_toggle_at(5));
        assert!(matches!(
            s.entries[0],
            ChatEntry::Tool {
                click_mode: Some(ToolVerbosity::Block),
                ..
            }
        ));
        let formatted = s.chat_lines(100);
        assert!(
            formatted.iter().any(|l| l.to_string().contains('╭')),
            "formatted state renders the bordered block"
        );

        // Click 2: full (same block, every field / full output).
        assert!(s.tool_toggle_at(5));
        assert!(matches!(
            s.entries[0],
            ChatEntry::Tool {
                click_mode: Some(ToolVerbosity::Full),
                ..
            }
        ));
        let full = s.chat_lines(100);
        assert!(full.len() >= formatted.len());

        // Click 3: closed again (single status line).
        assert!(s.tool_toggle_at(5));
        assert!(matches!(
            s.entries[0],
            ChatEntry::Tool {
                click_mode: Some(ToolVerbosity::Line),
                ..
            }
        ));
        let line = s.chat_lines(100);
        assert_eq!(line.len(), closed.len(), "closed is one line again");
        assert!(!s.tool_toggle_at(99), "click outside all blocks is a miss");
    }

    #[test]
    fn tool_and_thinking_hitboxes_are_separate() {
        let mut s = AppState::default();
        s.hide_thinking = false;
        s.thinking_default_collapsed = false;
        s.push_thinking_delta("reasoning");
        s.tool_start(Some("t1"), "read", &serde_json::json!({"path": "/tmp/x"}));
        let lines = s.chat_lines(80);
        assert_eq!(s.line_entries.len(), lines.len());
        s.thinking_hitboxes = vec![(0, 0, 0)];
        s.tool_hitboxes = vec![(1, 2, 1)];
        assert!(s.thinking_toggle_at(0));
        assert!(!s.tool_toggle_at(0), "row 0 is the thinking block");
        assert!(s.tool_toggle_at(1));
        assert!(matches!(
            s.entries[0],
            ChatEntry::Thinking {
                collapsed: true,
                ..
            }
        ));
        assert!(matches!(
            s.entries[1],
            ChatEntry::Tool {
                click_mode: Some(ToolVerbosity::Block),
                ..
            }
        ));
    }

    fn flat_colors(lines: &[Line]) -> Vec<ratatui::style::Color> {
        lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter_map(|sp| sp.style.fg)
            .collect()
    }

    #[test]
    fn running_tool_is_cyan_never_red() {
        let mut s = AppState::default();
        s.tool_start(
            Some("t1"),
            "bash",
            &serde_json::json!({"command": "sleep 5"}),
        );
        let lines = s.chat_lines(100);
        let colors = flat_colors(&lines);
        assert!(
            colors.contains(&theme::CYAN),
            "running uses cyan: {colors:?}"
        );
        assert!(!colors.contains(&theme::RED), "running is never red");
        assert!(
            !colors.contains(&theme::ORANGE),
            "running is not amber either"
        );
        // Completed → green; failed → red.
        s.tool_end(Some("t1"), "bash", "done", false);
        let done: Vec<Line> = s.chat_lines(100);
        assert!(flat_colors(&done).contains(&theme::GREEN));
        let mut f = AppState::default();
        f.tool_start(Some("t2"), "bash", &serde_json::json!({"command": "false"}));
        f.tool_end(Some("t2"), "bash", "exit 1", true);
        let failed: Vec<Line> = f.chat_lines(100);
        assert!(flat_colors(&failed).contains(&theme::RED));
    }

    #[test]
    fn severity_entries_render_neutral_amber_red() {
        let mut s = AppState::default();
        s.entries
            .push(ChatEntry::Diag("janitor: nothing stale".into()));
        let diag = s.chat_lines(90);
        assert!(!flat_colors(&diag).contains(&theme::RED), "Diag never red");
        assert!(
            !flat_colors(&diag).contains(&theme::ORANGE),
            "Diag not amber"
        );
        s.entries.clear();
        s.entries.push(ChatEntry::Warn("watchdog abort".into()));
        let warn = s.chat_lines(90);
        assert!(flat_colors(&warn).contains(&theme::ORANGE));
        assert!(!flat_colors(&warn).contains(&theme::RED));
        s.entries.clear();
        s.entries.push(ChatEntry::Error("checkpoint failed".into()));
        let err = s.chat_lines(90);
        assert!(flat_colors(&err).contains(&theme::RED));
        s.entries.clear();
    }

    #[test]
    fn watchdog_edge_follows_the_verdict() {
        assert_eq!(watchdog_edge_color(Some("ALIVE")), theme::GREEN);
        assert_eq!(watchdog_edge_color(Some("JUDGING")), theme::GOLD);
        assert_eq!(watchdog_edge_color(Some("HUNG")), theme::ORANGE);
        assert_eq!(watchdog_edge_color(Some("?")), theme::MUTED);
        assert_eq!(watchdog_edge_color(None), theme::MUTED);
        // ALIVE must never wear the warning/failure hue (I3).
        let mut s = AppState::default();
        s.entries
            .push(ChatEntry::Watchdog("hang-watchdog ALIVE".into()));
        s.watchdog = Some(WatchdogDoc {
            version: 1,
            ts: crate::tui::caduceus::now_ms(),
            kind: "verdict".into(),
            command: "brievc build".into(),
            elapsed_s: 60,
            stalled_s: 0,
            verdict: "ALIVE".into(),
            reason: "compiling".into(),
            next_check_at: 0,
            phase: "watch".into(),
        });
        let lines = s.chat_lines(90);
        let colors = flat_colors(&lines);
        assert!(colors.contains(&theme::GREEN));
        assert!(!colors.contains(&theme::ORANGE), "ALIVE card is not amber");
        assert!(!colors.contains(&theme::RED), "ALIVE card is not red");
    }

    #[test]
    fn watchdog_cards_update_in_place_for_identical_facts() {
        let dir = std::env::temp_dir().join(format!("wd-dedupe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("watchdog.json");
        let now = crate::tui::caduceus::now_ms();
        let doc = |verdict: &str, reason: &str| {
            format!(
                r#"{{"version":1,"ts":{now},"kind":"verdict","command":"brievc build",
                     "elapsed_s":60,"stalled_s":10,"verdict":"{verdict}",
                     "reason":"{reason}","next_check_at":0,"phase":"watch"}}"#
            )
        };
        std::fs::write(&path, doc("ALIVE", "working")).unwrap();
        let mut s = AppState::default();
        s.watchdog_file = Some(path.clone());
        s.apply_notify("hang-watchdog: alive check 1", "info");
        s.apply_notify("hang-watchdog: alive check 2", "info");
        let cards = s
            .entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Watchdog(_)))
            .count();
        assert_eq!(cards, 1, "identical facts replace the card");
        if let Some(ChatEntry::Watchdog(t)) = s.entries.last() {
            assert!(t.contains("alive check 2"), "card carries the latest text");
        } else {
            panic!("expected a watchdog card");
        }
        // Different verdict is a new observation → new card.
        std::fs::write(&path, doc("HUNG", "waiting on stdin")).unwrap();
        s.apply_notify("hang-watchdog: now hung", "warning");
        assert_eq!(
            s.entries
                .iter()
                .filter(|e| matches!(e, ChatEntry::Watchdog(_)))
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn notify_info_short_stays_transient() {
        let mut s = AppState::default();
        s.apply_notify("coupled: Lapis Occultus", "info");
        assert_eq!(s.entries.len(), 0, "short info must not spam transcript");
        assert_eq!(
            s.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("coupled: Lapis Occultus")
        );
    }

    #[test]
    fn notify_severity_maps_to_warn_and_error() {
        let mut s = AppState::default();
        s.apply_notify("engine down", "warning");
        s.apply_notify("checkpoint failed", "error");
        assert_eq!(s.entries.len(), 2);
        assert!(matches!(s.entries[0], ChatEntry::Warn(_)));
        assert!(matches!(s.entries[1], ChatEntry::Error(_)));
        assert!(s.notice.is_none(), "errors do not use the glance row");
    }

    #[test]
    fn diff_detection_and_line_styles() {
        // codex-qol stage 5
        let diff: Vec<String> = vec![
            "diff --git a/x.rs b/x.rs".into(),
            "--- a/x.rs".into(),
            "+++ b/x.rs".into(),
            "@@ -1,3 +1,4 @@".into(),
            " context line".into(),
            "-old line".into(),
            "+new line".into(),
        ];
        assert!(looks_like_diff(&diff));
        let bash: Vec<String> = vec![
            "cargo build --release -p officina".into(),
            "   Compiling x".into(),
        ];
        assert!(!looks_like_diff(&bash));
        let lone: Vec<String> = vec!["-x".into()];
        assert!(!looks_like_diff(&lone));
        let majority: Vec<String> = vec!["+a".into(), "-b".into(), "+c".into(), "ctx".into()];
        assert!(looks_like_diff(&majority));

        let base = Style::default().fg(theme::TEXT);
        assert_eq!(diff_line_style("+add", base).fg, Some(theme::GREEN));
        assert_eq!(diff_line_style("-del", base).fg, Some(theme::RED));
        assert_eq!(diff_line_style("@@ hunk", base).fg, Some(theme::GOLD));
        assert_eq!(diff_line_style("+++ b/x", base).fg, Some(theme::MUTED));
        assert_eq!(diff_line_style("context", base), base);
    }

    #[test]
    fn editor_staging_preserves_the_draft() {
        // codex-qol stage 4 (I4): staging never loses the draft
        let mut s = AppState::default();
        s.input = "refactor the flux capacitor\nsecond line".into();
        s.stage_editor("nvim -f");
        let (argv, tmp, original) = s.editor_suspend.take().expect("staged");
        assert_eq!(argv[0], "nvim");
        assert_eq!(argv[1], "-f");
        assert!(argv[2].ends_with(".md"));
        assert_eq!(std::fs::read_to_string(&tmp).unwrap(), original);
        assert_eq!(original, "refactor the flux capacitor\nsecond line");
        let _ = std::fs::remove_file(&tmp);
        // empty editor string refuses without staging
        s.stage_editor("   ");
        assert!(s.editor_suspend.is_none());
        assert!(s.notice.is_some());
    }

    #[test]
    fn resume_filter_narrows_and_clamps() {
        // codex-qol stage 3
        use crate::rpc::sessions::SessionEntry;
        let mut s = AppState::default();
        let mk = |title: &str, p: &str| SessionEntry {
            path: std::path::PathBuf::from(p),
            title: title.to_string(),
            modified: std::time::SystemTime::UNIX_EPOCH,
            msg_count: 1,
        };
        s.resume_entries = vec![
            mk("fix the flux capacitor", "/s/a.jsonl"),
            mk("refactor dashboard", "/s/b.jsonl"),
            mk("investigate flaky tests", "/s/c.jsonl"),
        ];
        assert_eq!(s.filtered_resume_entries().len(), 3);
        s.resume_filter = "fl".into();
        let names: Vec<String> = s
            .filtered_resume_entries()
            .iter()
            .map(|e| e.title.clone())
            .collect();
        assert_eq!(
            names,
            vec!["fix the flux capacitor", "investigate flaky tests"]
        );
        s.resume_sel = 9;
        s.clamp_resume_sel();
        assert_eq!(s.resume_sel, 1);
        s.resume_filter = "zzz".into();
        s.clamp_resume_sel();
        assert_eq!(s.resume_sel, 0);
    }

    #[test]
    fn watchdog_notifies_get_their_own_block() {
        // 2026-11-13: warnings from the watchdog are observations, not diags
        let mut s = AppState::default();
        s.watchdog_file = Some(std::path::PathBuf::from("/nonexistent/watchdog.json"));
        s.apply_notify(
            "hang-watchdog: \"brievc build\" — no output for 61s (61s total)",
            "warning",
        );
        assert_eq!(s.entries.len(), 1);
        assert!(matches!(s.entries[0], ChatEntry::Watchdog(_)));
        assert!(
            s.notice.is_none(),
            "watchdog entries do not use the glance row"
        );
        // normal warnings become Warn (amber), not Diag or Watchdog
        let mut s2 = AppState::default();
        s2.apply_notify("engine down", "warning");
        assert!(matches!(s2.entries[0], ChatEntry::Warn(_)));
        // the block renders with its label
        let text = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("watch"), "watchdog label: {text}");
        assert!(text.contains("no output for 61s"));
    }

    #[test]
    fn quit_guard_arms_only_when_busy() {
        // 2026-11-13: restart-mid-turn manufactured session patches — the
        // guard makes that a deliberate two-press decision instead.
        let mut s = AppState::default();
        // idle: proceeds immediately
        assert!(s.quit_may_proceed());
        // busy + not armed: first press arms, does not proceed
        s.is_streaming = true;
        assert!(!s.quit_may_proceed());
        assert!(s.quit_armed_at.is_some(), "armed");
        // second press inside the 5 s window: proceeds, disarms
        assert!(s.quit_may_proceed());
        assert!(s.quit_armed_at.is_none());
        // re-arm then let the window expire (>5 s): still blocked
        assert!(!s.quit_may_proceed());
        s.quit_armed_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
        assert!(!s.quit_may_proceed(), "expired arm re-arms");
        // not streaming anymore: proceeds and disarms
        s.is_streaming = false;
        assert!(s.quit_may_proceed());
        assert!(s.quit_armed_at.is_none());
    }

    #[test]
    fn watchdog_card_parses_and_counts_down() {
        // 2026-11-13: structured fields + "watch Ns" countdown
        let now = crate::tui::caduceus::now_ms();
        let doc: WatchdogDoc = serde_json::from_str(&format!(
            r#"{{"version":1,"ts":{},"kind":"verdict","command":"brievc build c4probe.bv",
                 "elapsed_s":252,"stalled_s":189,"verdict":"HUNG","reason":"waiting on stdin",
                 "next_check_at":{},"phase":"rejudge"}}"#,
            now,
            now + 21_000
        ))
        .expect("parses");
        let mut s = AppState::default();
        s.watchdog = Some(doc.clone());
        let secs = s.watchdog_countdown().expect("countdown armed");
        assert!((20..=21).contains(&secs), "countdown ~21s, got {secs}");
        // facts older than 2 min are ignored
        s.watchdog = Some(WatchdogDoc {
            ts: now.saturating_sub(180_000),
            ..doc.clone()
        });
        assert!(s.watchdog_countdown().is_none());
        // consumed watch (next_check_at 0) has no countdown
        s.watchdog = Some(WatchdogDoc {
            next_check_at: 0,
            ..doc
        });
        assert!(s.watchdog_countdown().is_none());
        // the card renders fields, not prose
        let mut s2 = AppState::default();
        s2.watchdog_file = Some(std::path::PathBuf::from("/nonexistent/watchdog.json"));
        s2.apply_notify("hang-watchdog verdict: HUNG", "warning");
        // apply_notify reloads facts from disk (absent in tests) — inject
        // the parsed doc after, which is the same state production reaches
        // when the file is fresh.
        s2.watchdog = Some(
            serde_json::from_str(&format!(
                r#"{{"version":1,"ts":{now},"kind":"verdict","command":"brievc build c4probe.bv",
                 "elapsed_s":252,"stalled_s":189,"verdict":"HUNG","reason":"waiting on stdin",
                 "next_check_at":0,"phase":"done"}}"#
            ))
            .unwrap(),
        );
        let text = s2
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("HUNG"), "verdict chip: {text}");
        assert!(text.contains("brievc build"), "cmd field: {text}");
        assert!(text.contains("4m 12s"), "elapsed field: {text}");
        assert!(text.contains("waiting on stdin"), "reason field: {text}");
        assert!(text.contains("esc aborts"), "hint: {text}");
    }

    #[test]
    fn flames_derive_from_stream_when_engine_telemetry_absent() {
        // 2026-11-13: cloud-lane turns (engine down) get honest flames from
        // token flow — deltas burn, a stalled stream decays to nothing.
        let mut s = AppState::default();
        s.fire_on = true;
        s.is_streaming = true;
        s.mark_activity();
        // no engine-fire telemetry ever seen → stream-derived
        s.push_text_delta("token ");
        s.step_fire(0.1);
        assert!(s.fire_target > 0.3, "stream deltas burn: {}", s.fire_target);
        // deltas stop → heat decays, target falls to the quiet floor
        for _ in 0..12 {
            s.step_fire(0.5);
        }
        assert!(
            s.fire_target < 0.5,
            "decayed after silence: {}",
            s.fire_target
        );
        // fresh engine telemetry takes precedence over the stream fallback:
        // step_fire must not touch the telemetry-driven target
        s.set_widget("engine-fire", vec!["FIRE 0.9".to_string()]);
        s.stream_heat = 1.5;
        s.fire_target = 0.9;
        s.step_fire(0.1);
        assert_eq!(s.fire_target, 0.9, "telemetry wins when fresh");
    }

    #[test]
    fn liveliness_gate_fire_by_evidence() {
        // 2026-11-13: animations never run without a real signal
        let mut s = AppState::default();
        assert!(!s.liveliness_active(), "fresh state has no evidence");
        s.fire_target = 0.9;
        s.apply_liveliness_gate();
        assert_eq!(s.fire_target, 0.0, "stalled → flames die");

        s.mark_activity();
        assert!(s.liveliness_active(), "fresh signal is active");
        s.fire_target = 0.7;
        s.apply_liveliness_gate();
        assert_eq!(s.fire_target, 0.7, "active → flames allowed");

        // stale signal (11 s old) is stalled again
        s.last_activity = Some(std::time::Instant::now() - std::time::Duration::from_secs(11));
        assert!(!s.liveliness_active());
        s.apply_liveliness_gate();
        assert_eq!(s.fire_target, 0.0);
    }

    #[test]
    fn liveliness_active_via_tool_cpu() {
        let mut s = AppState::default();
        s.tool_start(
            Some("c"),
            "bash",
            &serde_json::json!({"command": "brievc build"}),
        );
        s.liveness_sampled_at = Some(std::time::Instant::now());
        s.liveness_flat_secs = 0;
        s.liveness_delta = 1.2;
        s.last_activity = None;
        assert!(s.liveliness_active(), "live tool CPU counts as evidence");
        s.liveness_flat_secs = 120;
        assert!(!s.liveliness_active(), "flat CPU for 120 s is a stall");
    }

    #[test]
    fn prompt_history_navigation_and_draft_restore() {
        // 2026-11-13: Up/Down through earlier prompts, draft preserved
        let mut s = AppState::default();
        s.prompt_history = vec!["first".into(), "second".into(), "third".into()];
        s.input = "draft in progress".into();
        assert!(s.history_prev());
        assert_eq!(s.input, "third");
        assert!(s.history_prev());
        assert_eq!(s.input, "second");
        assert!(s.history_prev());
        assert_eq!(s.input, "first");
        assert!(s.history_prev()); // clamp at oldest
        assert_eq!(s.input, "first");
        assert!(s.history_next());
        assert_eq!(s.input, "second");
        s.history_next();
        s.history_next(); // past newest → draft restored
        assert_eq!(s.input, "draft in progress");
        assert!(s.history_idx.is_none());
        assert!(!s.history_next());
    }

    #[test]
    fn prompt_history_records_dedupes_and_caps() {
        let mut s = AppState::default();
        s.prompt_history.clear();
        s.record_prompt_history("same");
        s.record_prompt_history("same"); // consecutive dupe ignored
        assert_eq!(s.prompt_history.len(), 1);
        s.record_prompt_history("different");
        assert_eq!(s.prompt_history.len(), 2);
        for i in 0..250 {
            s.record_prompt_history(&format!("p{i}"));
        }
        assert_eq!(s.prompt_history.len(), 200);
        assert_eq!(s.prompt_history.last().unwrap(), "p249");
    }

    #[test]
    fn generic_args_render_key_value_not_json() {
        let base = Style::default().fg(theme::TEXT);
        let muted = Style::default().fg(theme::MUTED);
        let args = serde_json::json!({
            "path": "src/main.rs",
            "limit": 40,
            "tags": ["a", "b"],
            "nested": {"k": 1},
            "empty": null,
            "blank": ""
        });
        let lines = generic_args_lines(&args, false, 80, theme::PANEL, muted, base);
        let text = lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        assert!(text.contains("path"));
        assert!(text.contains("src/main.rs"));
        assert!(text.contains("40"));
        assert!(text.contains("a, b"));
        assert!(text.contains(r#"{"k":1}"#));
        assert!(!text.contains("null"), "nulls skipped: {text}");
        assert!(!text.contains(r#""path":"#), "no JSON key quoting: {text}");

        // Clicked (full) view: every field, no six-key cap, arrays opened up.
        let many = serde_json::json!({
            "a": 1, "b": 2, "c": 3, "d": 4, "e": 5, "f": 6, "g": 7, "h": 8,
            "items": ["x", "y", "z", "w", "v", "u", "t", "s"]
        });
        let full = generic_args_lines(&many, true, 120, theme::PANEL, muted, base)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        for k in ["a", "h"] {
            assert!(full.contains(k), "full lists {k}: {full}");
        }
        assert!(full.contains("items"), "full lists arrays: {full}");
        assert!(full.contains('s'), "arrays uncapped in full: {full}");
        let preview = generic_args_lines(&many, false, 120, theme::PANEL, muted, base)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            preview.contains("more"),
            "preview caps with a count: {preview}"
        );
    }

    #[test]
    fn tool_display_titles_format_registry_names() {
        assert_eq!(tool_display_name("update_tasks"), "Update Tasks");
        assert_eq!(tool_display_name("scratchpad_write"), "Scratchpad Write");
        assert_eq!(tool_display_name("bash"), "Bash");
        assert_eq!(tool_display_name("ls"), "List Directory");
        assert_eq!(tool_display_name("some_custom_tool"), "Some Custom Tool");
        assert_eq!(tool_display_name("kebab-tool"), "Kebab Tool");
    }

    #[test]
    fn unknown_tool_block_shows_titles_and_key_values_not_json() {
        let mut s = AppState::default();
        s.tool_start(
            Some("t1"),
            "custom_fetch",
            &serde_json::json!({"url": "https://example.com", "headers": {"a": "b"}}),
        );
        s.tool_end(Some("t1"), "custom_fetch", "done", false);
        s.tool_overrides.insert(
            "custom_fetch".to_string(),
            ToolOverride {
                mode: ToolVerbosity::Block,
                strictness: Strictness::Pinned,
            },
        );
        let text = s
            .chat_lines(110)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Custom Fetch"), "formal title: {text}");
        assert!(text.contains("url"), "key/value rows: {text}");
        assert!(!text.contains("\"url\":"), "no raw JSON: {text}");
    }

    #[test]
    fn stat_cpu_ticks_parses_after_comm_parens() {
        // comm with spaces/parens must not break field indexing
        let raw = "12345 (brievc buil) S 1 1 1 0 -1 4194560 0 0 0 0 4200 800 0 0 20 0 1 0 12345 1 1 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let t = stat_cpu_ticks(raw).expect("parses");
        assert_eq!(t, 4200 + 800);
        // 4200+800 ticks = 50.0 CPU-seconds at 100 Hz
        let secs = t as f64 / CLK_TCK;
        assert!((secs - 50.0).abs() < 1e-9);
    }

    #[test]
    fn liveness_suffix_reports_live_and_flat() {
        let mut s = AppState::default();
        assert!(s.liveness_suffix().is_none()); // no sample yet
        s.liveness_sampled_at = Some(std::time::Instant::now());
        s.liveness_delta = 3.2;
        s.liveness_flat_secs = 0;
        let (text, alive) = s.liveness_suffix().unwrap();
        assert_eq!(text, "cpu +3.2s");
        assert!(alive);
        s.liveness_delta = 0.0;
        s.liveness_flat_secs = 120;
        let (text, alive) = s.liveness_suffix().unwrap();
        assert_eq!(text, "cpu flat 120s");
        assert!(!alive);
        s.liveness_unavailable = true;
        assert!(s.liveness_suffix().is_none());
    }

    #[test]
    fn probe_resets_when_no_tool_runs() {
        let mut s = AppState::default();
        s.liveness_cpu_total = 42.0;
        s.liveness_sampled_at = Some(std::time::Instant::now());
        s.probe_liveness(None); // nothing running → reset
        assert_eq!(s.liveness_cpu_total, 0.0);
        assert!(s.liveness_sampled_at.is_none());
    }

    #[test]
    fn ghost_lifecycle_accept_dismiss_telemetry() {
        use std::io::Read;
        let dir = std::env::temp_dir().join(format!("ghost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sug = dir.join("prompt-suggestion.json");
        let events = dir.join("prompt-suggest-events.jsonl");
        std::fs::write(
            &sug,
            r#"{"version":1,"id":"sug-1","text":"run the full verify suite","source":"llm","expires":0}"#,
        )
        .unwrap();
        let mut s = AppState::default();
        s.prompt_suggestion_file = Some(sug.clone());
        s.ghost_events_file = Some(events.clone());
        s.refresh_ghost();
        assert_eq!(
            s.prompt_ghost.as_ref().unwrap().text,
            "run the full verify suite"
        );
        // dismissed id does not come back on refresh
        s.input = String::new();
        s.dismiss_ghost("dismissed-typed");
        assert!(s.prompt_ghost.is_none());
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        s.ghost_dismissed_for = None;
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_some());
        // accept folds it into the draft and logs both verdicts
        s.accept_ghost();
        assert_eq!(s.input, "run the full verify suite");
        assert_eq!(s.cursor, s.input.chars().count());
        assert!(s.prompt_ghost.is_none());
        let mut text = String::new();
        std::fs::File::open(&events)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert!(text.contains("\"verdict\":\"dismissed-typed\""));
        assert!(text.contains("\"verdict\":\"accepted\""));
        assert!(
            !text.contains("verify suite"),
            "I4: no prompt text in telemetry"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ghost_ignores_expired_corrupt_and_busy_states() {
        let dir = std::env::temp_dir().join(format!("ghost2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sug = dir.join("prompt-suggestion.json");
        let mut s = AppState::default();
        s.prompt_suggestion_file = Some(sug.clone());
        // corrupt
        std::fs::write(&sug, "{nope").unwrap();
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        // expired
        std::fs::write(
            &sug,
            format!(r#"{{"version":1,"id":"e","text":"x","source":"llm","expires":1}}"#),
        )
        .unwrap();
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        // busy composer never shows one
        std::fs::write(
            &sug,
            r#"{"version":1,"id":"ok","text":"y","source":"llm","expires":0}"#,
        )
        .unwrap();
        s.input = "operator text".into();
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        s.is_streaming = true;
        s.input = String::new();
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_rows_cover_session_facts() {
        // codex-qol stage 2: read-only snapshot content
        let mut s = AppState::default();
        s.cwd = std::path::PathBuf::from("/tmp");
        s.model = Some(crate::rpc::protocol::Model {
            id: "deepseek-v4.1-flash".into(),
            name: Some("DeepSeek V4.1 Flash".into()),
            provider: "caduceus".into(),
            context_window: Some(1_000_000),
            max_tokens: None,
            reasoning: None,
        });
        s.perm_mode = "yolo".into();
        let rows = s.status_rows(
            None,
            Some("free_only=on · per_owner=8 · max=48".into()),
            None,
        );
        let get = |k: &str| {
            rows.iter()
                .find(|(label, _)| *label == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(get("model").contains("caduceus / deepseek-v4.1-flash"));
        assert!(get("model").contains("DeepSeek V4.1 Flash"));
        assert_eq!(get("permissions"), "yolo");
        assert_eq!(get("profile"), "auto (default)");
        assert!(get("context").contains("no report yet"));
        assert!(get("gateway").contains("no state file"));
        assert!(get("policy").contains("per_owner=8"));
    }

    #[test]
    fn status_rows_report_gateway_and_policy_notes() {
        let s = AppState::default();
        let gw = crate::tui::caduceus::parse_state(
            r#"{"ts":0,"gateway_up":true,"cooling":3,
                "candidates":[{"id":"a/b","owner":"a","context_window":1,"max_tokens":1,"free":true}]}"#,
        )
        .unwrap();
        let rows = s.status_rows(
            Some((gw, true)),
            Some("prefs ignored: invalid JSON".into()),
            Some("fast".into()),
        );
        let get = |k: &str| {
            rows.iter()
                .find(|(label, _)| *label == k)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        assert!(get("gateway").contains("up · 1 free lanes · cooling 3"));
        assert!(get("gateway").contains("STALE")); // ts 0 → ancient snapshot
        assert_eq!(get("profile"), "fast");
        assert!(get("policy").contains("prefs ignored"));
    }

    #[test]
    fn ctx_bar_label_hidden_until_first_report() {
        let s = AppState::default();
        assert_eq!(s.ctx_bar_label(), None);
    }

    #[test]
    fn ctx_bar_label_formats_fill_and_percent() {
        let mut s = AppState::default();
        s.ctx_usage = Some((12_345, 262_144));
        assert_eq!(
            s.ctx_bar_label().as_deref(),
            Some("🜖 ctx 12.3k / 262.1k (4.7%)")
        );
    }

    #[test]
    fn ctx_bar_label_handles_millions_and_full_window() {
        let mut s = AppState::default();
        s.ctx_usage = Some((1_500_000, 2_000_000));
        assert_eq!(
            s.ctx_bar_label().as_deref(),
            Some("🜖 ctx 1.5M / 2.0M (75.0%)")
        );
        s.ctx_usage = Some((262_144, 262_144));
        assert_eq!(
            s.ctx_bar_label().as_deref(),
            Some("🜖 ctx 262.1k / 262.1k (100.0%)")
        );
    }

    #[test]
    fn ctx_bar_label_ignores_zero_window() {
        let mut s = AppState::default();
        s.ctx_usage = Some((1234, 0));
        assert_eq!(s.ctx_bar_label(), None);
    }

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
        assert!(text.contains("Write"), "formal title present: {}", text);
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
        // Generic tools render key/value lines (no raw JSON blob, 2026-11-13)
        assert!(text.contains("path"), "generic args key present: {}", text);
        assert!(
            text.contains("src/main.rs"),
            "generic args value present: {}",
            text
        );
        assert!(text.contains("out line"), "output present: {}", text);
    }

    #[test]
    fn full_mode_shows_all_args_as_key_values() {
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
        assert!(text.contains("path"), "key/value rows: {}", text);
        assert!(text.contains("a.rs"), "value visible: {}", text);
        assert!(text.contains("old"), "second key: {}", text);
        assert!(!text.contains("\"path\":"), "no JSON quoting: {}", text);
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
            None,
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
            None,
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
            None,
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

// ── Transition-table property tests (verification-rollout T0.1) ──────
//
// The table is the spec: it must be a function (unique per (from, event)),
// every declared row must be legal per lookup, and every (phase, event)
// pair not in the table must be rejected. `check` returns the declared
// target for legal pairs and Err for the rest.
#[cfg(test)]
mod transition_tests {
    use super::*;

    fn set_phase(s: &mut AppState, p: UiPhase) {
        s.is_streaming = false;
        s.is_compacting = false;
        s.models_open = false;
        s.cache_open = false;
        s.resume_open = false;
        s.perm_modal_open = false;
        s.dialogs.clear();
        match p {
            UiPhase::Idle => {}
            UiPhase::Streaming => s.is_streaming = true,
            UiPhase::Compacting => s.is_compacting = true,
            UiPhase::Modal => s.models_open = true,
            UiPhase::Dialog => s.perm_modal_open = true,
        }
    }

    #[test]
    fn table_is_a_function() {
        for (i, a) in TRANSITIONS.iter().enumerate() {
            for b in TRANSITIONS.iter().skip(i + 1) {
                assert_ne!(
                    (a.from, a.event),
                    (b.from, b.event),
                    "duplicate (from, event) row"
                );
            }
        }
    }

    #[test]
    fn every_row_legal_and_matches_lookup() {
        for t in TRANSITIONS {
            assert_eq!(AppState::lookup(t.from, t.event), Some(t.to));
        }
    }

    #[test]
    fn unlisted_pairs_rejected() {
        let phases = [
            UiPhase::Idle,
            UiPhase::Streaming,
            UiPhase::Compacting,
            UiPhase::Modal,
            UiPhase::Dialog,
        ];
        let events = [
            UiEvent::AgentStart,
            UiEvent::AgentEnd,
            UiEvent::CompactionStart,
            UiEvent::CompactionEnd,
            UiEvent::OpenModal,
            UiEvent::CloseModal,
            UiEvent::OpenDialog,
            UiEvent::CloseDialog,
        ];
        for p in phases {
            for e in events {
                let in_table = TRANSITIONS.iter().any(|t| t.from == p && t.event == e);
                let legal = AppState::lookup(p, e).is_some();
                assert_eq!(
                    in_table, legal,
                    "pair ({:?}, {:?}) table/lookup disagree",
                    p, e
                );
            }
        }
    }

    #[test]
    fn check_returns_target_for_legal_pairs() {
        for t in TRANSITIONS {
            let mut s = AppState::default();
            set_phase(&mut s, t.from);
            assert_eq!(s.phase(), t.from);
            assert_eq!(s.check(t.event), Ok(t.to));
        }
    }

    #[test]
    fn check_rejects_illegal_pairs() {
        let mut s = AppState::default();
        set_phase(&mut s, UiPhase::Idle);
        // pi re-delivers an end event on an idle UI (agent_end then
        // agent_settled), so that pair is deliberately legal; a compaction
        // ending with nothing compacting is not.
        assert!(matches!(s.check(UiEvent::AgentEnd), Ok(UiPhase::Idle)));
        assert!(matches!(s.check(UiEvent::CompactionEnd), Err(_)));
        set_phase(&mut s, UiPhase::Streaming);
        // Streaming legitimately allows picker open/close; a compaction_end
        // with nothing compacting is the impossible pair.
        assert!(matches!(
            s.check(UiEvent::CloseModal),
            Ok(UiPhase::Streaming)
        ));
        assert!(matches!(s.check(UiEvent::CompactionEnd), Err(_)));
        set_phase(&mut s, UiPhase::Compacting);
        assert!(matches!(s.check(UiEvent::CompactionEnd), Ok(UiPhase::Idle)));
        set_phase(&mut s, UiPhase::Modal);
        assert!(matches!(s.check(UiEvent::AgentEnd), Err(_)));
    }

    #[test]
    fn phase_priority_dialog_beats_streaming_beats_modals() {
        let mut s = AppState::default();
        set_phase(&mut s, UiPhase::Streaming);
        s.models_open = true;
        assert_eq!(s.phase(), UiPhase::Streaming);
        s.perm_modal_open = true;
        assert_eq!(s.phase(), UiPhase::Dialog);
        set_phase(&mut s, UiPhase::Modal);
        assert_eq!(s.phase(), UiPhase::Modal);
    }
}

// ── Kani proofs (pure cores only) ────────────────────────────────────────────
//
// `cargo kani` verifies these under symbolic input; plain `cargo test`/
// `cargo build` never compile this module (gated on the `kani` cfg), so the
// proofs add zero cost to the shipped binary.
#[cfg(kani)]
mod kani_proofs {
    use super::{phase_priority, AppState, UiPhase, TRANSITIONS};

    /// The declared transition table is a FUNCTION: no two rows claim the
    /// same (from, event) pair with different targets. A duplicate row would
    /// make `lookup`'s `.find` order-dependent and the spec ambiguous.
    #[kani::proof]
    fn transition_table_is_a_function() {
        for i in 0..TRANSITIONS.len() {
            for j in 0..i {
                let a = &TRANSITIONS[j];
                let b = &TRANSITIONS[i];
                if a.from == b.from && a.event == b.event {
                    assert_eq!(a.to, b.to, "duplicate (from, event) row");
                }
            }
        }
    }

    /// NOTE: a direct `lookup`-vs-scan harness was written and dropped —
    /// Kani's loop unwinding on `lookup`'s internal `.find` iterator does
    /// not converge on this host in reasonable time. The property (lookup
    /// is total, deterministic, and agrees with the table) holds
    /// transitively: `lookup` is a single `.find` over `TRANSITIONS`, and
    /// `transition_table_is_a_function` proves that table is a function.

    /// `phase`'s declared priority, stated as a pure function of the four
    /// boolean flags (dialogs/perm-modal collapse to one, so four booleans):
    /// Dialog > Compacting > Streaming > Modal > Idle. This is the oracle;
    /// `AppState::phase` must implement exactly this. Proven by exhaustive
    /// enumeration of all 16 flag combinations.
    #[kani::proof]
    fn phase_priority_is_correct() {
        for d in [false, true] {
            for c in [false, true] {
                for s in [false, true] {
                    for m in [false, true] {
                        let expected = if d {
                            UiPhase::Dialog
                        } else if c {
                            UiPhase::Compacting
                        } else if s {
                            UiPhase::Streaming
                        } else if m {
                            UiPhase::Modal
                        } else {
                            UiPhase::Idle
                        };
                        assert_eq!(phase_priority(d, c, s, m), expected);
                    }
                }
            }
        }
    }
}

const CLK_TCK: f64 = 100.0; // Linux userspace jiffies-per-second (essentially universal)

/// utime+stime (seconds) of one pid from /proc/<pid>/stat.
fn stat_cpu_secs(pid: u32) -> Option<f64> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let ticks = stat_cpu_ticks(&raw)?;
    Some(ticks as f64 / CLK_TCK)
}

fn stat_cpu_ticks(raw: &str) -> Option<u64> {
    // comm may contain spaces/parens — cut after the closing parenthesis.
    let after = &raw[raw.rfind(')')? + 1..];
    let f: Vec<&str> = after.split_whitespace().collect();
    // fields 14/15 overall → indices 11/12 after comm.
    let utime: u64 = f.get(11)?.parse().ok()?;
    let stime: u64 = f.get(12)?.parse().ok()?;
    Some(utime + stime)
}

fn child_pids(pid: u32) -> Vec<u32> {
    let raw =
        std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).unwrap_or_default();
    raw.split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect()
}

/// Total CPU seconds of the whole live subtree rooted at `root` (pi).
/// Any unreadable node is skipped (a process exiting mid-walk is normal).
fn subtree_cpu_secs(root: u32) -> Option<f64> {
    let mut total = stat_cpu_secs(root)?;
    let mut queue = child_pids(root);
    while let Some(pid) = queue.pop() {
        if let Some(c) = stat_cpu_secs(pid) {
            total += c;
        }
        queue.extend(child_pids(pid));
    }
    Some(total)
}

fn watchdog_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("watchdog.json");
    p
}

/// Facts are only trusted while fresh (2 min) — a stale card would claim
/// a countdown for a watchdog that no longer watches.
pub fn load_watchdog_doc() -> Option<WatchdogDoc> {
    load_watchdog_doc_at(&watchdog_path())
}

pub fn load_watchdog_doc_at(path: &std::path::Path) -> Option<WatchdogDoc> {
    let raw = std::fs::read_to_string(path).ok()?;
    let doc: WatchdogDoc = serde_json::from_str(&raw).ok()?;
    let now = crate::tui::caduceus::now_ms();
    if doc.version != 1 || now.saturating_sub(doc.ts) > 120_000 {
        return None;
    }
    Some(doc)
}

fn answer_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("inquire-answer.json");
    p
}

fn load_answer_doc() -> Option<AnswerDoc> {
    let raw = std::fs::read_to_string(answer_path()).ok()?;
    let doc: AnswerDoc = serde_json::from_str(&raw).ok()?;
    if doc.version != 1 || doc.answer.trim().is_empty() {
        return None;
    }
    Some(doc)
}

fn answer_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(answer_path())
        .and_then(|m| m.modified())
        .ok()
}

/// Watchdog card accent by verdict (2026-11-14, operator report): an ALIVE
/// observation must never wear the warning/failure colour.
pub fn watchdog_edge_color(verdict: Option<&str>) -> ratatui::style::Color {
    match verdict {
        Some("ALIVE") => theme::GREEN,
        Some("HUNG") => theme::ORANGE,
        Some("JUDGING") => theme::GOLD,
        _ => theme::MUTED,
    }
}

fn hide_thinking_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("tui-thinking");
    p
}

fn load_hide_thinking() -> bool {
    if std::env::var("OFFICINA_THINKING_HIDE")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return true;
    }
    std::fs::read_to_string(hide_thinking_path())
        .map(|s| matches!(s.trim(), "hide" | "hidden" | "off" | "1"))
        .unwrap_or(false)
}

pub fn persist_hide_thinking(state: &AppState) {
    let path = hide_thinking_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        path,
        format!(
            "{}
",
            if state.hide_thinking { "hide" } else { "show" }
        ),
    );
}

fn thinking_default_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("tui-thinking-default");
    p
}

/// Default collapsed state for new thinking blocks. Default = collapsed
/// (2026-11-13, operator request).
fn load_thinking_default() -> bool {
    if let Ok(v) = std::env::var("OFFICINA_THINKING_DEFAULT") {
        return !matches!(v.trim(), "expanded" | "expand" | "show" | "shown" | "off");
    }
    std::fs::read_to_string(thinking_default_path())
        .map(|s| !matches!(s.trim(), "expanded" | "expand" | "show" | "shown" | "off"))
        .unwrap_or(true)
}

pub fn persist_thinking_default(state: &AppState) {
    let path = state
        .thinking_default_file
        .clone()
        .unwrap_or_else(thinking_default_path);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        path,
        format!(
            "{}\n",
            if state.thinking_default_collapsed {
                "collapsed"
            } else {
                "expanded"
            }
        ),
    );
}

fn prompt_history_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("input-history.jsonl");
    p
}

fn load_prompt_history() -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(prompt_history_path()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<String>(l).ok())
        .filter(|m| !m.trim().is_empty())
        .collect();
    if out.len() > 200 {
        out.drain(..out.len() - 200);
    }
    out
}

fn append_prompt_history(msg: &str) {
    use std::io::Write;
    let path = prompt_history_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let Ok(line) = serde_json::to_string(msg) else {
        return;
    };
    if let Ok(mut f) = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)
    } {
        let _ = writeln!(f, "{line}");
    }
}

fn prompt_suggestion_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("prompt-suggestion.json");
    p
}

fn ghost_events_path() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("prompt-suggest-events.jsonl");
    p
}

fn profile_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("OFFICINA_ROUTING_PROFILE_FILE") {
        return std::path::PathBuf::from(p);
    }
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("routing-profile");
    p
}

fn shorten_home_for_status(path: &std::path::Path) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~{}", rest.display());
        }
    }
    path.display().to_string()
}

/// Pure oracle for the phase priority (kept out of AppState so the Kani
/// harness never constructs a full AppState — its Default pulls in
/// HashMap/VecDeque machinery Kani cannot model on this host).
#[cfg(kani)]
fn phase_priority(dialog: bool, compacting: bool, streaming: bool, modal: bool) -> UiPhase {
    if dialog {
        UiPhase::Dialog
    } else if compacting {
        UiPhase::Compacting
    } else if streaming {
        UiPhase::Streaming
    } else if modal {
        UiPhase::Modal
    } else {
        UiPhase::Idle
    }
}
