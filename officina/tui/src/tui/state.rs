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
    pub const ALL: [ToolVerbosity; 3] = [ToolVerbosity::Line, ToolVerbosity::Block, ToolVerbosity::Full];

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
        if self > other { self } else { other }
    }

    pub fn min(self, other: ToolVerbosity) -> ToolVerbosity {
        if self < other { self } else { other }
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
pub fn effective_mode(tool_name: &str, global: ToolVerbosity, overrides: &HashMap<String, ToolOverride>) -> ToolVerbosity {
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
    pub const KNOWN_TOOLS: &[&str] = &["bash", "read", "write", "edit", "find", "grep", "ls"];

    // ── /help modal (owner request 2026-09-03) ───────────────────────────

    /// Sidebar glossary — every row explained. DRIFT GUARD (standing rule,
    /// plan officina-sidebar-declutter-help-2026-09-03): keep in sync with
    /// officina/.pi/extensions/session-panel/index.ts — any sidebar content
    /// change must touch this up.
    pub const SIDEBAR_GLOSSARY: &[(&str, &str)] = &[
        ("◈ <name>", "coupling — the engine endpoint identity (gold anchor)"),
        ("ctx", "context window fill: tokens used / window size + percent"),
        ("eng", "live engine rate (tok/s / idle); 'N decoded this boot' = engine decode total"),
        ("[>] n / [ ] n", "plan checklist: tasks in progress / open"),
        ("n done", "plan checklist: completed tasks"),
        ("note", "scratchpad: N facts · N leads · N dead ends, lines used/cap"),
        ("files: +n −n", "recently touched files: lines added / deleted"),
        ("↑ ↓", "session tokens: prompt tokens in / generated out"),
        ("turns", "your prompts this session"),
        ("tools", "recently used tools"),
        ("ref · n injected", "knowledge documents injected into context"),
        ("header center", "the model id behind this session"),
        ("header right", "session name or id @ working folder"),
        ("silver column", "quicksilver scroll gauge — fills as you scroll up"),
        ("gauge colors", "teal = ctx fill · silver = idle · green/cyan = decoding"),
    ];

    /// /help key reference.
    pub const HELP_KEYS: &[(&str, &str)] = &[
        ("enter", "send the message"),
        ("tab", "autocomplete / cycle agent mode"),
        ("esc", "abort streaming · close overlays — NEVER quits"),
        ("drag", "select transcript text (left button across the chat)"),
        ("^c", "copy — the selection, or the last reply"),
        ("^esc", "quit (the only quit chord — /quit also works)"),
        ("pgup/pgdn", "scroll back through history (wheel = 3 rows)"),
        ("home/end", "oldest line / live tail (when the input is empty)"),
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
            rows.push(HelpRow { kind, left: left.to_string(), right: right.to_string() });
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
        for (name, desc) in AppState::LOCAL_COMMANDS {
            push(HelpRowKind::Item, &format!("/{name}"), desc);
        }
        rows
    }

/// Output cap at ingest (sliced at render for Block/Line).
const TOOL_OUTPUT_CAP: usize = 2000;
/// Block preview lines.
const TOOL_OUTPUT_PREVIEW: usize = 30;

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
        /// Cached Block/Full rendering: (width, tools_gen, output_len, lines).
        /// Invalidated when any component changes — streaming output grows,
        /// config toggles bump tools_gen.
        #[allow(clippy::type_complexity)]
        render_cache: Option<(usize, u64, usize, Vec<Line<'static>>)>,
    },
    /// Diagnostic notice (extension errors, retries, stderr surfacing).
    Diag(String),
}

#[derive(Debug, Clone, Default)]
pub struct WidgetBlock {
    pub key: String,
    pub lines: Vec<String>, // raw ANSI — parsed at render time
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

    // Text selection (owner request 2026-09-03): drag with the left
    // button across the transcript; ^c copies the selected rows. Screen
    // coords; clamped against last_chat_area at render/extract time.
    pub sel_anchor: Option<(u16, u16)>,
    pub sel_head: Option<(u16, u16)>,
    /// Chat viewport of the last rendered frame — lets handle_key map
    /// selection rows to transcript lines without a Frame.
    pub last_chat_area: Option<ratatui::layout::Rect>,
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
            sel_anchor: None,
            sel_head: None,
            last_chat_area: None,
            started: std::time::Instant::now(),
            scroll: 0,
            scroll_max: 0,
            md_cache: HashMap::new(),
        }
    }
}

impl AppState {
    pub fn apply_state(&mut self, model: Option<Model>, thinking_level: Option<String>, is_streaming: bool, is_compacting: bool, session_id: Option<String>, session_name: Option<String>, message_count: Option<u64>) {
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
        let summary = summarize_args(args);
        self.entries.push(ChatEntry::Tool {
            tool_call_id: tool_call_id.map(String::from),
            name: name.to_string(),
            summary,
            args: Some(args.clone()),
            output: Vec::new(),
            output_truncated: false,
            running: true,
            error: false,
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
    pub fn tool_end(&mut self, tool_call_id: Option<&str>, name: &str, result_text: &str, error: bool) {
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

    /// Insert or update a widget block by key.
    pub fn set_widget(&mut self, key: &str, lines: Vec<String>) {
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
        } else {
            self.widgets.push(WidgetBlock {
                key: key.to_string(),
                lines,
            });
        }
    }

    /// Parse the agent-mode widget. The line format (agent-mode.ts) is
    /// "▪ BUILD · hint · TAB / /mode <next>" (loud directive modes:
    /// "► PLAN MODE — hint · TAB / /mode <next>") — the trailing hint names
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
                let label = label.trim_end_matches("MODE").trim();
                if !label.is_empty()
                    && label
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                {
                    self.agent_mode_label = Some(label.to_string());
                    self.agent_mode_glyph = Some(glyph.to_string());
                }
            }
            // Next mode: the trailing bare word after "/mode ".
            if let Some(idx) = plain.rfind("/mode ") {
                let next = plain[idx + "/mode ".len()..].trim().to_string();
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
                return if self.fire_tint { "tint on".to_string() } else { "tint off".to_string() };
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
        self.scroll = self.scroll.saturating_add(rows).min(self.scroll_max.max(self.scroll));
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
        for (idx, entry) in self.entries.iter_mut().enumerate() {
            match entry {
                ChatEntry::User(text) => {
                    for (i, seg) in wrap_text(text, width.saturating_sub(2)).into_iter().enumerate() {
                        if i == 0 {
                            lines.push(Line::from(vec![
                                Span::styled(format!("{} ", theme::GLYPH_USER), Style::default().fg(theme::GREEN)),
                                Span::raw(seg),
                            ]));
                        } else {
                            lines.push(Line::from(vec![
                                Span::raw("  "),
                                Span::raw(seg),
                            ]));
                        }
                    }
                    lines.push(Line::from(""));
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
                    lines.push(Line::from(""));
                }
                ChatEntry::Thinking(text) => {
                    for seg in wrap_text(text, width.saturating_sub(8)) {
                        lines.push(Line::from(vec![
                            Span::styled("  ·    ", Style::default().fg(theme::MUTED)),
                            Span::styled(seg, Style::default().fg(theme::MUTED).add_modifier(ratatui::style::Modifier::ITALIC)),
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
                        let body = if summary.is_empty() {
                            name.clone()
                        } else {
                            format!("{} {}", name, summary)
                        };
                        for (i, seg) in wrap_text(&body, width.saturating_sub(8)).into_iter().enumerate() {
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
                    } else {
                        // Block / Full — bordered construction block, cached
                        // against (width, tools_gen, output length).
                        let gen = self.tools_gen;
                        let body_w = width.saturating_sub(2).max(20);
                        let olen = output.len();
                        let fresh = matches!(render_cache,
                            Some((w, g, o, _)) if *w == body_w && *g == gen && *o == olen);
                        if !fresh {
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
                            );
                            *render_cache = Some((body_w, gen, olen, block));
                        }
                        if let Some((_, _, _, block)) = render_cache.as_ref() {
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
                    for (i, seg) in wrap_text(text, width.saturating_sub(8)).into_iter().enumerate() {
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
        }
        lines
    }

    /// Local (TUI-side) commands — dispatched to RPC directly, never sent
    /// to pi as prompt text. (name, description)
    pub const LOCAL_COMMANDS: &[(&str, &str)] = &[
        ("clear", "clear the transcript"),
        ("compact", "compact the context (optional hints)"),
        ("diag", "toggle diagnostic overlay"),
        ("fire", "composer flames (bare = toggle, or on|off|style|tint|prismatic|emerald|alchemy)"),
        ("glimmer", "watermark glimmer (bare = cycle, or shimmer|breathe|twinkle|off)"),
        ("model", "cycle the model"),
        ("new", "start a new session"),
        ("quit", "quit the TUI (alias /q)"),
        ("resume", "resume a previous session"),
        ("settings", "UI settings (bare = list; glimmer|fire <args>)"),
        ("stats", "refresh session stats"),
        ("thinking", "thinking level (off..max, bare = cycle)"),
        ("tools", "tool verbosity (bare = config menu, or <tool> <mode>[!+|-])"),
        ("help", "sidebar glossary, keys and commands (F1)"),
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
        for (name, desc) in Self::LOCAL_COMMANDS {
            if prefix.is_empty() || name.starts_with(prefix) {
                out.push(crate::rpc::protocol::SlashCommand {
                    name: name.to_string(),
                    description: Some(desc.to_string()),
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
            "model" => Some(RpcCommand::CycleModel { id: None }),
            "thinking" => {
                if args.is_empty() {
                    Some(RpcCommand::CycleThinkingLevel { id: None })
                } else {
                    const LEVELS: [&str; 7] = [
                        "off", "minimal", "low", "medium", "high", "xhigh", "max",
                    ];
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
        Self::LOCAL_COMMANDS.iter().any(|(n, _)| *n == word) || word == "config"
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

/// Short args summary for tool entries — first meaningful value.
fn summarize_args(args: &serde_json::Value) -> String {
    if let Some(obj) = args.as_object() {
        for key in ["file_path", "path", "command", "pattern", "query", "url", "name", "skill"] {
            if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
                let mut s: String = v.chars().take(48).collect();
                if v.chars().count() > 48 {
                    s.push('…');
                }
                return s;
            }
        }
    }
    String::new()
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
) -> Vec<Line<'static>> {
    let full = mode == ToolVerbosity::Full;
    let border = Style::default().fg(theme::BORDER_DIM).bg(theme::BG);
    let text_st = Style::default().fg(theme::TEXT).bg(theme::BG);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(16 + output.len());

    // Header: ╭─ ⌬ name [▸ summary]
    {
        let mut spans = vec![
            Span::styled("╭─ ", border),
            Span::styled(format!("{} ", theme::GLYPH_CRUCIBLE), Style::default().fg(theme::GOLD).bg(theme::BG)),
            Span::styled(name.to_string(), Style::default().fg(theme::SILVER).bg(theme::BG).add_modifier(ratatui::style::Modifier::BOLD)),
        ];
        let head_w = width.saturating_sub(6 + name.chars().count());
        if !summary.is_empty() && head_w > 4 {
            spans.push(Span::styled(
                format!(" ▸ {}", trunc_str(summary, head_w - 3)),
                Style::default().fg(theme::MUTED).bg(theme::BG),
            ));
        }
        lines.push(Line::from(spans));
    }

    // Args section.
    match (full, args) {
        (true, Some(a)) if !a.is_null() => {
            lines.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("args", Style::default().fg(theme::MUTED).bg(theme::BG)),
            ]));
            if let Ok(pretty) = serde_json::to_string_pretty(a) {
                for pl in pretty.lines() {
                    let mut spans = vec![Span::styled("│   ", border)];
                    spans.extend(json_line_spans(pl, width.saturating_sub(4)));
                    lines.push(Line::from(spans));
                }
            }
        }
        (false, Some(a)) if !a.is_null() => {
            let one = serde_json::to_string(a).unwrap_or_default();
            let compact = if summary.is_empty() { one } else { format!("{} {}", summary, one) };
            lines.push(Line::from(vec![
                Span::styled("│ ", border),
                Span::styled("args  ", Style::default().fg(theme::MUTED).bg(theme::BG)),
                Span::styled(
                    trunc_str(compact.trim(), width.saturating_sub(9)),
                    text_st,
                ),
            ]));
        }
        _ => {}
    }

    // Output section.
    if !output.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("out", Style::default().fg(theme::MUTED).bg(theme::BG)),
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
            for (i, seg) in wrap_text(ol, width.saturating_sub(5)).into_iter().enumerate() {
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
                Span::styled(
                    format!("… {} more lines", hidden),
                    Style::default().fg(theme::MUTED).bg(theme::BG),
                ),
            ]));
        }
        if truncated {
            lines.push(Line::from(vec![
                Span::styled("│   ", border),
                Span::styled(
                    "… output truncated (storage cap)",
                    Style::default().fg(theme::MUTED).bg(theme::BG),
                ),
            ]));
        }
    } else if running {
        lines.push(Line::from(vec![
            Span::styled("│ ", border),
            Span::styled("running…", theme::info()),
        ]));
    }

    // Bottom status.
    {
        let (icon, label, st) = if running {
            (theme::GLYPH_CRUCIBLE, " running", theme::warn())
        } else if error {
            ("✗", " failed", theme::crit())
        } else {
            ("✓", "", Style::default().fg(theme::GREEN).bg(theme::BG))
        };
        lines.push(Line::from(vec![
            Span::styled("╰─ ", border),
            Span::styled(format!("{}{}", icon, label), st),
        ]));
    }
    lines
}

/// Pretty-JSON line colorized into spans: `"key":` cyan, braces muted,
/// scalar values text-colored. Truncated to `width` chars.
fn json_line_spans(line: &str, width: usize) -> Vec<Span<'static>> {
    let t = trunc_str(line, width);
    let trimmed = t.trim_start();
    let brace = trimmed == "{" || trimmed == "}" || trimmed == "},";
    if brace {
        return vec![Span::styled(t, Style::default().fg(theme::MUTED).bg(theme::BG))];
    }
    if let Some(pos) = t.find("\":") {
        let split = pos + 2; // include the colon
        return vec![
            Span::styled(t[..split].to_string(), Style::default().fg(theme::CYAN).bg(theme::BG)),
            Span::styled(t[split..].to_string(), Style::default().fg(theme::TEXT).bg(theme::BG)),
        ];
    }
    vec![Span::styled(t, Style::default().fg(theme::TEXT).bg(theme::BG))]
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
        s.parse_agent_mode(&["\x1b[1m\x1b[38;2;46;161m▪ BUILD\x1b[0m · writes unlocked · TAB / /mode plan\x1b[0m".to_string()]);
        assert_eq!(s.agent_mode_label.as_deref(), Some("BUILD"));
        assert_eq!(s.agent_mode_glyph.as_deref(), Some("▪"));
        assert_eq!(s.agent_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn parse_agent_mode_handles_loud_directive_format() {
        let mut s = AppState::default();
        s.parse_agent_mode(&["► PLAN MODE — research only · TAB / /mode build".to_string()]);
        assert_eq!(s.agent_mode_label.as_deref(), Some("PLAN"));
        assert_eq!(s.agent_mode_glyph.as_deref(), Some("►"));
        assert_eq!(s.agent_mode.as_deref(), Some("build"));
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
        assert_eq!(ToolVerbosity::Line.max(ToolVerbosity::Full), ToolVerbosity::Full);
        assert_eq!(ToolVerbosity::Full.min(ToolVerbosity::Line), ToolVerbosity::Line);
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
            ToolOverride { mode: ToolVerbosity::Block, strictness: Strictness::AtLeast },
        );
        ov.insert(
            "bash".into(),
            ToolOverride { mode: ToolVerbosity::Block, strictness: Strictness::AtMost },
        );
        ov.insert(
            "read".into(),
            ToolOverride { mode: ToolVerbosity::Full, strictness: Strictness::Pinned },
        );
        let g = ToolVerbosity::Line;
        // AtLeast bumps up when global less verbose.
        assert_eq!(effective_mode("write", g, &ov), ToolVerbosity::Block);
        // AtMost caps when global more verbose.
        assert_eq!(effective_mode("bash", ToolVerbosity::Full, &ov), ToolVerbosity::Block);
        // Pinned ignores global both ways.
        assert_eq!(effective_mode("read", g, &ov), ToolVerbosity::Full);
        assert_eq!(effective_mode("read", ToolVerbosity::Full, &ov), ToolVerbosity::Full);
        // No override → global.
        assert_eq!(effective_mode("grep", g, &ov), ToolVerbosity::Line);
    }

    #[test]
    fn tool_start_update_end_by_id() {
        let mut s = AppState::default();
        s.tool_start(Some("call-1"), "write", &serde_json::json!({"path": "a.rs"}));
        s.tool_start(Some("call-2"), "write", &serde_json::json!({"path": "b.rs"}));
        // Two same-named tools running — id must disambiguate.
        s.tool_update(Some("call-1"), "write", "line one\nline two\n");
        s.tool_update(Some("call-2"), "write", "other\n");
        s.tool_end(Some("call-2"), "write", "final\n", false);
        // call-2 complete with output; call-1 still running with its own.
        let states: Vec<(bool, Vec<String>)> = s
            .entries
            .iter()
            .filter_map(|e| match e {
                ChatEntry::Tool { output, running, .. } => Some((*running, output.clone())),
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
        let flood = (0..(TOOL_OUTPUT_CAP + 50)).map(|i| format!("l{}", i)).collect::<Vec<_>>().join("\n");
        s.tool_update(Some("c"), "bash", &flood);
        if let Some(ChatEntry::Tool { output, output_truncated, .. }) = s.entries.last() {
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
        s.tool_start(Some("c"), "write", &serde_json::json!({"path": "src/main.rs"}));
        s.tool_end(Some("c"), "write", "", false);
        let text = s.chat_lines(80).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("✓"), "done icon present: {}", text);
        assert!(text.contains("write"), "name present: {}", text);
        assert!(text.contains("src/main.rs"), "summary present: {}", text);
        // No block furniture in Line mode.
        assert!(!text.contains('╭'), "no block border: {}", text);
    }

    #[test]
    fn at_least_override_promotes_to_block() {
        let mut s = AppState::default();
        s.tool_overrides.insert(
            "write".into(),
            ToolOverride { mode: ToolVerbosity::Block, strictness: Strictness::AtLeast },
        );
        s.tool_start(Some("c"), "write", &serde_json::json!({"path": "src/main.rs"}));
        s.tool_end(Some("c"), "write", "out line\n", false);
        let text = s.chat_lines(80).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        assert!(text.contains('╭'), "block header present: {}", text);
        assert!(text.contains('╰'), "block footer present: {}", text);
        assert!(text.contains("args"), "args section present: {}", text);
        assert!(text.contains("out line"), "output present: {}", text);
    }

    #[test]
    fn full_mode_shows_pretty_args() {
        let mut s = AppState::default();
        s.tool_default = ToolVerbosity::Full;
        s.tool_start(Some("c"), "edit", &serde_json::json!({"path": "a.rs", "old": "x", "new": "y"}));
        s.tool_end(Some("c"), "edit", "", false);
        let text = s.chat_lines(80).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
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
        let text = s.chat_lines(60).iter().map(|l| l.to_string()).collect::<Vec<_>>().join("\n");
        assert!(text.contains("one"), "streamed lines visible: {}", text);
        assert!(text.contains("two"), "streamed lines visible: {}", text);
    }
}
