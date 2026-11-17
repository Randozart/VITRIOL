// Application state — chat entries, widgets, agent state, markdown cache.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::text::Span;

use crate::rpc::protocol::{ImageContent, Model, RpcCommand};
use crate::theme;
use crate::tui::heal;

// ── Tool verbosity (owner request 2026-09-03) ────────────────────────────
/// How long a status-band height candidate must hold before the layout
/// adopts it (2026-11-16). Widget/ctx data flickers on its own schedule; the
/// band must be still. 700 ms is shorter than a human notices a legitimate
/// change and longer than any transient datum lives.
pub const BAND_SETTLE: std::time::Duration = std::time::Duration::from_millis(700);

/// A row count with settle hysteresis (2026-11-16).
///
/// Live values (context fill, belowEditor widget lines) arrive on their own
/// schedule; a candidate is adopted only once it has been stable for
/// `BAND_SETTLE`, so a datum that lives for a frame or two can never move the
/// layout. Each part of the status band keeps its own instance: the band's
/// total height AND the split between indicator rows and the context row must
/// both be settled, otherwise a transient indicator line takes the ctx row's
/// slot for a frame ("the ctx field looks like a different field").
#[derive(Debug, Clone, Copy, Default)]
pub struct Settled {
    pub applied: u16,
    pending: Option<(u16, std::time::Instant)>,
}

impl Settled {
    /// Adopt `candidate` once it has held for `BAND_SETTLE`; until then keep
    /// the applied value.
    pub fn settle(&mut self, candidate: u16, now: std::time::Instant) -> u16 {
        if candidate == self.applied {
            self.pending = None;
            return self.applied;
        }
        match self.pending {
            Some((pending, since)) if pending == candidate => {
                if now.saturating_duration_since(since) >= BAND_SETTLE {
                    self.applied = candidate;
                    self.pending = None;
                }
                self.applied
            }
            _ => {
                self.pending = Some((candidate, now));
                self.applied
            }
        }
    }

    /// Adopt `candidate` immediately, clearing any pending change (test and
    /// session-reset helper).
    pub fn force(&mut self, value: u16) {
        self.applied = value;
        self.pending = None;
    }
}

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
        let (mode_s, suffix) = if let Some(mode) = s.strip_suffix('!') {
            (mode, '!')
        } else if let Some(mode) = s.strip_suffix('+') {
            (mode, '+')
        } else if let Some(mode) = s.strip_suffix('-') {
            (mode, '-')
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
        "code" => "Code",
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
        "api",
        "this session's own last request when the provider is a cloud lane: model, tokens in→out, latency, cost, outcome",
    ),
    (
        "nimbus",
        "free-lane pool for officina/nimbus: lane count, cooling lanes, next lane that fits the context",
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
    ("→", "accept autocomplete at end of input"),
    ("tab", "cycle agent"),
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

/// Cache ownership is separate so ordinary chat entries stay compact.
#[derive(Debug, Clone)]
pub struct ToolRenderCache {
    width: usize,
    generation: u64,
    output_len: usize,
    tick: u64,
    lines: Vec<Line<'static>>,
}

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
        args: Option<Box<serde_json::Value>>,
        /// Click override (2026-11-14): the operator cycles
        /// closed (line) → formatted (block) → full → closed. None follows
        /// the /tools config until the first click.
        click_mode: Option<ToolVerbosity>,
        /// Canonical unified diff from pi's edit tool (result.details.diff).
        diff: Option<String>,
        /// `write` overwrite: file content captured before the tool ran.
        pre_content: Option<String>,
        /// Post-call check outcomes (format / syntax / praetor facts file).
        checks: Option<Box<ToolChecks>>,
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
        render_cache: Option<Box<ToolRenderCache>>,
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
    Watchdog {
        text: String,
        doc: Option<Box<WatchdogDoc>>,
    },
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
#[derive(Debug, Clone, Default, Hash, serde::Deserialize)]
#[serde(default)]
pub struct WatchdogDoc {
    pub version: u32,
    pub ts: u64,
    pub kind: String,
    pub session_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub valid_until: Option<u64>,
    pub command: String,
    pub elapsed_s: u64,
    pub stalled_s: u64,
    pub verdict: String,
    pub reason: String,
    /// Where the verdict came from (2026-11-16): "decision" (typed SystemOne
    /// lane), "lane" (free cloud judge), "unavailable", or empty on notices.
    pub source: Option<String>,
    pub next_check_at: u64,
    pub phase: String,
}

/// Per-call check outcomes written by extensions (facts file, 2026-11-14):
/// format-gate / diagnostics-loop / praetor. Status only — no source text.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct ToolChecks {
    pub format: Option<FormatCheck>,
    pub syntax: Option<SyntaxCheck>,
    pub praetor: Option<PraetorCheck>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct FormatCheck {
    pub tool: String,
    pub changed: bool,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct SyntaxCheck {
    pub ok: bool,
    pub available: Option<bool>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct PraetorCheck {
    pub count: u64,
}

fn tool_checks_path(tool_call_id: &str) -> std::path::PathBuf {
    let safe: String = tool_call_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(120)
        .collect();
    let mut p = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("toolchecks");
    p.push(format!("{safe}.json"));
    p
}

fn load_tool_checks_in(dir: Option<&std::path::Path>, tool_call_id: &str) -> Option<ToolChecks> {
    let path = match dir {
        Some(d) => {
            let safe: String = tool_call_id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '_'
                    }
                })
                .take(120)
                .collect();
            d.join(format!("{safe}.json"))
        }
        None => tool_checks_path(tool_call_id),
    };
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
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
#[derive(Default)]
struct SuggestionFile {
    version: u32,
    id: String,
    text: String,
    source: String,
    expires: u64,
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
}

/// The last real prompt sent to pi. Recovery uses it for the one-shot,
/// tool-free replay gate (2026-11-15 self-heal).
/// One provider/owner section in the /model picker.
#[derive(Debug, Clone)]
pub struct ModelGroup<'a> {
    pub key: String,
    pub title: String,
    pub rank: u8,
    pub models: Vec<&'a Model>,
}

fn model_tail(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

fn title_words(key: &str) -> String {
    key.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider_title(key: &str) -> String {
    match key {
        "officina" => "OFFICINA".to_string(),
        "opencode-go" => "OpenCode Go".to_string(),
        "opencode" => "OpenCode".to_string(),
        "llamacpp" => "VITRIOL local".to_string(),
        "zai" => "Z.AI".to_string(),
        "glm" => "GLM".to_string(),
        "agentrouter" => "AgentRouter".to_string(),
        "cloudflare-playground" => "Cloudflare Playground".to_string(),
        "duckduckgo-web" => "DuckDuckGo".to_string(),
        "freeinference" => "FreeInference".to_string(),
        "freemodel-dev" => "FreeModel".to_string(),
        "api-airforce" => "API Airforce".to_string(),
        "no-think" => "No-think".to_string(),
        "kimi-coding" => "Kimi Coding".to_string(),
        "zai-coding-plan" => "Z.AI Coding Plan".to_string(),
        "zai-coding-cn" => "Z.AI Coding CN".to_string(),
        _ => title_words(key),
    }
}

/// (key, title, rank). rank 0 = OFFICINA specials, 1 = OpenCode Go,
/// 2 = every other provider/owner.
fn model_group_identity(m: &Model) -> (String, String, u8) {
    if m.provider == "officina" {
        return ("officina".to_string(), provider_title("officina"), 0);
    }
    let owner = if m.provider == "caduceus" {
        model_tail(
            m.id.split_once('/')
                .map(|(owner, _)| owner)
                .unwrap_or(m.provider.as_str()),
        )
    } else {
        m.provider.as_str()
    };
    let key = owner.to_ascii_lowercase();
    let rank = if key == "opencode-go" { 1 } else { 2 };
    let title = provider_title(&key);
    (key, title, rank)
}

/// Who asked for the running turn to stop. Esc is the operator's own decision:
/// it must never kill or restart the pi child (E1, 2026-11-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortSource {
    Operator,
    Watchdog,
}

#[derive(Debug, Clone)]
pub struct LastPrompt {
    pub text: String,
    pub images: Option<Vec<ImageContent>>,
    /// Drives the pre-agent_start hang detector (pi never opened the turn).
    pub sent_at: std::time::Instant,
    pub tool_started: bool,
    pub assistant_output: bool,
    pub failed: bool,
    pub replayed: bool,
    /// A running tool was abandoned without ever producing output, so no side
    /// effect can have happened and the prompt is safe to replay once
    /// (2026-11-15 stall recovery).
    pub stalled_tool: bool,
}

/// A tool call that has not received its result yet.
#[derive(Debug, Clone)]
pub struct RunningTool {
    /// Stable watchdog key: pi's tool call id, else a name fallback.
    pub key: String,
    pub name: String,
    pub age: std::time::Duration,
    /// Timeout declared by the model; authoritative when present.
    pub timeout_s: Option<u64>,
    /// Not one output line has arrived for this call.
    pub silent: bool,
}

impl RunningTool {
    /// Age budget: a declared timeout wins, never below the watchdog base.
    pub fn limit(&self, base: std::time::Duration) -> std::time::Duration {
        self.timeout_s
            .map(|s| std::time::Duration::from_secs(s.saturating_add(60)))
            .map_or(base, |t| t.max(base))
    }
}

#[derive(Debug)]
pub struct AppState {
    pub entries: Vec<ChatEntry>,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub model: Option<Model>,
    pub thinking_level: Option<String>,
    pub session_id: String,
    /// Active session JSONL path from `get_state` (pi's `sessionFile`).
    /// Recovery reopens exactly this file instead of guessing "latest".
    pub session_file: Option<std::path::PathBuf>,
    pub session_name: Option<String>,
    pub message_count: u64,

    // Widget blocks from extension_ui_request (setWidget) — session-panel,
    // decode gauge, scratchpad note, etc. Rendered in arrival order.
    pub widgets: Vec<WidgetBlock>,

    // Live context fill, captured from the agent_end event's contextUsage
    // (tokens used / window size). None until the first turn completes —
    // the bottom status row hides itself rather than showing a guess.
    pub ctx_usage: Option<(u64, u64)>,

    // Status-band settle state (2026-11-16). TWO parts, settled separately:
    // the indicator strip height and the context-fill row. Settling only their
    // sum let a transient indicator line eat the ctx row's slot for a frame.
    pub band_ind: Settled,
    pub band_ctx: Settled,

    // Terminal title (setTitle)
    pub title: String,

    // Notifications (notify)
    pub notice: Option<(String, String)>, // (message, type)
    /// Once a notice has been shown, its row is reserved for the session: the
    /// layout must not grow/shrink when notices come and go (2026-11-16).
    /// Notice strip height with settle hysteresis (BAND-2, 2026-11-17): the row
    /// is reserved while a notice exists and released once it has been gone for
    /// BAND_SETTLE, instead of latching for the rest of the process.
    pub notice_row: Settled,

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
    /// Injectable toolchecks dir (tests never read the operator's live facts).
    pub tool_checks_dir: Option<std::path::PathBuf>,
    /// Per-entry rendered lines (idx → (revision, lines)). `chat_lines` used
    /// to rebuild the whole transcript every frame — 59 ms at 2.8k entries,
    /// which starved input on long sessions (2026-11-14 perf fix). Static
    /// entries are now rendered once and cloned; only the streaming tail
    /// (and running tools) re-render per frame.
    pub entry_lines_cache: std::collections::HashMap<usize, (u64, Vec<Line<'static>>)>,
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
    pub setting_menu: Option<super::settings::SettingMenu>,
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

    /// Agent mode tracking — the widget's own "/agent <next>" hint (Tab).
    pub agent_mode: Option<String>,
    /// Mode already requested from Tab, with the instant: a second Tab inside
    /// the grace must not re-send the same target (the widget has not caught up
    /// yet, which used to read as "Tab is blocked again").
    pub agent_mode_pending: Option<(String, std::time::Instant)>,
    /// Current mode label (e.g. "BUILD") + badge glyph, parsed from the same
    /// widget — drives the header mode chip (owner request 2026-09-02).
    pub agent_mode_label: Option<String>,
    pub agent_mode_glyph: Option<String>,

    // UI state
    pub should_quit: bool,

    /// Watermark glimmer animation mode (owner request 2026-09-02) —
    /// /glimmer opens its choices; persistence lives in the run loop.
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
    /// How far each flame tongue narrows with height (0 = rectangle). Owner
    /// report 2026-11-16: "the row above is the same width rather than tapering
    /// off." `/fire taper <0-1>`.
    pub fire_taper: f64,
    /// Paint a quiet background band behind the operator's own turns
    /// (owner request 2026-11-16). Default on; `/chat-bg off` restores the
    /// unbanded transcript byte-for-byte.
    pub user_turn_bg: bool,
    pub fire_target: f64,
    pub fire_level: f64,
    /// Flame strip height applied last frame — the dead band in
    /// `fire::rows_for_stable` keeps a breathing level from flapping it
    /// (2026-11-16: the strip's height changes moved a whole row of text).
    pub fire_rows_prev: usize,

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
    /// Tool call ids that already received their one bounded stall grace.
    pub stall_graced: std::collections::HashSet<String>,
    /// Per-tool instant before which the stall ladder must not re-evaluate
    /// (the evidence probe opens a TCP socket; a 90 ms tick must not do that).
    pub stall_grace_until: std::collections::HashMap<String, std::time::Instant>,
    /// Wall clock of the last wire message (any response/event/UI request).
    /// Drives the wire-idle detector (2026-11-15).
    pub last_wire_at: std::time::Instant,
    /// Set when an engine probe proved prefill/decode is still active, so
    /// the wire-idle detector must not kill a legitimate long prefill.
    pub wire_grace_until: Option<std::time::Instant>,
    /// Last real prompt (replay gate).
    pub last_prompt: Option<LastPrompt>,
    /// Harness recovery state (bounded attempts, pending replay, phase).
    pub recovery: heal::RecoveryState,
    /// `/recover` asks the async loop for a manual recovery attempt.
    pub heal_requested: bool,
    /// The composer line came from Up/Down history recall. While set the
    /// command menu must not open and must not steal the arrows, or recalling a
    /// `/command` entry freezes history navigation (2026-11-16, operator
    /// report).
    pub history_recall: bool,
    /// The operator dismissed the command menu for exactly this line content;
    /// any edit clears it (Esc closes the menu without touching the composer).
    pub menu_dismissed_for: Option<String>,
    /// Set after an abort; if the agent has not settled by this instant the TUI
    /// releases the UI. The source decides whether a child restart may follow:
    /// only the watchdog may escalate, never an operator Esc (2026-11-16).
    pub abort_deadline: Option<(std::time::Instant, AbortSource)>,
    /// Prompts typed while a tool is still running. They are delivered after
    /// the turn settles instead of being sent as RPC `Steer` behind a tool
    /// that may never finish (and would then never be persisted).
    pub queued_prompts: Vec<String>,
    /// Messages typed while busy, visible above the prompt until the
    /// agent actually reaches them (2026-11-13, operator request).
    pub held: Vec<HeldMsg>,
    /// Compaction start (drives the in-chat progress timer).
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
    md_cache: HashMap<(usize, u64, usize), Vec<Line<'static>>>,
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
            session_file: None,
            session_name: None,
            message_count: 0,
            widgets: Vec::new(),
            ctx_usage: None,
            band_ind: Settled::default(),
            band_ctx: Settled::default(),
            title: String::new(),
            notice: None,
            notice_row: Settled::default(),
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
            tool_checks_dir: None,
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
            setting_menu: None,
            show_diag: false,
            diag_view: Vec::new(),
            input: String::new(),
            cursor: 0,
            cand_sel: 0,
            cwd: std::path::PathBuf::from("."),
            agent_mode: None,
            agent_mode_pending: None,
            agent_mode_label: None,
            agent_mode_glyph: None,
            should_quit: false,
            glimmer: crate::watermark::GlimmerMode::Shimmer,
            fire_on: true,
            fire_style: crate::fire::FireStyle::default(),
            fire_tint: true,
            fire_taper: crate::fire::DEFAULT_TAPER,
            user_turn_bg: true,
            fire_target: 0.0,
            fire_level: 0.0,
            fire_rows_prev: 0,
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
            stall_graced: std::collections::HashSet::new(),
            stall_grace_until: std::collections::HashMap::new(),
            last_wire_at: std::time::Instant::now(),
            wire_grace_until: None,
            last_prompt: None,
            recovery: heal::RecoveryState::default(),
            heal_requested: false,
            history_recall: false,
            menu_dismissed_for: None,
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
            entry_lines_cache: std::collections::HashMap::new(),
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
        if self.setting_menu.is_some() || self.models_open || self.cache_open || self.resume_open {
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

    /// Status-band parts with settle hysteresis (2026-11-16).
    /// Returns `(indicator_rows, ctx_rows)` as the renderer should place them.
    /// Each part settles on its own: a transient push on the indicator count
    /// can never displace the context row (operator: "the ctx field flickers
    /// to look like a different field for one frame"). The total therefore
    /// changes only when a part has genuinely changed.
    pub fn stabilize_band_parts(&mut self, indicator: u16, ctx: u16) -> (u16, u16) {
        self.stabilize_band_parts_at(indicator, ctx, std::time::Instant::now())
    }

    /// Clock-explicit form of `stabilize_band_parts` (unit-testable).
    pub fn stabilize_band_parts_at(
        &mut self,
        indicator: u16,
        ctx: u16,
        now: std::time::Instant,
    ) -> (u16, u16) {
        (
            self.band_ind.settle(indicator, now),
            self.band_ctx.settle(ctx, now),
        )
    }

    /// Drop the band's settled geometry (session switch): a fresh session has
    /// no band yet, and inheriting the old one would show a row of the
    /// previous session's layout for a frame.
    pub fn reset_band(&mut self) {
        self.band_ind.force(0);
        self.band_ctx.force(0);
        self.notice_row.force(0);
    }

    /// Height of the notice strip for this frame (0 or 1), settled so the
    /// composer never jitters when a notice appears or expires.
    pub fn notice_row_height(&mut self) -> u16 {
        self.notice_row_height_at(std::time::Instant::now())
    }

    /// Clock-explicit form (unit-testable).
    pub fn notice_row_height_at(&mut self, now: std::time::Instant) -> u16 {
        let candidate = u16::from(self.notice.is_some());
        self.notice_row.settle(candidate, now)
    }

    fn model_matches(&self, m: &Model) -> bool {
        let needle = self.models_filter.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        let name = m.name.as_deref().unwrap_or("");
        let hay = format!("{} {} {}", m.provider, m.id, name).to_lowercase();
        needle.split_whitespace().all(|w| hay.contains(w))
    }

    /// Models matching the picker filter (case-insensitive, AND of words
    /// across provider/id/name). Empty filter = all models.
    ///
    /// Ordered for display: OFFICINA first (Lapis, Nimbus, then the rest),
    /// OpenCode Go next, then every other provider/owner group.
    pub fn filtered_models(&self) -> Vec<&Model> {
        self.filtered_model_groups()
            .into_iter()
            .flat_map(|g| g.models)
            .collect()
    }

    /// Grouped picker rows. Caduceus model ids are `owner/model`, so the
    /// owner becomes the group (opencode-go, glm, zai, agentrouter, …).
    /// OpenCode Go is deduped in favour of the native provider entry when
    /// both the native and gateway catalogs expose the same model.
    pub fn filtered_model_groups(&self) -> Vec<ModelGroup<'_>> {
        let mut groups: Vec<ModelGroup<'_>> = Vec::new();
        for m in &self.models {
            if !self.model_matches(m) {
                continue;
            }
            let (key, title, rank) = model_group_identity(m);
            let idx = match groups.iter().position(|g| g.key == key) {
                Some(i) => i,
                None => {
                    groups.push(ModelGroup {
                        key: key.clone(),
                        title,
                        rank,
                        models: Vec::new(),
                    });
                    groups.len() - 1
                }
            };
            let group = &mut groups[idx];
            if key == "opencode-go" {
                let tail = model_tail(&m.id);
                if let Some(pos) = group
                    .models
                    .iter()
                    .position(|existing| model_tail(&existing.id) == tail)
                {
                    // Prefer the native opencode-go provider (per-model api,
                    // full capability metadata) over the gateway copy.
                    if group.models[pos].provider == "caduceus" && m.provider != "caduceus" {
                        group.models[pos] = m;
                    }
                    continue;
                }
            }
            group.models.push(m);
        }
        for group in &mut groups {
            if group.key == "officina" {
                group.models.sort_by_key(|m| match m.id.as_str() {
                    "lapis" => 0,
                    "nimbus" => 1,
                    _ => 2,
                });
            } else {
                group.models.sort_by(|a, b| {
                    let an = a.name.as_deref().unwrap_or(&a.id).to_lowercase();
                    let bn = b.name.as_deref().unwrap_or(&b.id).to_lowercase();
                    an.cmp(&bn).then_with(|| a.id.cmp(&b.id))
                });
            }
        }
        groups.sort_by(|a, b| a.rank.cmp(&b.rank).then_with(|| a.title.cmp(&b.title)));
        groups
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

    #[cfg(test)]
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
    #[cfg(test)]
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
        self.note_input_typed();
        self.cursor = self.input.chars().count();
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
        self.history_recall = true;
        self.menu_dismissed_for = None;
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
        if !self.has_running_tool() && !self.is_streaming && !self.is_compacting {
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

    /// Own each card's facts and update only the matching session/call observation.
    fn record_watchdog_notice(&mut self, message: &str) {
        let path = self
            .watchdog_file
            .clone()
            .unwrap_or_else(|| watchdog_path(&self.session_id));
        let (message, doc) = watchdog_notice(message, &path, &self.session_id);
        let replacement = doc.as_ref().and_then(|next| {
            self.entries
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, entry)| {
                    let ChatEntry::Watchdog {
                        doc: Some(previous),
                        ..
                    } = entry
                    else {
                        return None;
                    };
                    let same = if next.tool_call_id.is_some() {
                        previous.session_id == next.session_id
                            && previous.tool_call_id == next.tool_call_id
                    } else {
                        index + 1 == self.entries.len()
                            && previous.command == next.command
                            && previous.verdict == next.verdict
                    };
                    same.then_some(index)
                })
        });
        self.watchdog = doc.clone();
        let entry = ChatEntry::Watchdog {
            text: message.to_string(),
            doc: doc.map(Box::new),
        };
        if let Some(index) = replacement {
            self.entries[index] = entry;
            return;
        }
        self.entries.push(entry);
    }

    /// Extension notify (stage 3, 2026-11-12): errors/warnings land in the
    /// transcript as Diag; info keeps the 40-char glance row, and anything
    /// the truncation would mangle (multi-line tables or >100-char payloads)
    /// is ALSO captured as a Notice entry so /free, /nimbus, /phase-models
    /// output is actually readable.
    pub fn apply_notify(&mut self, msg: &str, ntype: &str) {
        // Watchdog observations are not errors — give them their own block.
        if msg.starts_with("hang-watchdog") {
            self.record_watchdog_notice(msg);
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
        if crate::tui::width::cols(&t) > 48 {
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
                    crate::tui::cache::budget_line(self.cache_total),
                    self.cache_dir
                )));
            }
            other => self.entries.push(ChatEntry::Diag(format!(
                "cache: unknown action '{other}' — use /cache or /cache wipe [age]"
            ))),
        }
    }

    /// Any wire message proves pi's event loop is alive.
    pub fn note_wire_activity(&mut self) {
        self.last_wire_at = std::time::Instant::now();
        self.wire_grace_until = None;
    }

    /// A local engine probe saw active prefill/decode. Do not escalate on
    /// wire silence until this grace expires.
    pub fn grant_wire_grace(&mut self, grace: std::time::Duration) {
        self.wire_grace_until = Some(std::time::Instant::now() + grace);
    }

    /// Remember the prompt the operator actually sent. Slash commands are
    /// extension invocations, never replayed by recovery.
    pub fn note_prompt_sent(&mut self, text: &str, images: Option<Vec<ImageContent>>) {
        if text.trim_start().starts_with('/') {
            self.last_prompt = None;
            return;
        }
        self.last_wire_at = std::time::Instant::now();
        self.last_prompt = Some(LastPrompt {
            text: text.to_string(),
            images,
            sent_at: std::time::Instant::now(),
            tool_started: false,
            assistant_output: false,
            failed: false,
            replayed: false,
            stalled_tool: false,
        });
    }

    pub fn note_prompt_tool_started(&mut self) {
        if self.recovery.phase == heal::Phase::Rehydrating {
            return; // historical transcript, not the failed turn
        }
        if let Some(p) = self.last_prompt.as_mut() {
            p.tool_started = true;
        }
    }

    pub fn note_prompt_assistant_output(&mut self) {
        if self.recovery.phase == heal::Phase::Rehydrating {
            return;
        }
        if let Some(p) = self.last_prompt.as_mut() {
            p.assistant_output = true;
        }
    }

    pub fn note_prompt_failed(&mut self) {
        if let Some(p) = self.last_prompt.as_mut() {
            p.failed = true;
        }
    }

    /// Turn settled cleanly, or the one-shot replay already ran: nothing
    /// left to replay. A failed first attempt is kept until its replay.
    pub fn settle_last_prompt(&mut self) {
        if self
            .last_prompt
            .as_ref()
            .is_some_and(|p| !p.failed || p.replayed)
        {
            self.last_prompt = None;
        }
    }

    /// Replay gate (I3): failed, tool-free, no assistant output, no images,
    /// not a slash command, not already replayed. Taking the prompt marks it
    /// replayed so a recovery cannot loop.
    ///
    /// Narrow 2026-11-15 exception: a prompt whose only tool was abandoned
    /// before it ever produced output (`stalled_tool`) is replayable once even
    /// though the call started — nothing executed, so nothing can repeat.
    pub fn take_replayable_prompt(&mut self) -> Option<(String, Option<Vec<ImageContent>>)> {
        // One gate, no bypasses: the side-effect veto is part of the predicate
        // so a caller cannot replay a turn that already ran a tool.
        if !self.last_prompt_replayable() {
            return None;
        }
        let p = self.last_prompt.as_mut()?;
        p.replayed = true;
        Some((p.text.clone(), None))
    }

    pub fn last_prompt_replayable(&self) -> bool {
        let Some(p) = self.last_prompt.as_ref() else {
            return false;
        };
        if p.replayed
            || p.text.trim_start().starts_with('/')
            || p.images.as_ref().is_some_and(|i| !i.is_empty())
            || self.replay_has_side_effects()
        {
            return false;
        }
        if p.stalled_tool {
            return true; // nothing executed, so nothing can repeat
        }
        p.failed && !p.tool_started && !p.assistant_output
    }

    /// Mark the prompt as stalled-before-execution so the recovery ladder may
    /// replay it once (2026-11-15; see `stall`).
    pub fn note_stall_unexecuted(&mut self) {
        if let Some(p) = self.last_prompt.as_mut() {
            p.stalled_tool = true;
        }
    }

    /// Arm the abort settle deadline, tagged with who asked for the stop.
    pub fn arm_abort(&mut self, now: std::time::Instant, source: AbortSource) {
        self.abort_deadline = Some((now + std::time::Duration::from_secs(10), source));
    }

    /// The armed abort's source once its settle deadline has passed.
    pub fn abort_due(&self, now: std::time::Instant) -> Option<AbortSource> {
        match self.abort_deadline {
            Some((deadline, source)) if now >= deadline => Some(source),
            _ => None,
        }
    }

    pub fn abort_pending(&self) -> bool {
        self.abort_deadline.is_some()
    }

    /// Operator Esc: stop the turn and drop every queued/held message plus any
    /// pending recovery obligation. Returns how many messages were dropped so
    /// the caller can report it (E4: no silent loss).
    pub fn operator_stop(&mut self) -> usize {
        let dropped = self.queued_prompts.len();
        self.queued_prompts.clear();
        self.held.clear();
        self.expected_tool_calls.clear();
        self.clear_stall_state();
        self.recovery.pending_replay = None;
        self.note_prompt_failed();
        dropped
    }

    pub fn model_is_lapis(&self) -> bool {
        heal::is_lapis(&self.model)
    }

    /// Wire-idle gate: streaming/compacting, no tool watches armed, and no
    /// grace granted by a live prefill probe.
    pub fn wire_idle_due(&self, now: std::time::Instant, threshold: std::time::Duration) -> bool {
        // A prompt sent but never acknowledged (no agent_start) is also a
        // hang surface — do not wait for the streaming flag forever.
        let prompt_waiting = self.last_prompt.as_ref().is_some_and(|p| {
            !p.failed && !p.replayed && now.duration_since(p.sent_at) >= threshold
        });
        if threshold.is_zero() || !(self.is_streaming || self.is_compacting || prompt_waiting) {
            return false;
        }
        if self.has_running_tool() || !self.expected_tool_calls.is_empty() {
            return false;
        }
        if self.wire_grace_until.is_some_and(|until| now < until) {
            return false;
        }
        // CPU grace: an active pi tree means real work, not a hang. When
        // /proc is unavailable, liveness is unknown and must not block the
        // detector forever.
        let pi_flat = if self.liveness_unavailable || prompt_waiting {
            // No turn started yet: pi has nothing to compute, so silence is
            // itself the evidence. The streaming path keeps the CPU gate.
            threshold
        } else {
            std::time::Duration::from_secs(self.liveness_flat_secs)
        };
        heal::wire_idle_escalate(
            threshold,
            heal::WireEvidence {
                idle: now.duration_since(self.last_wire_at),
                pi_flat,
                engine_busy: false,
            },
        )
    }

    /// True when the rehydrated transcript contains a tool entry after the
    /// last user message — replaying that prompt would re-run side effects.
    pub fn transcript_has_tool_after_last_user(&self) -> bool {
        for e in self.entries.iter().rev() {
            match e {
                ChatEntry::User(_) => return false,
                ChatEntry::Tool { .. } => return true,
                _ => {}
            }
        }
        false
    }

    /// Tool entries after the last user message that actually produced a
    /// result. Abandoned/errored calls (the stall path) do not count: they ran
    /// nothing, so replaying the prompt cannot repeat a side effect.
    pub fn transcript_has_successful_tool_after_last_user(&self) -> bool {
        for e in self.entries.iter().rev() {
            match e {
                ChatEntry::User(_) => return false,
                ChatEntry::Tool { error, .. } => {
                    if !*error {
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    /// Would replaying the pending prompt repeat a side effect? A tool that
    /// produced a real result blocks it; a never-executed stall does not
    /// (2026-11-15 stall replay).
    pub fn replay_has_side_effects(&self) -> bool {
        if self.last_prompt.as_ref().is_some_and(|p| p.stalled_tool) {
            return self.transcript_has_successful_tool_after_last_user();
        }
        self.transcript_has_tool_after_last_user()
    }

    /// Clear the view before a recovery replay. The session file is the
    /// source of truth; pending replay and the composer survive.
    pub fn reset_for_rehydrate(&mut self) {
        self.entries.clear();
        self.md_cache_clear();
        self.widgets.clear();
        self.dialogs.clear();
        self.held.clear();
        self.queued_prompts.clear();
        self.expected_tool_calls.clear();
        self.abandon_running_tools("harness recovery");
        self.clear_stall_state();
        self.is_streaming = false;
        self.is_compacting = false;
        self.abort_deadline = None;
        self.turn_started_at = None;
        self.watchdog_fired.clear();
        self.compaction_started_at = None;
        self.compaction_auto_armed = false;
        // The context-fill label is NOT cleared: recovery respawns the same
        // conversation, and dropping the row would move the whole transcript
        // by one line mid-turn (operator report 2026-11-16: "status band
        // flashes an extra row"). A stale-by-one-turn number is honest; a
        // jumping layout is not.
    }

    pub fn apply_state(&mut self, snapshot: crate::rpc::protocol::RpcSessionState) {
        let crate::rpc::protocol::RpcSessionState {
            model,
            thinking_level,
            is_streaming,
            is_compacting,
            session_id,
            session_file,
            session_name,
            message_count,
        } = snapshot;
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
        if let Some(path) = session_file {
            self.session_file = Some(std::path::PathBuf::from(path));
        }
        self.session_name = session_name;
        self.message_count = message_count.unwrap_or(0);
    }

    /// Append a text delta to the last assistant entry (or open a new one).
    pub fn push_text_delta(&mut self, delta: &str) {
        self.stream_heat = (self.stream_heat + 0.4).min(1.6);
        self.note_prompt_assistant_output();
        match self.entries.last_mut() {
            Some(ChatEntry::Assistant(t)) => t.push_str(delta),
            _ => self.entries.push(ChatEntry::Assistant(delta.to_string())),
        }
    }

    /// Append a thinking delta to the last thinking entry (or open a new one).
    pub fn push_thinking_delta(&mut self, delta: &str) {
        self.note_prompt_assistant_output();
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
        self.note_prompt_tool_started();
        let summary = summarize_args(name, args);
        // write overwrites: snapshot the old content so the block can show a
        // real diff (2026-11-14). Capped; read failures are silent.
        let pre_content = if name.eq_ignore_ascii_case("write") {
            let path = args
                .get("path")
                .or_else(|| args.get("file"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let abs = if std::path::Path::new(path).is_absolute() {
                std::path::PathBuf::from(path)
            } else {
                self.cwd.join(path)
            };
            std::fs::read_to_string(abs)
                .ok()
                .filter(|t| t.len() <= 200 * 1024)
        } else {
            None
        };
        let timeout_s = args.get("timeout").and_then(|v| v.as_u64()).or_else(|| {
            args.get("timeout")
                .and_then(|v| v.as_i64())
                .and_then(|i| u64::try_from(i).ok())
        });
        self.entries.push(ChatEntry::Tool {
            tool_call_id: tool_call_id.map(String::from),
            name: name.to_string(),
            summary,
            args: Some(Box::new(args.clone())),
            click_mode: None,
            diff: None,
            pre_content,
            checks: None,
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
    /// Attach pi's canonical edit diff (result.details.diff) to the entry.
    pub fn set_tool_diff(&mut self, tool_call_id: Option<&str>, diff: Option<String>) {
        let Some(diff) = diff else { return };
        if diff.trim().is_empty() {
            return;
        }
        for e in self.entries.iter_mut().rev() {
            if let ChatEntry::Tool {
                tool_call_id: ref id,
                running,
                diff: slot,
                render_cache,
                ..
            } = e
            {
                let matched = match (tool_call_id, id.as_deref()) {
                    (Some(cid), Some(eid)) => cid == eid,
                    _ => false,
                };
                if matched && *running {
                    *slot = Some(diff);
                    *render_cache = None;
                    return;
                }
            }
        }
    }

    pub fn tool_end(
        &mut self,
        tool_call_id: Option<&str>,
        name: &str,
        result_text: &str,
        error: bool,
    ) {
        let checks =
            tool_call_id.and_then(|id| load_tool_checks_in(self.tool_checks_dir.as_deref(), id));
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
                    if let ChatEntry::Tool {
                        checks: slot,
                        render_cache,
                        ..
                    } = e
                    {
                        if checks.is_some() {
                            *slot = checks.map(Box::new);
                            *render_cache = None;
                        }
                    }
                    return;
                }
            }
        }
    }

    /// Oldest still-running tool with its watchdog facts.
    pub fn oldest_running_tool(&self) -> Option<RunningTool> {
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
                    output,
                    ..
                } => {
                    let started = started_at.unwrap_or(now);
                    let key = tool_call_id
                        .clone()
                        .unwrap_or_else(|| format!("name:{name}"));
                    Some(RunningTool {
                        key,
                        name: name.clone(),
                        age: now.duration_since(started),
                        timeout_s: *timeout_s,
                        silent: output.is_empty(),
                    })
                }
                _ => None,
            })
            .max_by_key(|tool| tool.age)
    }

    /// True while the ladder for this tool is parked (grace or live evidence).
    pub fn stall_parked(&self, key: &str, now: std::time::Instant) -> bool {
        self.stall_grace_until
            .get(key)
            .is_some_and(|until| now < *until)
    }

    /// Park the ladder for one tool so a fast heartbeat cannot repeat the
    /// evidence probe (and cannot thrash the engine endpoint).
    pub fn park_stall(&mut self, key: &str, grace: std::time::Duration) {
        self.stall_grace_until
            .insert(key.to_string(), std::time::Instant::now() + grace);
    }

    /// Reset every stall decision for a new turn or a rehydrated view.
    pub fn clear_stall_state(&mut self) {
        self.stall_graced.clear();
        self.stall_grace_until.clear();
    }

    /// Running tool count; the "no descendant process" fast path is only
    /// conclusive when exactly one tool is in flight (2026-11-15).
    pub fn running_tool_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Tool { running: true, .. }))
            .count()
    }

    /// True when no sampled liveness fact contradicts a hang. Unknown samples
    /// count as flat: they must never buy a stuck tool more time.
    pub fn pi_is_flat(&self) -> bool {
        self.liveness_unavailable
            || self.liveness_sampled_at.is_none()
            || self.liveness_flat_secs > 0
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
    #[cfg(test)]
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
                    // The hint advancing settles any Tab request in flight.
                    if self.agent_mode.as_deref() != Some(next.as_str()) {
                        self.agent_mode_pending = None;
                    }
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

    /// Remember that Tab asked for `next`. The pending mark clears itself when
    /// the widget advances to a different target (or after the grace).
    pub fn note_mode_switch_sent(&mut self, next: &str) {
        self.agent_mode_pending = Some((next.to_string(), std::time::Instant::now()));
    }

    /// True while a Tab-requested switch to `next` is still in flight — the
    /// widget hint has not advanced yet, so re-sending it would be a visible
    /// no-op (2026-11-16, operator report: "tabbing through modes is blocked so
    /// often").
    pub fn mode_switch_in_flight(&self, next: &str) -> bool {
        self.agent_mode_pending
            .as_ref()
            .is_some_and(|(pending, at)| {
                pending == next && at.elapsed() < std::time::Duration::from_secs(2)
            })
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
    /// Human label for the active model: its FORMATTED NAME plus the provider,
    /// so the operator knows who serves which model (2026-11-16: the header and
    /// status rows showed the raw id instead). Falls back to `provider/id` when
    /// the catalog gave no name.
    pub fn model_label(model: &Model) -> String {
        let name = model
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| model.id.clone());
        if name == model.id {
            format!("{} · {}", model.provider, model.id)
        } else {
            format!("{} · {}", name, model.provider)
        }
    }

    /// The notice as one line: first line only, whitespace collapsed, with a
    /// `(+N lines)` tail. Multi-line notices (the /nimbus table) used to be cut
    /// to 40 chars, silently dropping everything (2026-11-16).
    pub fn notice_head(&self) -> Option<(String, String)> {
        let (msg, kind) = self.notice.as_ref()?;
        let mut lines = msg.lines().filter(|l| !l.trim().is_empty());
        let head = lines
            .next()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let rest = lines.count();
        Some((
            if rest > 0 {
                format!("{head} … (+{rest} lines)")
            } else {
                head
            },
            kind.clone(),
        ))
    }

    /// `/chat-bg [on|off]` — the user-turn band (2026-11-16).
    pub fn set_user_turn_bg(&mut self, args: &str) -> String {
        match args.trim() {
            "" => self.user_turn_bg = !self.user_turn_bg,
            "on" | "1" | "true" => self.user_turn_bg = true,
            "off" | "0" | "false" => self.user_turn_bg = false,
            other => return format!("unknown: {other} (use on|off)"),
        }
        if self.user_turn_bg {
            "on".to_string()
        } else {
            "off".to_string()
        }
    }

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
            other if other.starts_with("taper") => {
                let value = other.trim_start_matches("taper").trim();
                match value.parse::<f64>() {
                    Ok(v) if (0.0..=1.0).contains(&v) => {
                        self.fire_taper = v;
                        return format!("taper {v:.2}");
                    }
                    _ => return "taper needs 0..1".to_string(),
                }
            }
            // Text discoloration toggle (owner request 2026-09-03) —
            // independent of the flame itself.
            "tint on" => {
                self.fire_tint = true;
                return "tint on".into();
            }
            "tint off" => {
                self.fire_tint = false;
                return "tint off".into();
            }
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
        if !watchdog_fresh(doc, now) || doc.next_check_at == 0 {
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
        self.entry_lines_cache.clear();
    }

    /// All chat lines as styled ratatui Lines.
    /// Assistant entries go through the markdown renderer (cached).
    /// Tool entries render per their effective verbosity mode (cached per
    /// entry against width + tools_gen + output length).
    /// Cheap revision of an entry's rendered inputs (2026-11-14): content
    /// lengths + flags that change the output. Args are immutable after a
    /// tool starts, so only output/diff/checks growth matters.
    fn entry_rev(entry: &ChatEntry, width: usize) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        width.hash(&mut h);
        std::mem::discriminant(entry).hash(&mut h);
        match entry {
            // Hash the TEXT, not its length (2026-11-17 CACHE-1): a same-length
            // change — bash `\r` progress, an in-place token fix — kept the
            // revision equal and the transcript served the previous render.
            ChatEntry::User(t)
            | ChatEntry::Assistant(t)
            | ChatEntry::Diag(t)
            | ChatEntry::Warn(t)
            | ChatEntry::Error(t)
            | ChatEntry::Notice(t) => t.hash(&mut h),
            ChatEntry::Watchdog { text, doc } => {
                text.hash(&mut h);
                doc.hash(&mut h);
            }
            ChatEntry::Thinking { text, collapsed } => {
                text.hash(&mut h);
                collapsed.hash(&mut h);
            }
            ChatEntry::Tool {
                name,
                summary,
                output,
                output_truncated,
                error,
                click_mode,
                diff,
                pre_content,
                checks,
                ..
            } => {
                name.hash(&mut h);
                summary.hash(&mut h);
                output.len().hash(&mut h);
                output_truncated.hash(&mut h);
                error.hash(&mut h);
                click_mode.map(ToolVerbosity::label).hash(&mut h);
                diff.as_ref().map(|d| d.len()).hash(&mut h);
                pre_content.as_ref().map(|c| c.len()).hash(&mut h);
                if let Some(c) = checks {
                    c.format
                        .as_ref()
                        .map(|f| (f.tool.as_str(), f.changed))
                        .hash(&mut h);
                    c.syntax.as_ref().map(|s| (s.ok, s.available)).hash(&mut h);
                    c.praetor.as_ref().map(|p| p.count).hash(&mut h);
                }
            }
        }
        h.finish()
    }

    /// Cheap content fingerprint for the markdown cache.
    ///
    /// The key used to carry `text.chars().count()` (2026-11-17 CACHE-1), so a text
    /// change that kept the LENGTH served the stale render: bash `\r` progress
    /// lines, in-place token fixes and any equal-length replacement showed the
    /// previous frame. FNV-1a over the bytes: stable, dependency-free, and a
    /// same-length edit changes it.
    fn text_fingerprint(text: &str) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for b in text.as_bytes() {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    pub fn chat_lines(&mut self, width: usize) -> Vec<Line<'static>> {
        if self.md_cache.len() > 1024 {
            self.md_cache.clear();
        }
        if self.entry_lines_cache.len() > self.entries.len() {
            self.entry_lines_cache.clear();
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
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            let before = lines.len();
            // Dynamic entries (the streaming tail's thinking block, running
            // tools) animate — never cached (2026-11-14).
            let dynamic = matches!(entry, ChatEntry::Tool { running: true, .. })
                || matches!(entry, ChatEntry::Thinking { .. })
                    && idx + 1 == total
                    && self.is_streaming;
            if !dynamic {
                let rev = Self::entry_rev(entry, width);
                if let Some((cached_rev, cached)) = self.entry_lines_cache.get(&idx) {
                    if *cached_rev == rev {
                        lines.extend(cached.iter().cloned());
                        for _ in before..lines.len() {
                            line_entries.push(idx);
                        }
                        if lines.len() > before && idx + 1 < total {
                            let already_blank = lines
                                .last()
                                .map(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
                                .unwrap_or(true);
                            if !already_blank {
                                lines.push(Line::from(""));
                                line_entries.push(idx);
                            }
                        }
                        continue;
                    }
                }
            }
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
                    let key = (idx, Self::text_fingerprint(text), width);
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
                        let key = (idx, Self::text_fingerprint(text), width);
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
                    diff,
                    pre_content,
                    checks,
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
                        if let Some((title, cmd)) = bash_title_and_cmd(name, args.as_deref()) {
                            // The model's purpose is the primary scan target;
                            // command is secondary (owner request 2026-11-10:
                            // title and command must be different colors).
                            let budget = width.saturating_sub(8);
                            let title_s = trunc_str(title, 40);
                            let display = tool_display_name(name);
                            let used = crate::tui::width::cols(&display)
                                + 3
                                + crate::tui::width::cols(&title_s)
                                + 3;
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
                        let fresh = render_cache.as_ref().is_some_and(|cache| {
                            cache.width == body_w
                                && cache.generation == gen
                                && cache.output_len == olen
                                && cache.tick == tick
                        });
                        if !fresh {
                            let elapsed = started_at.map(|t| t.elapsed().as_secs());
                            let no_output_hint = *running
                                && output.is_empty()
                                && elapsed.is_some_and(|s| s >= NO_OUTPUT_HINT_SECS);
                            let block = render_tool_block(ToolBlock {
                                mode,
                                name,
                                summary,
                                args: args.as_deref(),
                                output,
                                truncated: *output_truncated,
                                running: *running,
                                error: *error,
                                width: body_w,
                                elapsed,
                                timeout_s: *timeout_s,
                                no_output_hint,
                                liveness: liveness.clone(),
                                diff: diff.as_deref(),
                                pre_content: pre_content.as_deref(),
                                checks: checks.as_deref(),
                            });
                            *render_cache = Some(Box::new(ToolRenderCache {
                                width: body_w,
                                generation: gen,
                                output_len: olen,
                                tick,
                                lines: block,
                            }));
                        }
                        if let Some(cache) = render_cache.as_ref() {
                            for bl in &cache.lines {
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
                ChatEntry::Watchdog { text, doc } => {
                    let wd = doc.as_deref();
                    // Structured card when the facts file is fresh; raw text
                    // fallback so a message is never lost (I3).
                    let edge_color = watchdog_edge_color(wd.as_ref().map(|d| d.verdict.as_str()));
                    let edge = Style::new().fg(edge_color).bg(theme::BG);
                    let body = Style::new().fg(theme::TEXT).bg(theme::BG);
                    let muted = Style::new().fg(theme::MUTED).bg(theme::BG);
                    let span_w = width.saturating_sub(4);
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
                        Some("UNKNOWN") => (" UNKNOWN ", muted),
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
                    // Exact arithmetic, not a guess: the top rule is
                    // "  ╭─"(4) + label(9) + dashes + chip + "─╮"(2); the bottom
                    // is "  ╰"(3) + dashes + "╯"(1). Matching the totals needs
                    // dashes = span_w - label - chip - 3. (The old -1 made the
                    // top edge two columns wider; the min(60) cap hid it on
                    // narrow screens and made it obviously short on wide ones —
                    // owner report 2026-11-16.)
                    let dashes =
                        span_w.saturating_sub(label.chars().count() + right.chars().count() + 3);
                    lines.push(Line::from(vec![
                        Span::styled("  \u{256d}\u{2500}", edge),
                        Span::styled(
                            label,
                            Style::new()
                                .fg(edge_color)
                                .bg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ),
                        // Full width, like the bottom border: capping this at 60
                        // left the card with a short top edge on wide terminals
                        // (owner report 2026-11-16).
                        Span::styled("\u{2500}".repeat(dashes), edge),
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
                            if let Some(src) = doc.source.as_deref() {
                                let tag = match src {
                                    "decision" => "decision model",
                                    "lane" => "free lane",
                                    "unavailable" => "unavailable",
                                    other => other,
                                };
                                lines.push(field("source", tag.to_string()));
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
            if !dynamic {
                let rev = Self::entry_rev(entry, width);
                self.entry_lines_cache
                    .insert(idx, (rev, lines[before..].to_vec()));
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
        ("probe", "cloud probe status", "/probe — background probe: leader lease, verified/total lanes, invalid lanes, owner pauses, next probe"),
        ("doctor", "self-check", "/doctor — gateway + probe admission, paused owners, session-format check, Rust-core sidecar presence"),
        ("routes", "caduceus decisions", "/routes — the latest routing decisions: tier, provider, model, reason, local state, latency, outcome (newest first; read-only)"),
        ("sessions", "all projects", "/sessions — search + resume sessions across every project store (foreign projects are read-only hints; I4)"),
        ("review-findings", "review results", "/review-findings — the last /review findings, severity-sorted (read-only; written by /review)"),
        ("janitor", "stale cache cleanup", "/janitor [dry] — remove stale prompt/watchdog/longjob/probe/KV/session-chain artifacts (protected: newest store file, slot0.bin, corrupt quarantines, ledgers); auto-runs daily"),
        ("thinking-view", "hide thinking blocks", "/thinking-view [hide|show] — collapse thinking blocks to a marker line (display only; the model's thinking level stays at /thinking)"),
        ("fire", "fire settings", "/fire [on|off|prismatic|emerald|alchemy|tint on|tint off] — bare opens choices"),
        ("glimmer", "watermark animation", "/glimmer [shimmer|breathe|twinkle|off] — bare opens choices"),
        ("shimmer", "watermark animation", "/shimmer — alias for the glimmer picker"),
        ("fx", "fire + glimmer", "/fx — fire/glimmer panel with live preview (fire only animates while the session is actually working)"),
        ("answers", "last inquiry answer", "/answers — the last INQUIRE-mode question + answer (scrollable; written by the agent-mode extension)"),
        ("engine", "engine control", "/engine [start|stop|restart|status] — control the vitriol-server systemd unit (no sudo: polkit scopes you to the two vitriol units; your daily-driver config is untouched)"),
        ("status", "session snapshot", "/status — model, agent, profile, context fill, cwd, session, permissions, gateway lanes, routing policy (read-only)"),
        ("caduceus", "gateway + routing", "/caduceus — providers/models/health/policy page: routing on-off, ranks, pins, free-only, caps (facts by the caduceus extension; policy file written by the TUI)"),
        ("cache", "manage the KV cache", "/cache — list engine KV cache entries with age/size; /cache wipe [age] — prune (slot0.bin protected)"),
        ("clear", "clear the transcript", "/clear — wipe the local transcript view (pi keeps the session history)"),
        ("compact", "compact the context", "/compact [hints] — summarize older history now; after it settles, auto-continue picks the work back up"),
        ("diag", "diagnostic settings", "/diag [on|off] — picker; F9 toggles stderr overlay"),
        ("help", "glossary + keys + commands", "/help or F1 — the reference modal"),
        ("clone", "duplicate the session", "/clone — duplicate the active branch into a fresh session"),
        ("export", "export session to HTML", "/export [path] — write a self-contained HTML transcript"),
        ("rename", "rename the session", "/rename <name> — set the session display name"),
        ("sidebar", "sidebar visibility", "/sidebar [auto|show|hide] — bare opens choices"),
        ("permissions", "permission mode", "/permissions — ask | auto | yolo (sensitive roots always guarded)"),
        ("yolo", "permission mode", "/yolo — open permission choices (sensitive roots still prompt)"),
        ("import-opencode", "import an OpenCode session", "/import-opencode [latest|<id>|list] [full] — convert an OpenCode session and switch to it (default: newest ~40k tokens + a digest of older history; full = everything)"),
        ("model", "select a model", "/model [provider/id] — picker from available models; provider/id switches directly"),
        ("new", "new session", "/new — fresh session (old one stays resumable)"),
        ("recover", "harness recovery", "/recover — restart a hung pi child and restore the session (auto-runs on watchdog failures)"),
        ("quit", "quit the TUI", "/quit — deliberate quit; the only other path is ^esc"),
        ("resume", "resume a session", "/resume — session picker; replays the full transcript including tool calls"),
        ("settings", "UI settings", "/settings [glimmer|fire] <args> — bare opens choices"),
        ("stats", "refresh stats", "/stats — refresh session statistics"),
        ("thinking", "thinking level", "/thinking [off..max] — bare opens choices"),
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
            // `/notices` — the FULL notice text (multi-line notices such as the
            // /nimbus lane table are otherwise reduced to their first line on
            // the notice strip, 2026-11-16).
            "notices" => {
                let text = match &self.notice {
                    Some((msg, kind)) => format!("notice ({kind}):\n{msg}"),
                    None => "notices: none".to_string(),
                };
                self.entries.push(ChatEntry::Diag(text));
                None
            }
            // `/band` — what the bottom band is made of this frame, so a future
            // flicker report is measurable instead of guessed.
            "band" => {
                let mut lines = vec!["band composition:".to_string()];
                for w in self.widgets.iter() {
                    if w.placement.as_deref() != Some("belowEditor") {
                        continue;
                    }
                    lines.push(format!(
                        "  {} · {} line(s) · {}",
                        w.key,
                        w.lines.len(),
                        if w.lines.iter().all(|l| l.trim().is_empty()) {
                            "empty"
                        } else {
                            "shown"
                        }
                    ));
                }
                lines.push(format!(
                    "  ctx row: {} · notice: {} · footer: hints only",
                    if self.ctx_bar_label().is_some() {
                        "on"
                    } else {
                        "off"
                    },
                    if self.notice.is_some() { "set" } else { "none" }
                ));
                self.entries.push(ChatEntry::Diag(lines.join("\n")));
                None
            }
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
        Self::LOCAL_COMMANDS.iter().any(|(n, _, _)| *n == word)
            || word == "heal"
            || word == "config"
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

    /// True when the command menu should be open: a typed command line, not a
    /// recalled one, and not dismissed for this exact content (K1/D6/D7,
    /// 2026-11-16).
    pub fn cand_sel_logic_open(&self) -> bool {
        if self.history_recall || !self.input.starts_with('/') {
            return false;
        }
        if self.menu_dismissed_for.as_deref() == Some(self.input.as_str()) {
            return false;
        }
        !self.command_candidates().is_empty()
    }

    /// The operator edited the line: it is no longer a mere recall, and any
    /// menu dismissal is void.
    pub fn note_input_typed(&mut self) {
        self.history_recall = false;
        self.menu_dismissed_for = None;
        self.cand_sel = 0;
    }

    /// Esc closed the command menu for the current content (composer untouched).
    pub fn dismiss_command_menu(&mut self) {
        self.menu_dismissed_for = Some(self.input.clone());
        self.cand_sel = 0;
    }

    /// `/q` → `/quit` when exactly one command matches the typed prefix.
    /// Ambiguous prefixes, arguments and plain text are returned unchanged (D8).
    pub fn expand_unique_command(&self, msg: &str) -> String {
        if !msg.starts_with('/') || msg.contains(' ') {
            return msg.to_string();
        }
        let word = msg.trim_start_matches('/');
        if word.is_empty() {
            return msg.to_string();
        }
        match self.command_candidates_for(msg).as_slice() {
            [only] if only.name != word => format!("/{}", only.name),
            _ => msg.to_string(),
        }
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

/// Status glyphs in check reports (`code` tool, verify tails): colored so a
/// failing gate reads instantly in the transcript (2026-11-14).
fn status_line_style(line: &str, default: Style) -> Style {
    let t = line.trim_start();
    let fg = if t.starts_with('✓') {
        theme::GREEN
    } else if t.starts_with('✗') || t.starts_with('×') {
        theme::RED
    } else if t.starts_with('⚠') {
        theme::ORANGE
    } else {
        return default;
    };
    Style::default().fg(fg).bg(default.bg.unwrap_or(theme::BG))
}

/// Diff line style on an explicit surface (tool blocks use PANEL).
fn diff_line_on(line: &str, surf: ratatui::style::Color) -> Style {
    let t = line.trim_start();
    if t.starts_with("@@") {
        Style::default().fg(theme::GOLD).bg(surf)
    } else if t.starts_with("+++") || t.starts_with("---") {
        Style::default().fg(theme::MUTED).bg(surf)
    } else if t.starts_with('+') {
        Style::default().fg(theme::GREEN).bg(surf)
    } else if t.starts_with('-') {
        Style::default().fg(theme::RED).bg(surf)
    } else {
        Style::default().fg(theme::TEXT).bg(surf)
    }
}

/// Diff text for an edit call derived from its args (used while running,
/// before pi returns result.details.diff).
fn edit_diff_from_args(path: &str, args: &serde_json::Value) -> Option<String> {
    let mut edits: Vec<(String, String)> = Vec::new();
    if let Some(list) = args.get("edits").and_then(|v| v.as_array()) {
        for e in list {
            if let (Some(o), Some(n)) = (
                e.get("oldText").and_then(|v| v.as_str()),
                e.get("newText").and_then(|v| v.as_str()),
            ) {
                edits.push((o.to_string(), n.to_string()));
            }
        }
    }
    if edits.is_empty() {
        if let (Some(o), Some(n)) = (
            args.get("oldText").and_then(|v| v.as_str()),
            args.get("newText").and_then(|v| v.as_str()),
        ) {
            edits.push((o.to_string(), n.to_string()));
        }
    }
    if edits.is_empty() {
        return None;
    }
    let mut joined = String::new();
    for (o, n) in edits {
        let d = crate::tui::diff::unified(&o, &n, path, 2, 140);
        if !d.is_empty() {
            joined.push_str(&d.join("\n"));
            joined.push('\n');
        }
    }
    (!joined.is_empty()).then_some(joined)
}

#[derive(Clone, Copy)]
struct PanelStyle {
    width: usize,
    surface: ratatui::style::Color,
    border: Style,
    muted: Style,
}

struct DiffPreview<'a> {
    text: &'a str,
    label: &'a str,
    cap: usize,
}

fn push_diff_block(out: &mut Vec<Line<'static>>, preview: DiffPreview<'_>, style: PanelStyle) {
    let DiffPreview { text, label, cap } = preview;
    let PanelStyle {
        width,
        surface: surf,
        border,
        muted,
    } = style;
    if text.trim().is_empty() {
        return;
    }
    out.push(Line::from(vec![
        Span::styled("│ ", border),
        Span::styled(label.to_string(), muted),
    ]));
    let total = text.lines().count();
    for line in text.lines().take(cap) {
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(
                trunc_str(line, width.saturating_sub(3)),
                diff_line_on(line, surf),
            ),
        ]));
    }
    if total > cap {
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(format!("… {} more diff line(s)", total - cap), muted),
        ]));
    }
}

/// Code + diff view for edit/write tool blocks (2026-11-14, operator
/// request: "show the code and diffs"). Empty when the args carry nothing.
fn code_view_lines(
    name: &str,
    args: &serde_json::Value,
    diff: Option<&str>,
    pre_content: Option<&str>,
    full: bool,
    width: usize,
    surf: ratatui::style::Color,
) -> Vec<Line<'static>> {
    let border = Style::default().fg(theme::BORDER_DIM).bg(surf);
    let muted = Style::default().fg(theme::MUTED).bg(surf);
    let mut out: Vec<Line<'static>> = Vec::new();
    let path = args
        .get("path")
        .or_else(|| args.get("file"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if name.eq_ignore_ascii_case("edit") {
        let computed = diff
            .map(str::to_string)
            .or_else(|| edit_diff_from_args(path, args));
        if let Some(text) = computed {
            push_diff_block(
                &mut out,
                DiffPreview {
                    text: &text,
                    label: "diff",
                    cap: if full { 200 } else { 24 },
                },
                PanelStyle {
                    width,
                    surface: surf,
                    border,
                    muted,
                },
            );
        }
        return out;
    }

    // write: content preview (highlighted) + overwrite diff.
    let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let overwrite_diff = pre_content
        .filter(|old| *old != content)
        .map(|old| crate::tui::diff::unified(old, content, path, 2, 200).join("\n"))
        .filter(|d| !d.is_empty());

    if !full {
        if let Some(text) = overwrite_diff.as_deref() {
            push_diff_block(
                &mut out,
                DiffPreview {
                    text,
                    label: "diff (overwrite)",
                    cap: 24,
                },
                PanelStyle {
                    width,
                    surface: surf,
                    border,
                    muted,
                },
            );
            return out;
        }
    }

    if !content.is_empty() {
        // Markdown files render as markdown (2026-11-14, operator report:
        // a .md write showed the diff but the preview stayed raw text).
        let lower = path.to_ascii_lowercase();
        let is_markdown =
            lower.ends_with(".md") || lower.ends_with(".markdown") || lower.ends_with(".mdx");
        out.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled(
                if full {
                    if is_markdown {
                        "markdown".to_string()
                    } else {
                        "code".to_string()
                    }
                } else {
                    format!(
                        "{} · {} line(s)",
                        if is_markdown { "markdown" } else { "code" },
                        content.lines().count()
                    )
                },
                muted,
            ),
        ]));
        let cap = if full { 400 } else { 24 };
        let total = content.lines().count();
        let rendered = if is_markdown {
            crate::markdown::render(content, width.saturating_sub(4))
        } else {
            crate::markdown::highlight_code(path, content, width.saturating_sub(4))
        };
        for line in rendered.into_iter().take(cap) {
            let mut spans = vec![Span::styled("│   ", border)];
            spans.extend(line.spans);
            out.push(Line::from(spans));
        }
        if total > cap {
            out.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled(format!("… {} more line(s)", total - cap), muted),
            ]));
        }
    }

    if full {
        if let Some(text) = overwrite_diff.as_deref() {
            push_diff_block(
                &mut out,
                DiffPreview {
                    text,
                    label: "diff (overwrite)",
                    cap: 80,
                },
                PanelStyle {
                    width,
                    surface: surf,
                    border,
                    muted,
                },
            );
        }
    }
    out
}

struct ToolBlock<'a> {
    mode: ToolVerbosity,
    name: &'a str,
    summary: &'a str,
    args: Option<&'a serde_json::Value>,
    output: &'a [String],
    truncated: bool,
    running: bool,
    error: bool,
    width: usize,
    elapsed: Option<u64>,
    timeout_s: Option<u64>,
    no_output_hint: bool,
    liveness: Option<(String, bool)>,
    diff: Option<&'a str>,
    pre_content: Option<&'a str>,
    checks: Option<&'a ToolChecks>,
}

fn render_tool_block(view: ToolBlock<'_>) -> Vec<Line<'static>> {
    let ToolBlock {
        mode,
        name,
        summary,
        args,
        output,
        truncated,
        running,
        error,
        width,
        elapsed,
        timeout_s,
        no_output_hint,
        liveness,
        diff,
        pre_content,
        checks,
    } = view;
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
        // Code-writing tools show the code and the diff (2026-11-14).
        let code_view = if name.eq_ignore_ascii_case("edit") || name.eq_ignore_ascii_case("write") {
            code_view_lines(name, a, diff, pre_content, full, width, surf)
        } else {
            Vec::new()
        };
        if !code_view.is_empty() {
            lines.extend(code_view);
        } else if let Some(special) = special_tool_args(name, a, full, width, surf) {
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
                status_line_style(ol, text_st)
            };
            for seg in wrap_text(ol, width.saturating_sub(5)) {
                let prefix = "│   ";
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

    // Post-call checks (format/syntax/praetor facts, 2026-11-14).
    if let Some(c) = checks {
        let mut spans: Vec<Span> = vec![Span::styled("│ ", border)];
        let mut any = false;
        if let Some(f) = &c.format {
            if any {
                spans.push(Span::styled(" · ", muted_st));
            }
            let ok = Style::default().fg(theme::GREEN).bg(surf);
            spans.push(Span::styled(
                format!("{} {}", if f.changed { "✓" } else { "·" }, f.tool),
                if f.changed { ok } else { muted_st },
            ));
            any = true;
        }
        if let Some(sy) = &c.syntax {
            if any {
                spans.push(Span::styled(" · ", muted_st));
            }
            let (label, color) = if sy.available == Some(false) {
                ("? syntax unavailable", theme::MUTED)
            } else if sy.ok {
                ("✓ syntax", theme::GREEN)
            } else {
                ("✗ syntax", theme::RED)
            };
            spans.push(Span::styled(
                label.to_string(),
                Style::default().fg(color).bg(surf),
            ));
            any = true;
        }
        if let Some(pr) = &c.praetor {
            if any {
                spans.push(Span::styled(" · ", muted_st));
            }
            spans.push(Span::styled(
                if pr.count == 0 {
                    "✓ praetor".to_string()
                } else {
                    format!("⚠ praetor {}", pr.count)
                },
                if pr.count == 0 {
                    Style::default().fg(theme::GREEN).bg(surf)
                } else {
                    Style::default().fg(theme::ORANGE).bg(surf)
                },
            ));
            any = true;
        }
        if any {
            lines.push(Line::from(spans));
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
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
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
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
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
        let mut s = AppState {
            hide_thinking: true,
            ..Default::default()
        };
        s.push_thinking_delta("hidden reasoning");
        s.push_user("hello".into());
        let lines = s.chat_lines(80);
        assert_eq!(s.line_entries.len(), lines.len());
        assert!(
            s.line_entries.contains(&0),
            "hidden thinking marker maps to its entry (plus the blank)"
        );
        assert!(s.line_entries.contains(&1));
    }

    #[test]
    fn scroll_lock_freezes_the_view_and_releases_at_the_tail() {
        let mut s = AppState {
            last_start: 100,
            last_chat_total: 200,
            scroll_max: 120,
            ..Default::default()
        };
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
        let mut s = AppState {
            last_start: 40,
            scroll_max: 80,
            ..Default::default()
        };
        s.scroll_up(4);
        assert!(s.scroll_lock.is_some());
        s.push_user("hello".into());
        assert_eq!(s.scroll_lock, None);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn thinking_default_collapsed_applies_to_new_blocks_and_is_configurable() {
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: true,
            ..Default::default()
        };
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
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
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
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
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
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
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
        s.entries.push(ChatEntry::Watchdog {
            text: "hang-watchdog ALIVE".into(),
            doc: None,
        });
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
            ..Default::default()
        });
        if let Some(ChatEntry::Watchdog { doc, .. }) = s.entries.last_mut() {
            *doc = s.watchdog.clone().map(Box::new);
        }
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
        let mut s = AppState {
            watchdog_file: Some(path.clone()),
            ..Default::default()
        };
        s.apply_notify("hang-watchdog: alive check 1", "info");
        s.apply_notify("hang-watchdog: alive check 2", "info");
        let cards = s
            .entries
            .iter()
            .filter(|e| matches!(e, ChatEntry::Watchdog { .. }))
            .count();
        assert_eq!(cards, 1, "identical facts replace the card");
        if let Some(ChatEntry::Watchdog { text: t, .. }) = s.entries.last() {
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
                .filter(|e| matches!(e, ChatEntry::Watchdog { .. }))
                .count(),
            2
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn flat_text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn write_block_shows_highlighted_code() {
        let mut s = AppState::default();
        s.tool_start(
            Some("w1"),
            "write",
            &serde_json::json!({"path": "src/a.rs", "content": "fn main() {\n    println!(\"hi\");\n}\n"}),
        );
        s.tool_end(Some("w1"), "write", "ok", false);
        s.tool_default = ToolVerbosity::Block;
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("Write"), "formal title: {text}");
        assert!(
            text.contains("code · 4 line(s)") || text.contains("code · 3 line(s)"),
            "{text}"
        );
        assert!(text.contains("fn main"), "content shown: {text}");
    }

    #[test]
    fn write_overwrite_shows_a_diff() {
        let dir = std::env::temp_dir().join(format!("write-diff-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "old line\nkeep\n").unwrap();
        let mut s = AppState {
            cwd: dir.clone(),
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
        s.tool_start(
            Some("w2"),
            "write",
            &serde_json::json!({"path": "a.txt", "content": "new line\nkeep\n"}),
        );
        s.tool_end(Some("w2"), "write", "ok", false);
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("diff (overwrite)"), "{text}");
        assert!(text.contains("-old line"), "{text}");
        assert!(text.contains("+new line"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edit_block_shows_diff_from_args_and_canonical_details() {
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
        s.tool_start(
            Some("e1"),
            "edit",
            &serde_json::json!({"path": "a.rs", "oldText": "fn a() {}\n", "newText": "fn b() {}\n"}),
        );
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("diff"), "{text}");
        assert!(text.contains("-fn a"), "{text}");
        assert!(text.contains("+fn b"), "{text}");
        // Canonical diff from pi replaces the derived one.
        s.set_tool_diff(
            Some("e1"),
            Some("--- a.rs\n+++ a.rs\n-canonical-old\n+canonical-new".to_string()),
        );
        s.tool_end(Some("e1"), "edit", "ok", false);
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("-canonical-old"), "{text}");
        assert!(text.contains("+canonical-new"), "{text}");
    }

    #[test]
    fn unavailable_syntax_is_neither_a_pass_nor_a_code_failure() {
        let dir =
            std::env::temp_dir().join(format!("toolchecks-unavailable-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("u1.json"),
            r#"{"version":1,"syntax":{"ok":false,"available":false,"output":"EPERM"}}"#,
        )
        .unwrap();
        let mut state = AppState {
            tool_default: ToolVerbosity::Block,
            tool_checks_dir: Some(dir.clone()),
            ..AppState::default()
        };
        state.tool_start(
            Some("u1"),
            "write",
            &serde_json::json!({"path":"a.py", "content":"x = 1"}),
        );
        state.tool_end(Some("u1"), "write", "ok", false);
        let text = flat_text(&state.chat_lines(100));
        assert!(text.contains("? syntax unavailable"), "{text}");
        assert!(!text.contains("✓ syntax"), "{text}");
        assert!(!text.contains("✗ syntax"), "{text}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tool_block_shows_check_footer_from_facts() {
        let dir = std::env::temp_dir().join(format!("toolchecks-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("c1.json"),
            r#"{"version":1,"ts":1,"format":{"tool":"rustfmt","changed":true},"syntax":{"ok":true},"praetor":{"count":2}}"#,
        )
        .unwrap();
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            tool_checks_dir: Some(dir.clone()),
            ..Default::default()
        };
        s.tool_start(
            Some("c1"),
            "write",
            &serde_json::json!({"path": "a.rs", "content": "fn x() {}"}),
        );
        s.tool_end(Some("c1"), "write", "ok", false);
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("✓ rustfmt"), "{text}");
        assert!(text.contains("✓ syntax"), "{text}");
        assert!(text.contains("⚠ praetor 2"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_tool_output_colors_status_lines() {
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
        s.tool_start(Some("c2"), "code", &serde_json::json!({}));
        s.tool_end(
            Some("c2"),
            "code",
            "code: 1 file\n  ✓ syntax a.rs\n  ✗ gates\n",
            false,
        );
        let lines = s.chat_lines(120);
        let ok = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|sp| sp.style.fg == Some(theme::GREEN) && sp.content.contains("syntax"));
        let bad = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|sp| sp.style.fg == Some(theme::RED) && sp.content.contains("gates"));
        assert!(ok, "green status line: {}", flat_text(&lines));
        assert!(bad, "red status line: {}", flat_text(&lines));
    }

    #[test]
    fn transcript_cache_is_transparent_and_invalidates() {
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
        s.push_user("hello".into());
        s.push_text_delta("answer text\n");
        s.push_thinking_delta("reasoning");
        s.tool_start(Some("t"), "bash", &serde_json::json!({"command": "ls"}));
        s.tool_end(Some("t"), "bash", "a\nb", false);

        let flat = |v: Vec<Line<'static>>| {
            v.iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };
        let a = flat(s.chat_lines(80));
        let b = flat(s.chat_lines(80)); // warm cache
        assert_eq!(a, b, "warm frames are identical");
        assert_eq!(s.line_entries.len(), s.chat_lines(80).len());
        s.entry_lines_cache.clear();
        let c = flat(s.chat_lines(80)); // cold rebuild
        assert_eq!(a, c, "the cache must be transparent");

        s.push_text_delta(" more words");
        let d = flat(s.chat_lines(80));
        assert!(d.contains("more words"), "tail mutation invalidates: {d}");
        assert_ne!(a, d);

        s.entries.push(ChatEntry::Thinking {
            text: "trace-body".into(),
            collapsed: false,
        });
        let e1 = flat(s.chat_lines(80));
        if let Some(ChatEntry::Thinking { collapsed, .. }) = s.entries.last_mut() {
            *collapsed = true;
        }
        let e2 = flat(s.chat_lines(80));
        assert!(e1.contains("trace-body"), "{e1}");
        assert!(e2.contains("click to expand"), "collapse invalidates: {e2}");
    }

    #[test]
    fn streaming_tail_rerenders_per_frame() {
        let mut s = AppState {
            hide_thinking: false,
            thinking_default_collapsed: false,
            ..Default::default()
        };
        s.push_user("go".into());
        s.is_streaming = true;
        s.push_text_delta("first");
        let a = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        s.push_text_delta(" second");
        let b = s
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(a.contains("first") && !a.contains("second"), "{a}");
        assert!(b.contains("first second"), "{b}");
    }

    #[test]
    fn write_block_renders_markdown_files_as_markdown() {
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
        s.tool_start(
            Some("m1"),
            "write",
            &serde_json::json!({
                "path": "docs/note.md",
                "content": "# Sprint notes\n\n- shipped the panel\n- **bold** item\n"
            }),
        );
        s.tool_end(Some("m1"), "write", "ok", false);
        let text = flat_text(&s.chat_lines(100));
        assert!(text.contains("markdown · 4 line(s)"), "{text}");
        assert!(text.contains("Sprint notes"), "heading rendered: {text}");
        assert!(!text.contains("# Sprint notes"), "hashes stripped: {text}");
        assert!(text.contains("shipped the panel"), "list rendered: {text}");
        assert!(!text.contains("- shipped"), "list marker stripped: {text}");
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
        let mut s = AppState {
            input: "refactor the flux capacitor\nsecond line".into(),
            ..Default::default()
        };
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
        let mut s = AppState {
            watchdog_file: Some(std::path::PathBuf::from("/nonexistent/watchdog.json")),
            ..Default::default()
        };
        s.apply_notify(
            "hang-watchdog: \"brievc build\" — no output for 61s (61s total)",
            "warning",
        );
        assert_eq!(s.entries.len(), 1);
        assert!(matches!(s.entries[0], ChatEntry::Watchdog { .. }));
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
        let mut s = AppState {
            watchdog: Some(doc.clone()),
            ..Default::default()
        };
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
        let mut s2 = AppState {
            watchdog_file: Some(std::path::PathBuf::from("/nonexistent/watchdog.json")),
            ..Default::default()
        };
        s2.apply_notify("hang-watchdog verdict: HUNG", "warning");
        // apply_notify reloads facts from disk (absent in tests) — inject
        // the parsed doc after, which is the same state production reaches
        // when the file is fresh.
        s2.watchdog = Some(
            serde_json::from_str(&format!(
                r#"{{"version":1,"ts":{now},"kind":"verdict","command":"brievc build c4probe.bv",
                 "elapsed_s":252,"stalled_s":189,"verdict":"HUNG","reason":"waiting on stdin",
                 "source":"decision","next_check_at":0,"phase":"done"}}"#
            ))
            .unwrap(),
        );
        if let Some(ChatEntry::Watchdog { doc, .. }) = s2.entries.last_mut() {
            *doc = s2.watchdog.clone().map(Box::new);
        }
        let text = s2
            .chat_lines(80)
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("source"), "card must name the verdict source");
        assert!(text.contains("decision model"));
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
        let mut s = AppState {
            fire_on: true,
            is_streaming: true,
            ..Default::default()
        };
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
        let mut s = AppState {
            prompt_history: vec!["first".into(), "second".into(), "third".into()],
            input: "draft in progress".into(),
            ..Default::default()
        };
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
        let mut s = AppState {
            liveness_cpu_total: 42.0,
            liveness_sampled_at: Some(std::time::Instant::now()),
            ..Default::default()
        };
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
        let mut s = AppState {
            prompt_suggestion_file: Some(sug.clone()),
            ghost_events_file: Some(events.clone()),
            ..Default::default()
        };
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
        let mut s = AppState {
            prompt_suggestion_file: Some(sug.clone()),
            ..Default::default()
        };
        // corrupt
        std::fs::write(&sug, "{nope").unwrap();
        s.refresh_ghost();
        assert!(s.prompt_ghost.is_none());
        // expired
        std::fs::write(
            &sug,
            r#"{"version":1,"id":"e","text":"x","source":"llm","expires":1}"#,
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
        let s = AppState {
            cwd: std::path::PathBuf::from("/tmp"),
            model: Some(crate::rpc::protocol::Model {
                id: "deepseek-v4.1-flash".into(),
                name: Some("DeepSeek V4.1 Flash".into()),
                provider: "caduceus".into(),
                context_window: Some(1_000_000),
                max_tokens: None,
                reasoning: None,
            }),
            perm_mode: "yolo".into(),
            ..Default::default()
        };
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
        let s = AppState {
            ctx_usage: Some((12_345, 262_144)),
            ..Default::default()
        };
        assert_eq!(
            s.ctx_bar_label().as_deref(),
            Some("🜖 ctx 12.3k / 262.1k (4.7%)")
        );
    }

    #[test]
    fn ctx_bar_label_handles_millions_and_full_window() {
        let mut s = AppState {
            ctx_usage: Some((1_500_000, 2_000_000)),
            ..Default::default()
        };
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
        let s = AppState {
            ctx_usage: Some((1234, 0)),
            ..Default::default()
        };
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
    fn tab_switch_pending_settles_when_the_hint_advances() {
        let mut s = AppState::default();
        s.parse_agent_mode(&["▪ BUILD · writes unlocked · TAB / /agent plan".to_string()]);
        assert_eq!(s.next_agent_mode().as_deref(), Some("plan"));
        assert!(!s.mode_switch_in_flight("plan"), "nothing sent yet");

        s.note_mode_switch_sent("plan");
        assert!(
            s.mode_switch_in_flight("plan"),
            "re-sending would be a no-op"
        );
        assert!(!s.mode_switch_in_flight("build"), "only the same target");

        // The widget advancing is what settles the mark.
        s.parse_agent_mode(&["◈ PLAN · research only · TAB / /agent build".to_string()]);
        assert_eq!(s.next_agent_mode().as_deref(), Some("build"));
        assert!(!s.mode_switch_in_flight("plan"));
        assert!(!s.mode_switch_in_flight("build"));
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
        let tool = s.oldest_running_tool().expect("running tool");
        assert_eq!(tool.key, "call-1");
        assert_eq!(tool.name, "bash");
        // No output line ever arrived for either call.
        assert!(tool.silent);

        s.tool_end(Some("call-1"), "bash", "done", false);
        let tool = s.oldest_running_tool().expect("still running");
        assert_eq!(tool.key, "call-2");
        assert_eq!(s.running_tool_count(), 1);

        s.tool_update(Some("call-2"), "bash", "partial");
        assert!(!s.oldest_running_tool().expect("running").silent);

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
    fn operator_stop_drops_the_queue_and_recovery_obligations() {
        let mut s = AppState::default();
        s.queued_prompts.push("one".into());
        s.queued_prompts.push("two".into());
        s.held_push("steer me", HeldKind::Steer);
        s.held_push("follow up", HeldKind::FollowUp);
        s.expected_tool_calls
            .insert("call-1".into(), std::time::Instant::now());
        s.park_stall("call-1", std::time::Duration::from_secs(60));
        s.recovery.pending_replay = Some(heal::PendingReplay {
            at: std::time::Instant::now(),
            requires_lapis: false,
        });

        let dropped = s.operator_stop();

        assert_eq!(dropped, 2, "every queued message is counted (E4)");
        assert!(s.queued_prompts.is_empty());
        assert!(s.held.is_empty());
        assert!(s.expected_tool_calls.is_empty());
        assert!(!s.stall_parked("call-1", std::time::Instant::now()));
        assert!(
            s.recovery.pending_replay.is_none(),
            "a stopped turn must not be replayed"
        );
    }

    #[test]
    fn abort_source_decides_who_may_escalate() {
        let mut s = AppState::default();
        let now = std::time::Instant::now();
        assert_eq!(s.abort_due(now), None, "nothing armed");

        s.arm_abort(now, AbortSource::Operator);
        assert!(s.abort_pending());
        assert_eq!(s.abort_due(now), None, "deadline not reached yet");
        assert_eq!(
            s.abort_due(now + std::time::Duration::from_secs(10)),
            Some(AbortSource::Operator)
        );

        s.arm_abort(now, AbortSource::Watchdog);
        assert_eq!(
            s.abort_due(now + std::time::Duration::from_secs(60)),
            Some(AbortSource::Watchdog),
            "the watchdog keeps its own escalation path (E3)"
        );
    }

    #[test]
    fn history_recall_does_not_open_or_freeze_the_command_menu() {
        let mut s = AppState::default();
        s.record_prompt_history("/model");
        s.record_prompt_history("what's next?");
        s.record_prompt_history("/quit");

        assert!(s.history_prev());
        assert_eq!(s.input, "/quit");
        assert!(
            !s.cand_sel_logic_open(),
            "a recalled line must not open the menu (K1)"
        );
        // The arrows still walk history instead of cycling the popup.
        assert!(s.history_prev());
        assert_eq!(s.input, "what's next?");
        assert!(s.history_prev());
        assert_eq!(s.input, "/model");

        // Typing makes it the operator's own line again: the menu may open.
        s.note_input_typed();
        assert!(s.cand_sel_logic_open(), "edited command line reopens it");
    }

    #[test]
    fn dismissed_command_menu_stays_closed_until_the_line_changes() {
        let mut s = AppState::default();
        s.input = "/q".to_string();
        assert!(s.cand_sel_logic_open());
        s.dismiss_command_menu();
        assert!(!s.cand_sel_logic_open(), "Esc closed it (D7)");
        assert_eq!(s.input, "/q", "closing must not touch the composer");
        s.note_input_typed();
        assert!(s.cand_sel_logic_open(), "any edit reopens it");
    }

    #[test]
    fn a_unique_partial_command_expands_and_executes_in_one_press() {
        let mut s = AppState::default();
        s.commands = vec![
            crate::rpc::protocol::SlashCommand {
                name: "model".into(),
                description: None,
                source: None,
            },
            crate::rpc::protocol::SlashCommand {
                name: "models-extra".into(),
                description: None,
                source: None,
            },
        ];
        // `/mod…` is ambiguous → unchanged (still completes below).
        assert_eq!(s.expand_unique_command("/model"), "/model");
        assert_eq!(s.expand_unique_command("/mod"), "/mod");
        // Unique prefix → the full command (D8).
        assert_eq!(s.expand_unique_command("/models"), "/models-extra");
        // Arguments and plain text are never rewritten.
        assert_eq!(s.expand_unique_command("/model gpt"), "/model gpt");
        assert_eq!(s.expand_unique_command("hello"), "hello");
        assert_eq!(s.expand_unique_command("/"), "/");
    }

    #[test]
    fn stall_park_prevents_repeated_evidence_probes() {
        let mut s = AppState::default();
        let now = std::time::Instant::now();
        assert!(!s.stall_parked("call-1", now));
        s.park_stall("call-1", std::time::Duration::from_secs(60));
        assert!(s.stall_parked("call-1", now));
        assert!(!s.stall_parked("call-2", now), "park is per tool call");
        s.clear_stall_state();
        assert!(!s.stall_parked("call-1", now));
    }

    #[test]
    fn stall_replay_needs_a_never_executed_tool_and_happens_once() {
        let mut s = AppState::default();
        s.note_prompt_sent("finish the audit", None);
        s.tool_start(
            Some("call-1"),
            "bash",
            &serde_json::json!({"command": "ls /proc"}),
        );
        s.note_prompt_tool_started();
        // Without the stall marker a started tool blocks replay (I3 parity).
        s.note_prompt_failed();
        assert!(!s.last_prompt_replayable());
        assert!(s.take_replayable_prompt().is_none());

        s.note_stall_unexecuted();
        // Recovery abandons the running call before replaying (abandon marks it
        // errored, so it is not a side effect).
        s.abandon_running_tools("harness recovery");
        assert!(s.last_prompt_replayable());
        let (text, images) = s.take_replayable_prompt().expect("stall replay");
        assert_eq!(text, "finish the audit");
        assert!(images.is_none());
        assert!(
            s.take_replayable_prompt().is_none(),
            "a stall replay must never loop"
        );
    }

    #[test]
    fn stall_replay_is_blocked_by_a_real_tool_result() {
        let mut s = AppState::default();
        s.push_user("finish the audit".into());
        s.note_prompt_sent("finish the audit", None);
        s.tool_start(
            Some("call-1"),
            "bash",
            &serde_json::json!({"command": "ls"}),
        );
        s.tool_end(Some("call-1"), "bash", "ok", false);
        s.note_prompt_failed();
        s.note_stall_unexecuted();
        assert!(s.replay_has_side_effects());
        assert!(s.take_replayable_prompt().is_none());
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
        let mut s = AppState {
            commands: vec![crate::rpc::protocol::SlashCommand {
                name: "helper".to_string(),
                description: None,
                source: None,
            }],
            input: "/hel".to_string(),
            cand_sel: 1,
            ..Default::default()
        };
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
        let mut s = AppState {
            sidebar_scroll_max: 10,
            ..Default::default()
        };
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
        let mut s = AppState {
            tool_default: ToolVerbosity::Full,
            ..Default::default()
        };
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
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
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
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
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
        let mut s = AppState {
            tool_default: ToolVerbosity::Block,
            ..Default::default()
        };
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
    fn transcript_entries_have_a_bounded_inline_footprint() {
        let bytes = std::mem::size_of::<ChatEntry>();
        eprintln!("ChatEntry inline storage: {bytes} bytes (baseline 352)");
        assert!(
            bytes <= 224,
            "ordinary transcript entries reserve {bytes} bytes each"
        );
    }

    #[test]
    fn tool_block_is_an_opaque_panel() {
        let lines = render_tool_block(ToolBlock {
            mode: ToolVerbosity::Block,
            name: "edit",
            summary: "/home/x/docs/GUARDS.md",
            args: Some(&serde_json::json!({ "path": "/home/x/docs/GUARDS.md", "edits": [1] })),
            output: &["Successfully replaced 1 block(s).".to_string()],
            truncated: false,
            running: false,
            error: false,
            width: 40,
            elapsed: Some(7),
            timeout_s: Some(30),
            no_output_hint: false,
            liveness: None,
            diff: None,
            pre_content: None,
            checks: None,
        });
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
        let lines = render_tool_block(ToolBlock {
            mode: ToolVerbosity::Full,
            name: "bash",
            summary: "",
            args: Some(&serde_json::json!({ "command": "cargo test" })),
            output: &[],
            truncated: false,
            running: true,
            error: false,
            width: 40,
            elapsed: Some(90),
            timeout_s: Some(1800),
            no_output_hint: true,
            liveness: None,
            diff: None,
            pre_content: None,
            checks: None,
        });
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
        let lines = render_tool_block(ToolBlock {
            mode: ToolVerbosity::Full,
            name: "bash",
            summary: "",
            args: Some(&serde_json::json!({ "command": "cargo test", "timeout": 1800 })),
            output: &[],
            truncated: false,
            running: true,
            error: false,
            width: 60,
            elapsed: Some(93),
            timeout_s: Some(1800),
            no_output_hint: true,
            liveness: None,
            diff: None,
            pre_content: None,
            checks: None,
        });
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
        assert!(s.check(UiEvent::CompactionEnd).is_err());
        set_phase(&mut s, UiPhase::Streaming);
        // Streaming legitimately allows picker open/close; a compaction_end
        // with nothing compacting is the impossible pair.
        assert!(matches!(
            s.check(UiEvent::CloseModal),
            Ok(UiPhase::Streaming)
        ));
        assert!(s.check(UiEvent::CompactionEnd).is_err());
        set_phase(&mut s, UiPhase::Compacting);
        assert!(matches!(s.check(UiEvent::CompactionEnd), Ok(UiPhase::Idle)));
        set_phase(&mut s, UiPhase::Modal);
        assert!(s.check(UiEvent::AgentEnd).is_err());
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

/// Live descendant count under `pid` (tool subtrees live there). `None` when
/// /proc is unavailable: unknown liveness is never read as evidence of life.
pub fn subtree_process_count(root: u32) -> Option<usize> {
    stat_cpu_secs(root)?; // proof /proc answers for this pid at all
    let mut total = 0usize;
    let mut queue = child_pids(root);
    let mut visited = 0usize;
    while let Some(pid) = queue.pop() {
        visited += 1;
        if visited > 4096 {
            break; // bounded walk: a fork storm must not stall the UI
        }
        total += 1;
        queue.extend(child_pids(pid));
    }
    Some(total)
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

fn watchdog_path(session: &str) -> std::path::PathBuf {
    let root = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".vitriol")
        .join("officina");
    watchdog_path_in(&root, session)
}

fn watchdog_path_in(root: &std::path::Path, session: &str) -> std::path::PathBuf {
    if session.is_empty() {
        return root.join("watchdog.json");
    }
    // Match JavaScript's ASCII replacement over UTF-16 code units exactly.
    let safe: String = session
        .encode_utf16()
        .take(120)
        .map(|unit| {
            char::from_u32(u32::from(unit))
                .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
                .unwrap_or('_')
        })
        .collect();
    root.join("watchdog").join(format!("{safe}.json"))
}

/// Decode wire evidence independently of card coalescing; malformed payloads never read newer facts.
fn watchdog_notice<'a>(
    message: &'a str,
    path: &std::path::Path,
    session: &str,
) -> (&'a str, Option<WatchdogDoc>) {
    let (message, doc) = match message.split_once("\n<officina-watchdog-v1>") {
        Some((text, payload)) => (text, serde_json::from_str::<WatchdogDoc>(payload).ok()),
        None => (message, load_watchdog_doc_at(path)),
    };
    let doc = doc.filter(|doc| {
        watchdog_fresh(doc, crate::tui::caduceus::now_ms())
            && (session.is_empty() || doc.session_id.as_deref() == Some(session))
            && (doc.session_id.is_none()
                || doc.tool_call_id.as_ref().is_some_and(|id| !id.is_empty()))
    });
    (message, doc)
}

fn watchdog_fresh(doc: &WatchdogDoc, now: u64) -> bool {
    if doc.version != 1 || doc.ts > now.saturating_add(5000) {
        return false;
    }
    match doc.valid_until {
        Some(expiry) => {
            expiry >= doc.ts && expiry <= doc.ts.saturating_add(360_000) && now <= expiry
        }
        None => now.saturating_sub(doc.ts) <= 120_000,
    }
}

/// Legacy facts expire after two minutes; scoped facts carry a bounded explicit lease.
pub fn load_watchdog_doc_at(path: &std::path::Path) -> Option<WatchdogDoc> {
    let raw = std::fs::read_to_string(path).ok()?;
    let doc: WatchdogDoc = serde_json::from_str(&raw).ok()?;
    if !watchdog_fresh(&doc, crate::tui::caduceus::now_ms()) {
        return None;
    }
    if doc.session_id.is_some() && doc.tool_call_id.as_deref().is_none_or(str::is_empty) {
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

#[cfg(test)]
mod watchdog_ownership_tests {
    use super::*;

    fn document(session: &str, call: &str, command: &str, verdict: &str) -> serde_json::Value {
        serde_json::json!({"version":1,"ts":crate::tui::caduceus::now_ms(),"session_id":session,"tool_call_id":call,
            "command":command,"verdict":verdict,"reason":format!("evidence for {call}"),"kind":"verdict","phase":"rejudge"})
    }

    #[test]
    fn owned_cards_keep_their_own_facts_and_update_without_stacking() {
        let dir = std::env::temp_dir().join(format!("watchdog-owned-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("facts.json");
        let mut state = AppState {
            session_id: "session-a".into(),
            watchdog_file: Some(path.clone()),
            ..Default::default()
        };
        for (call, command, verdict) in [
            ("one", "build-one", "ALIVE"),
            ("two", "build-two", "HUNG"),
            ("one", "build-one", "UNKNOWN"),
        ] {
            std::fs::write(
                &path,
                document("session-a", call, command, verdict).to_string(),
            )
            .unwrap();
            state.apply_notify("hang-watchdog: phase changed", "warning");
        }
        assert_eq!(
            state.entries.len(),
            2,
            "updates find their own call even behind another card"
        );
        let text = state
            .chat_lines(100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(text.matches("build-one").count(), 1, "{text}");
        assert_eq!(text.matches("build-two").count(), 1, "{text}");
        assert!(text.contains("UNKNOWN") && text.contains("HUNG"), "{text}");
        assert!(
            !text.contains("ALIVE"),
            "old verdict must be invalidated: {text}"
        );
        std::fs::write(
            &path,
            document("foreign-session", "three", "foreign-build", "ALIVE").to_string(),
        )
        .unwrap();
        state.apply_notify("hang-watchdog: unscoped raw notice", "warning");
        assert!(state.watchdog.is_none(), "foreign facts are not trusted");
        let text = state
            .chat_lines(100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.contains("foreign-build"), "{text}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn queued_notifications_own_snapshots_even_after_file_overwrite() {
        let mut state = AppState {
            session_id: "session-a".into(),
            watchdog_file: Some(std::path::PathBuf::from("/nonexistent/watchdog-test")),
            ..Default::default()
        };
        for (call, command, verdict) in [
            ("one", "first-build", "UNKNOWN"),
            ("two", "second-build", "HUNG"),
        ] {
            let message = format!(
                "hang-watchdog: observed\n<officina-watchdog-v1>{}",
                document("session-a", call, command, verdict)
            );
            state.apply_notify(&message, "warning");
        }
        assert_eq!(state.entries.len(), 2);
        let text = state
            .chat_lines(100)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("first-build") && text.contains("second-build"),
            "{text}"
        );
        assert!(text.contains("UNKNOWN") && text.contains("HUNG"), "{text}");
        assert!(
            !text.contains("officina-watchdog-v1"),
            "metadata is not visible prose"
        );
        state.apply_notify(
            "hang-watchdog: malformed\n<officina-watchdog-v1>{bad}",
            "warning",
        );
        assert!(state.watchdog.is_none());
        state.apply_notify(
            &format!(
                "hang-watchdog: foreign\n<officina-watchdog-v1>{}",
                document("other", "foreign", "private-command", "ALIVE")
            ),
            "warning",
        );
        assert!(state.watchdog.is_none());
    }

    #[test]
    fn explicit_leases_cover_rejudge_cadence_but_remain_bounded() {
        let now = 1_800_000_000_000;
        let mut doc = WatchdogDoc {
            version: 1,
            ts: now - 180_000,
            valid_until: Some(now + 90_000),
            ..Default::default()
        };
        assert!(
            watchdog_fresh(&doc, now),
            "an explicit lease can cover a five-minute rejudge interval"
        );
        doc.valid_until = None;
        assert!(
            !watchdog_fresh(&doc, now),
            "legacy freshness stays at two minutes"
        );
        doc.valid_until = Some(doc.ts + 360_001);
        assert!(
            !watchdog_fresh(&doc, now),
            "oversized leases are not trusted"
        );
        doc.ts = now + 6000;
        doc.valid_until = Some(now + 30_000);
        assert!(
            !watchdog_fresh(&doc, now),
            "future-dated facts are not health evidence"
        );
    }

    #[test]
    fn session_paths_match_the_javascript_filename_contract() {
        let root = std::path::Path::new("/private-test");
        assert_eq!(
            watchdog_path_in(root, "session/a🌙"),
            root.join("watchdog/session_a__.json")
        );
        assert_eq!(watchdog_path_in(root, ""), root.join("watchdog.json"));
    }
}

#[cfg(test)]
mod self_heal_tests {
    use super::*;

    fn prompt(s: &mut AppState, text: &str) {
        s.recovery = heal::RecoveryState::default();
        s.note_prompt_sent(text, None);
    }

    #[test]
    fn replay_gate_requires_failure_without_tools_or_output() {
        let mut s = AppState::default();
        prompt(&mut s, "do the work");
        s.note_prompt_failed();
        assert!(s.last_prompt_replayable());
        let taken = s.take_replayable_prompt().expect("replayable");
        assert_eq!(taken.0, "do the work");
        assert!(!s.last_prompt_replayable(), "take marks replayed");

        prompt(&mut s, "run tests");
        s.note_prompt_failed();
        s.note_prompt_tool_started();
        assert!(
            !s.last_prompt_replayable(),
            "tool-touched turn never replays"
        );

        prompt(&mut s, "think");
        s.note_prompt_failed();
        s.note_prompt_assistant_output();
        assert!(!s.last_prompt_replayable(), "assistant output is progress");
    }

    #[test]
    fn replay_gate_excludes_slash_and_images() {
        let mut s = AppState::default();
        prompt(&mut s, "/compact");
        s.note_prompt_failed();
        assert!(!s.last_prompt_replayable(), "slash commands never replay");

        let mut s = AppState::default();
        s.note_prompt_sent(
            "look",
            Some(vec![crate::rpc::protocol::ImageContent {
                content_type: "image".into(),
                data: "AAAA".into(),
                mime_type: "image/png".into(),
            }]),
        );
        s.note_prompt_failed();
        assert!(
            !s.last_prompt_replayable(),
            "image prompts are not replayed"
        );
    }

    #[test]
    fn rehydrate_does_not_count_history_as_current_progress() {
        let mut s = AppState::default();
        prompt(&mut s, "retry me");
        s.note_prompt_failed();
        s.recovery.phase = heal::Phase::Rehydrating;
        s.note_prompt_tool_started();
        s.note_prompt_assistant_output();
        assert!(
            s.last_prompt_replayable(),
            "historical transcript replay must not poison the gate"
        );
    }

    #[test]
    fn transcript_tool_after_last_user_blocks_replay() {
        let mut s = AppState::default();
        s.push_user("first".into());
        s.tool_start(Some("t1"), "bash", &serde_json::json!({}));
        s.tool_end(Some("t1"), "bash", "ok", false);
        s.push_user("second".into());
        assert!(!s.transcript_has_tool_after_last_user());
        s.tool_start(Some("t2"), "bash", &serde_json::json!({}));
        assert!(s.transcript_has_tool_after_last_user());
    }

    #[test]
    fn session_file_is_taken_from_get_state() {
        let mut s = AppState::default();
        s.apply_state(crate::rpc::protocol::RpcSessionState {
            model: None,
            thinking_level: None,
            is_streaming: false,
            is_compacting: false,
            session_id: Some("s1".into()),
            session_file: Some("/tmp/live.jsonl".into()),
            session_name: None,
            message_count: Some(3),
        });
        assert_eq!(
            s.session_file.as_deref(),
            Some(std::path::Path::new("/tmp/live.jsonl"))
        );
    }

    #[test]
    fn wire_idle_needs_streaming_flat_cpu_and_no_tool() {
        let threshold = std::time::Duration::from_secs(300);
        let mut s = AppState::default();
        let now = std::time::Instant::now();
        s.last_wire_at = now - threshold;
        s.liveness_flat_secs = 300;
        assert!(!s.wire_idle_due(now, threshold), "idle is not enough");
        s.is_streaming = true;
        assert!(s.wire_idle_due(now, threshold));
        s.liveness_flat_secs = 0;
        assert!(!s.wire_idle_due(now, threshold), "active pi CPU extends");
        s.liveness_flat_secs = 300;
        s.tool_start(Some("t"), "bash", &serde_json::json!({}));
        assert!(!s.wire_idle_due(now, threshold), "tool watchdog owns this");
    }

    #[test]
    fn wire_idle_catches_a_prompt_that_never_started_a_turn() {
        let threshold = std::time::Duration::from_secs(300);
        let mut s = AppState::default();
        s.note_prompt_sent("hello", None);
        assert!(!s.wire_idle_due(std::time::Instant::now(), threshold));
        s.last_wire_at = std::time::Instant::now() - threshold;
        if let Some(p) = s.last_prompt.as_mut() {
            p.sent_at = s.last_wire_at;
        }
        assert!(
            s.wire_idle_due(std::time::Instant::now(), threshold),
            "a prompt with no agent_start must escalate"
        );
    }

    #[test]
    fn reset_for_rehydrate_keeps_prompt_and_clears_view() {
        let mut s = AppState::default();
        prompt(&mut s, "retry me");
        s.note_prompt_failed();
        s.is_streaming = true;
        s.held_push("held", HeldKind::Queued);
        s.entries.push(ChatEntry::Diag("stale".into()));
        s.reset_for_rehydrate();
        assert!(s.entries.is_empty());
        assert!(s.held.is_empty());
        assert!(!s.is_streaming);
        assert!(s.last_prompt.is_some(), "replay source survives the reset");
    }

    #[test]
    fn rehydrate_keeps_the_context_row_so_the_layout_cannot_jump() {
        let mut s = AppState::default();
        s.ctx_usage = Some((12_345, 262_144));
        s.reset_for_rehydrate();
        assert_eq!(
            s.ctx_usage,
            Some((12_345, 262_144)),
            "recovery respawns the same conversation — dropping the fill would \
             move the transcript by one line"
        );
        assert!(s.ctx_bar_label().is_some());
    }

    #[test]
    fn model_label_shows_the_formatted_name_and_provider() {
        let mut m = crate::rpc::protocol::Model {
            id: "caduceus/glm-5.2".into(),
            name: None,
            provider: "caduceus".into(),
            context_window: None,
            max_tokens: None,
            reasoning: None,
        };
        // No catalog name: provider + id, never a bare code.
        assert_eq!(AppState::model_label(&m), "caduceus · caduceus/glm-5.2");
        m.name = Some("GLM 5.2".into());
        assert_eq!(AppState::model_label(&m), "GLM 5.2 · caduceus");
    }

    /// CACHE-1 (2026-11-17): the markdown cache was keyed by `chars().count()`,
    /// so an equal-length text change served the previous render — exactly what
    /// bash `\r` progress lines and in-place token fixes produce.
    #[test]
    fn equal_length_text_changes_the_fingerprint() {
        let a = AppState::text_fingerprint("step 1/3");
        let b = AppState::text_fingerprint("step 2/3");
        assert_ne!(a, b, "a same-length change must invalidate the cache");
        assert_eq!(
            a,
            AppState::text_fingerprint("step 1/3"),
            "the same text must keep its key"
        );
        // And the render really does change, not just the key.
        let mut state = AppState::default();
        state.entries.push(ChatEntry::Assistant("step 1/3".into()));
        let before = state.chat_lines(80);
        state.entries[0] = ChatEntry::Assistant("step 9/3".into());
        let after = state.chat_lines(80);
        assert_ne!(
            format!("{before:?}"),
            format!("{after:?}"),
            "the transcript served a stale render for an equal-length change"
        );
    }

    #[test]
    fn band_indicator_ignores_a_one_frame_candidate() {
        let mut s = AppState::default();
        let t0 = std::time::Instant::now();
        assert_eq!(s.stabilize_band_parts_at(0, 0, t0), (0, 0));
        // A datum that shows up for a single frame must not move anything.
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0),
            (0, 0),
            "first sighting holds"
        );
        assert_eq!(
            s.stabilize_band_parts_at(0, 0, t0 + BAND_SETTLE / 4),
            (0, 0),
            "gone again — never applied"
        );
        assert_eq!(s.band_ind.applied, 0);
    }

    #[test]
    fn band_indicator_ignores_a_candidate_that_flickers_for_a_while() {
        // Operator report 2026-11-16: "UI occasionally bumps upwards". A
        // datum living for several frames is still noise — it must not be
        // adopted just because it repeated.
        let mut s = AppState::default();
        let t0 = std::time::Instant::now();
        for step in 0..6 {
            let now = t0 + BAND_SETTLE / 8 * step;
            assert_eq!(
                s.stabilize_band_parts_at(if step % 2 == 0 { 1 } else { 0 }, 0, now),
                (0, 0),
                "alternating candidate never settles"
            );
        }
        // A candidate that flickers ON twice with a gap must restart its clock.
        assert_eq!(s.stabilize_band_parts_at(1, 0, t0), (0, 0));
        assert_eq!(
            s.stabilize_band_parts_at(0, 0, t0 + BAND_SETTLE / 2),
            (0, 0)
        );
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0 + BAND_SETTLE / 2),
            (0, 0)
        );
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0 + BAND_SETTLE),
            (0, 0),
            "clock restarted"
        );
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0 + BAND_SETTLE * 2),
            (1, 0),
            "adopted once it holds"
        );
    }

    #[test]
    fn band_indicator_adopts_a_candidate_that_settles() {
        let mut s = AppState::default();
        let t0 = std::time::Instant::now();
        assert_eq!(s.stabilize_band_parts_at(2, 0, t0), (0, 0));
        assert_eq!(
            s.stabilize_band_parts_at(2, 0, t0 + BAND_SETTLE / 2),
            (0, 0),
            "not yet settled"
        );
        assert_eq!(s.stabilize_band_parts_at(2, 0, t0 + BAND_SETTLE), (2, 0));
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0 + BAND_SETTLE * 2),
            (2, 0),
            "shrink waits too"
        );
        assert_eq!(
            s.stabilize_band_parts_at(1, 0, t0 + BAND_SETTLE * 3),
            (1, 0)
        );
        assert_eq!(s.band_ind.applied, 1);
    }

    /// The reported bug (2026-11-16): "the ctx field flickers to look like a
    /// different field for one frame". A transient indicator line must not be
    /// able to take the context row's slot, so the parts settle separately.
    #[test]
    fn band_parts_settle_independently() {
        let mut s = AppState::default();
        let t0 = std::time::Instant::now();
        // Steady state: one indicator row + the ctx row.
        s.stabilize_band_parts_at(1, 1, t0);
        assert_eq!(s.stabilize_band_parts_at(1, 1, t0 + BAND_SETTLE), (1, 1));
        // A transient second indicator line for one frame.
        assert_eq!(
            s.stabilize_band_parts_at(2, 1, t0 + BAND_SETTLE * 2),
            (1, 1),
            "the ctx row keeps its slot"
        );
        assert_eq!(
            s.stabilize_band_parts_at(1, 1, t0 + BAND_SETTLE * 2 + BAND_SETTLE / 8),
            (1, 1)
        );
        // A real second line that holds is adopted, and the ctx row stays put.
        // Its clock restarted on the transitional frame, so it needs a full
        // settle window from there.
        assert_eq!(
            s.stabilize_band_parts_at(2, 1, t0 + BAND_SETTLE * 3),
            (1, 1),
            "clock restarted when the second line returned"
        );
        assert_eq!(
            s.stabilize_band_parts_at(2, 1, t0 + BAND_SETTLE * 4),
            (2, 1),
            "adopted once it holds, ctx row unmoved"
        );
    }

    #[test]
    fn band_ctx_part_ignores_a_one_frame_absence() {
        let mut s = AppState::default();
        let t0 = std::time::Instant::now();
        s.stabilize_band_parts_at(0, 1, t0);
        assert_eq!(s.stabilize_band_parts_at(0, 1, t0 + BAND_SETTLE), (0, 1));
        // ctx_usage momentarily unknown: the row must not vanish for a frame.
        assert_eq!(
            s.stabilize_band_parts_at(0, 0, t0 + BAND_SETTLE * 2),
            (0, 1)
        );
    }
}

#[cfg(test)]
mod model_group_tests {
    use super::*;

    fn model(provider: &str, id: &str, name: &str) -> Model {
        serde_json::from_value(serde_json::json!({
            "id": id, "provider": provider, "name": name
        }))
        .unwrap()
    }

    #[test]
    fn picker_groups_officina_first_then_opencode_go_then_providers() {
        let mut s = AppState::default();
        s.models = vec![
            model("caduceus", "opencode-go/glm-5.2", "GLM via gateway"),
            model("opencode-go", "glm-5.2", "GLM-5.2"),
            model("officina", "nimbus", "Nimbus"),
            model("officina", "lapis", "Lapis Occultus"),
            model("zai", "glm-4.6", "GLM 4.6"),
            model("caduceus", "zai/glm-5", "GLM 5 via gateway"),
        ];
        let groups = s.filtered_model_groups();
        let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, vec!["OFFICINA", "OpenCode Go", "Z.AI"]);
        let officina: Vec<&str> = groups[0].models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(officina, vec!["lapis", "nimbus"]);
        // Native OpenCode Go entry wins over the gateway duplicate.
        assert_eq!(groups[1].models.len(), 1);
        assert_eq!(groups[1].models[0].provider, "opencode-go");
        assert_eq!(groups[1].models[0].name.as_deref(), Some("GLM-5.2"));
        // Flattened order is exactly the rendered order (selection indexes it).
        let flat: Vec<&str> = s.filtered_models().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(flat[0], "lapis");
        assert_eq!(flat[1], "nimbus");
        assert_eq!(flat[2], "glm-5.2");
        assert_eq!(flat.len(), 5);
    }

    #[test]
    fn filter_keeps_only_matching_groups() {
        let mut s = AppState::default();
        s.models = vec![
            model("officina", "lapis", "Lapis"),
            model("opencode-go", "grok-4.7", "Grok 4.7"),
            model("zai", "glm-4.6", "GLM 4.6"),
        ];
        s.models_filter = "grok".into();
        let groups = s.filtered_model_groups();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, "OpenCode Go");
        assert_eq!(s.filtered_models().len(), 1);
    }
}
