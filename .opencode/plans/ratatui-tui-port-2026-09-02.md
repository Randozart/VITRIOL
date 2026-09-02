# Officina TUI Port: pi-tui → Ratatui

**Date**: 2026-09-02
**Status**: PLANNED → IN PROGRESS
**Owner**: VITRIOL

---

## Executive Summary

Replace pi-tui with Ratatui as Officina's TUI layer. pi-coding-agent's AI
agent logic stays intact — it runs as a subprocess via the existing JSONL
RPC protocol. Ratatui owns all rendering: sidebar, chat, editor, gauges,
dialogs. This gives us full terminal control, top-aligned sidebar, native
widgets, and eliminates the pi-tui viewport constraint that forced
bottom-anchoring.

### Why

1. **pi-tui has no sidebar concept.** Our sidebar is a hack: a two-column
   split rendered as a flat line array, bottom-anchored by exploiting
   pi-tui's "viewport = last N lines" behavior. Top-alignment is
   impossible — content at the top scrolls away as conversation grows.

2. **pi-tui's viewport is a sliding window.** `viewportStart = workingHeight - termHeight`
   means anything not at the tail of the output disappears. The sidebar
   must be at the END of the output to stay visible, which forces
   bottom-anchoring and upward growth.

3. **OpenCode and Crush use different frameworks** (opentui/SolidJS and
   Bubbletea/Lipgloss respectively) that have proper viewport/scrolling
   primitives. pi-tui doesn't.

4. **We own the whole stack.** pi-tui is a dependency we can replace. The
   AI agent logic (LLM calls, tool execution, session management,
   extensions) lives in pi-coding-agent, which has a production-ready
   RPC mode (`pi --mode rpc`) that communicates via JSON-line stdin/stdout.

5. **GLUE protocol (briev-compiler) is not suitable.** It's a compile-time
   FFI broker for Brief programs — generates static libraries via LLVM IR.
   We need runtime IPC between two live processes, not compile-time
   linking.

6. **NAPI embedding is impractical.** pi-coding-agent is a large ESM
   TypeScript package with deep Node.js dependencies (jiti, undici,
   photon-node, glob, chalk). Embedding Node.js in Rust via V8 bindings
   fights runtime initialization, native addon loading, and module
   resolution. Subprocess RPC gives the same API surface with none of
   that pain.

---

## Architecture

### Process Model

```
officina (Rust binary, main process)
│
├── Terminal I/O ─────────── crossterm (raw mode, events, resize)
├── TUI rendering ────────── ratatui (layout, widgets, gauges, dividers)
├── Async runtime ────────── tokio (subprocess, I/O, timers)
├── JSON protocol ────────── serde_json (serialize/describe RPC messages)
├── HTTP polling ─────────── reqwest (engine /metrics, /slots for live telemetry)
├── Engine telemetry ─────── polls llama-server directly (bypasses Node.js)
├── Disk readers ─────────── reads task-state + scratchpad files directly
│
└── Spawns: node dist/rpc-entry.js --mode rpc
    │
    └── pi-coding-agent (Node.js subprocess)
        ├── Agent loop (LLM streaming, tool execution)
        ├── All existing extensions load and run headlessly
        ├── session-panel emits extension_ui_request → Ratatui renders sidebar
        ├── vitriol-decode emits extension_ui_request → Ratatui renders gauges
        └── scratchpad/task-state write to disk → Ratatui reads directly
```

### What We Gain

1. **Top-aligned sidebar** — content starts at row 0, grows down. No
   bottom-anchoring, no padding hacks.
2. **Full terminal control** — no pi-tui viewport sliding window
   constraints.
3. **Native Ratatui widgets** — gauges, tables, lists, sparklines, block
   borders, styled text.
4. **Direct engine polling** — Rust polls /metrics and /slots directly via
   HTTP. No Node.js roundtrip for telemetry.
5. **Direct disk reads** — Ratatui reads task-state and scratchpad files
   directly. No Node.js roundtrip.
6. **Custom rendering** — coldBlue divider, section priorities, 42-cell
   truncation — all in Rust, no ANSI string manipulation.
7. **Performance** — Rust render loop at 60fps, no JS→ANSI→terminal
   overhead.

### What We Lose (temporarily)

1. **Extension hot-reload** — pi's extension loader runs in Node.js;
   Ratatui can't reload extensions without restarting the subprocess.
2. **Widget mode fallback** — the belowEditor widget fallback for narrow
   terminals is pi-tui-specific; Ratatui needs its own responsive layout.
3. **Overlay modals** — pi-tui's overlay system (used for /history scroll,
   dialogs) needs Ratatui equivalents (Clear + absolute positioning).

---

## Crate Structure

```
officina/
├── Cargo.toml            # workspace: officina (bin) + officina-native (cdylib, existing)
├── src/
│   ├── main.rs           # entry: parse args, spawn pi, run TUI loop
│   ├── rpc/
│   │   ├── mod.rs        # RPC bridge: spawn, JSONL framing, request/response correlation
│   │   ├── protocol.rs   # all RPC types: commands, events, responses (serde Deserialize)
│   │   └── bridge.rs     # RpcBridge: async spawn + send/receive + event dispatch
│   ├── tui/
│   │   ├── mod.rs        # TUI event loop (crossterm)
│   │   ├── state.rs      # App state: messages, sidebar sections, engine, tasks, scratchpad
│   │   ├── layout.rs     # OfficinaSplit: main column + sidebar column + divider
│   │   ├── sidebar.rs    # sidebar sections, gauges, dividers, truncation
│   │   ├── editor.rs     # input area (textarea or crossterm input)
│   │   ├── chat.rs       # message rendering (markdown, tool results, thinking)
│   │   └── input.rs      # keyboard/mouse/resize event handling
│   ├── engine/
│   │   ├── mod.rs        # engine telemetry poller (HTTP /metrics, /slots)
│   │   └── braille.rs    # braille gauge rendering (port from current Rust addon)
│   └── types.rs          # shared types (Model, SessionStats, ContextUsage, etc.)
├── native/               # existing NAPI addon (keep for backward compat, decouple later)
└── build.rs
```

### Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
reqwest = { version = "0.12", features = ["json"] }
anyhow = "1"
```

---

## RPC Bridge

### Protocol (existing, from pi-coding-agent)

- **Transport**: JSONL over stdin/stdout (LF-delimited)
- **Direction**: client→server on stdin, server→client on stdout
- **Correlation**: `"id"` field: `"req_N"` for commands, UUID for extension UI
- **Request**: `{"type":"<cmd>","id":"req_N", ...fields}`
- **Response**: `{"type":"response","command":"<cmd>","id":"req_N","success":true/false, "data":...}`
- **Events**: `{"type":"<event_type>", ...fields}` (no "id", no "response" type)
- **UI requests**: `{"type":"extension_ui_request","id":"<uuid>","method":"<method>",...}`
- **UI responses**: `{"type":"extension_ui_response","id":"<uuid>",...}`
- **Idle signal**: `{"type":"agent_settled"}`

### Commands (30+)

Prompting: `prompt`, `steer`, `follow_up`, `abort`
Session: `new_session`, `switch_session`, `fork`, `clone`, `set_session_name`
State: `get_state`, `get_messages`, `get_entries`, `get_tree`, `get_session_stats`
Model: `set_model`, `cycle_model`, `get_available_models`
Thinking: `set_thinking_level`, `cycle_thinking_level`, `get_available_thinking_levels`
Queue: `set_steering_mode`, `set_follow_up_mode`
Compaction: `compact`, `set_auto_compaction`
Retry: `set_auto_retry`, `abort_retry`
Bash: `bash`, `abort_bash`
Export: `export_html`

### Events

Lifecycle: `agent_start`, `agent_end`, `agent_settled`, `turn_start`, `turn_end`
Streaming: `message_start`, `message_update`, `message_end`
Tools: `tool_execution_start`, `tool_execution_update`, `tool_execution_end`
Queue: `queue_update`
Compaction: `compaction_start`, `compaction_end`
Bash: `bash_execution_update`
Extension: `extension_ui_request`, `extension_error`
Model: `model_select`

### Extension UI Sub-Protocol

Dialog methods (require response): `select`, `confirm`, `input`, `editor`
Fire-and-forget: `notify`, `setStatus`, `setWidget`, `setTitle`, `set_editor_text`

**Planned additions** (requires small patch to pi-coding-agent RPC mode):

| Method | Fields | Purpose |
|--------|--------|---------|
| `setSidebar` | `sections: [{id, priority, lines}]` | Replace all sidebar sections |
| `setSidebarSection` | `id, priority, lines` | Update one section |
| `extension_data` | `key: string, payload: any` | Arbitrary structured data |

---

## TUI Layout

```
┌──────────────────────────────────────┬───┬──────────────────────────────┐
│            MAIN COLUMN               │ │ │         SIDEBAR (42w)        │
│                                      │b│ │                              │
│  [chat messages, tool results,       │o│ │  ◈ Lapis Occultus – VITRIOL  │
│   thinking blocks, markdown]         │r│ │  Qwen3.8-27B-Q3_K_M        │
│                                      │d│ │  ─────────────────────────── │
│                                      │e│ │  ctx ⣿⣿⣿⣿⣿⣿ 72.3%     │
│                                      │r│ │       59.4k/82.0k           │
│                                      │ │ │  ing ⣿⣿⣿⣿⣿⣿ 45.2 tok/s │
│                                      │C│ │       12.3k tokens          │
│                                      │o│ │  eng ⣿⣿⣿⣿⣿⣿ 9.98 tok/s│
│                                      │l│ │       · 1/1 · 42.1k         │
│                                      │d│ │  · · · · · · · · · · · · ·  │
│                                      │B│ │  tasks [>] 1 · [ ] 3 · 5 dn│
│                                      │l│ │  note  2f 1l 0d · 3/60     │
│                                      │u│ │  files: src/main.rs +12 −3  │
│                                      │e│ │  · · · · · · · · · · · · ·  │
│                                      │ │ │  session · ~/VITRIOL        │
│                                      │ │ │    ↑12.3k ↓45.6k · 8 turns│
│                                      │ │ │  skills read · edit · bash  │
│                                      │ │ │  ref AGENTS.md · SESSION_LOG│
│                                      │ │ │  · · · · · · · · · · · · ·  │
│                                      │ │ │  /resume · /tree · /history │
├──────────────────────────────────────┼───┼──────────────────────────────┤
│  > [input area]                      │   │                              │
└──────────────────────────────────────┴───┴──────────────────────────────┘
```

- Sidebar is **top-aligned** (content starts at row 0, grows down)
- coldBlue `│` divider between columns (panel background behind it)
- Thick `──────` dividers between major groups
- Thin `· · ·` dividers between subsections
- Every line truncated to 42 cells (Rust-side, no ANSI wrapping)

---

## State Management

```rust
pub struct AppState {
    // Agent state (from RPC events)
    pub messages: Vec<AgentMessage>,
    pub is_streaming: bool,
    pub model: Option<Model>,
    pub context_usage: Option<ContextUsage>,
    pub session_id: String,
    pub session_name: Option<String>,

    // Sidebar sections (from extension_ui_request setSidebar)
    pub sidebar: Vec<SidebarSection>,

    // Engine telemetry (from direct HTTP polling)
    pub engine: EngineSnapshot,

    // Task summary (from disk: .pi/tasks/<session>.json)
    pub tasks: Option<TaskSummary>,

    // Scratchpad summary (from disk: .officina/SCRATCHPAD.md)
    pub scratchpad: Option<ScratchpadSummary>,

    // Input
    pub input: String,
    pub cursor: usize,
}
```

---

## Input Handling

| Key | Action |
|-----|--------|
| Enter | Send message (prompt) |
| Ctrl+C | Abort current generation |
| Ctrl+N | New session |
| Ctrl+S | Cycle model |
| Tab | Focus sidebar (scroll sections) |
| Esc | Unfocus sidebar |
| PgUp/PgDn | Scroll chat (when focused) |
| / | Enter command mode |

---

## Phased Implementation

### Phase 0: Frontend-Independent QoL Plumbing

**Goal**: Do all data-layer and shared-logic improvements now. These are
frontend-independent and ship immediately, improving the current Officina
while we build the Ratatui replacement.

Changes:
1. `_shared/engine.ts` — add `cumulativeIngest` to EngineSnapshot (cumulative
   prompt_tokens_total since boot)
2. `_shared/engine.ts` — poll `getContextUsage()` on engine update (not just
   `message_end`) to keep ctx section live during ingestion
3. `session-panel/index.ts` — context percentage: `pct.toFixed(1)` (1 decimal)
4. `session-panel/index.ts` — ctx line: use `/` instead of `of` (saves 2 cells)
5. `session-panel/index.ts` — model name on second line (dimGray, enriched only)
6. `session-panel/index.ts` — section dividers (thick between major, thin between minor)
7. `session-panel/index.ts` — scratchpad summary section (import getScratchpadSummary)
8. `session-panel/index.ts` — ingestion gauge section (cumulative tokens + rate)
9. `session-panel/index.ts` — every section truncated to 42 cells (defensive)
10. Native addon: coldBlue divider in gap column (Rust ansi.rs + JS fallback)
11. Vendor patch: update build-patch.mjs gap rendering

### Phase 1: RPC Bridge + Minimal TUI

**Goal**: Ratatui binary spawns pi-coding-agent, communicates via JSONL,
renders a basic chat interface.

- `officina/Cargo.toml` with ratatui, crossterm, tokio, serde_json, reqwest
- `rpc/protocol.rs` — all RPC types (serde Deserialize)
- `rpc/bridge.rs` — spawn, JSONL framing, request/response correlation
- `tui/state.rs` — AppState struct
- `tui/layout.rs` — basic two-column split
- `tui/chat.rs` — message rendering
- `tui/input.rs` — keyboard handling
- `main.rs` — wire it all together
- **Verify**: send a prompt, see streaming response in terminal

### Phase 2: Sidebar + Dividers

**Goal**: Full sidebar rendering in Ratatui with all sections.

- Port braille gauge rendering to standalone Rust module
- Implement sidebar layout (42w, top-aligned, coldBlue divider)
- Handle `setSidebar` extension_ui_request
- Section dividers (thick/thin)
- Context percentage, model name, all existing sections
- **Verify**: sidebar renders with all current sections

### Phase 3: Engine Telemetry + Ingestion

**Goal**: Live engine data in the sidebar.

- HTTP poller for /metrics, /slots (Rust reqwest)
- Port capacity/activity/mercury ramps
- Ingestion gauge + cumulative token counter
- Live context fill during prefill
- **Verify**: gauges update in real-time during generation

### Phase 4: Extension Data Integration

**Goal**: All sidebar data sources connected.

- Task summary from disk (.pi/tasks/<session>.json)
- Scratchpad summary from disk (.officina/SCRATCHPAD.md)
- Skills/knowledge from extension_ui_request events
- **Verify**: all sidebar sections populated

### Phase 5: Polish + Feature Parity

**Goal**: Full feature parity with current Officina.

- Input handling (vim keys, command mode)
- Session management (new/switch/fork/clone)
- Compaction UI
- Bash output streaming
- Extension dialog handling (select/confirm/input/editor)
- **Verify**: complete feature parity

### Phase 6: Decommission pi-tui

**Goal**: Remove pi-tui dependency.

- Remove OfficinaSplit patches from build-patch.mjs
- Remove vitriol-decode belowEditor widget (now in sidebar)
- Remove session-panel sidebar sections (now rendered by Ratatui)
- Clean up native addon (keep ansi.rs helpers, remove mergeSplitRows)
- **Verify**: officina still works for headless/print mode (Node.js RPC only)

---

## Extension Migration

| Extension | Current (pi-tui) | Future (Ratatui) |
|-----------|-------------------|-------------------|
| session-panel | `registerSidebarSection()` → `setSidebar(lines)` | Emits `setSidebar` extension_ui_request → Ratatui renders |
| vitriol-decode | `setWidget("vitriol-decode", lines)` | Emits widget data → Ratatui renders gauges in sidebar |
| task-state | `getTaskSummary()` (disk read) | Ratatui reads .pi/tasks/ directly |
| scratchpad | `getScratchpadSummary()` (disk read) | Ratatui reads .officina/SCRATCHPAD.md directly |
| skill-inject | `getRecentTools()` (in-memory) | Emits via `extension_data` event |
| knowledge-inject | `getLastTopics()` (in-memory) | Emits via `extension_data` event |
| agent-mode | Registers sections | Emits via `setSidebar` |
| plan-mode | Registers sections | Emits via `setSidebar` |

---

## Decision Record

| Decision | Rationale |
|----------|-----------|
| Subprocess RPC, not NAPI embedding | pi-coding-agent is a large ESM package with Node.js deps. Embedding Node.js in Rust fights runtime init. Subprocess gives same API surface. |
| GLUE protocol not suitable | Compile-time FFI for Brief programs. We need runtime IPC between live processes. |
| Ratatui, not custom ANSI | Ratatui is mature, has native widgets (gauges, tables, lists), handles terminal resize, differential rendering. |
| Keep pi-coding-agent as-is | AI agent logic is solid. We replace the rendering layer, not the brain. |
| Top-aligned sidebar | pi-tui's viewport forces bottom-anchoring. Ratatui gives full control. |
| ColdBlue divider | Visual separation between columns. Panel background behind the bar. |
| Direct HTTP for engine telemetry | Bypasses Node.js roundtrip. Rust polls /metrics directly. |
| Direct disk reads for tasks/scratchpad | Bypasses Node.js roundtrip. Extensions write to disk; Ratatui reads. |
| Phase 0 first | Frontend-independent improvements ship immediately. Validates sidebar layout decisions for Ratatui. |
