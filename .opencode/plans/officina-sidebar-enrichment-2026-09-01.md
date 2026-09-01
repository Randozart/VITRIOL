# Officina Sidebar Enrichment — Final Plan

## Status: Implemented — Change 2 REVERTED (see Decision Record below)
## Date: 2026-09-01

## INCIDENT RECORD (2026-09-01, late session): regen wiped four committed features

**Symptoms reported by owner**: no scrolling, sidebar grew unanchored with the
transcript, no mode recolor of the editor border.

**Root cause**: NOT the `officina` branch (the features were committed here:
40b13e1 mouse scrollback, 777930b mode-tinted border). The generated
`runtime/patched/interactive-mode.officina.js` carried FOUR hand-applied
patches never captured in `runtime/build-patch.mjs`:
P4 sidebar bottom-anchor, P7 scrollback (renumbered P10), P8 mouse
reporting/wheel, P9 dynamic mode tint. Every `node runtime/build-patch.mjs`
run this session regenerated the file from the pristine reference and
**silently wiped all four**.

**Fix (same day)**: all four ported into build-patch.mjs as anchored patches;
8 canary assertions added — the build now FAILS if any patch is missing from
the output. Rule written into AGENTS.md ("Vendor Patch Rule"): a patch that
exists only in a generated file is a patch waiting to be lost. Verification:
generated-file scroll/wheel/mouse marker count 19 (HEAD: 18),
`__officinaModeBorder` sites 3 (= HEAD), `node --check` passes.

## Decision Record (2026-09-01, owner-confirmed)

**Below-editor = live state; sidebar = session data.**

Change 2 ("move all below-editor widgets into the sidebar") was implemented,
then REVERTED after owner feedback: the bold BUILD/PLAN indicator recoloring
at the bottom next to the composer was the primary mode signal, and moving it
into a data panel hid it. Same for the VITRIOL telemetry gauges and the
gold plan/deep-research/phase indicators.

Final layout:
- **Below-editor widgets (restored, unconditional):** agent-mode (bold
  glyph+label, recolors on TAB), vitriol-decode (braille gauges + slots +
  tok/s), phase-model (`◇ plan: …`), plan-mode (gold `◆ PLAN MODE`),
  deep-research (gold `◇ DEEP RESEARCH`).
- **Sidebar (session-panel data rows only):** coupling, session title, ctx,
  engine numbers (eng/ing), tasks, files, session stats, skills, knowledge,
  command hints. Engine telemetry deliberately shows in BOTH places
  (gauges at bottom, precise numbers in sidebar) — owner choice.

Sidebar-only infra that SURVIVED the revert (still earning its keep):
- `_shared/sidebar.ts` registry (sections, listeners, render guard) —
  session-panel data rows use it; `createBelowEditorFallback` + visibility
  bridge kept as dormant infrastructure.
- Sidebar line cap raised from `MAX_WIDGET_LINES` (10) to terminal height.
- `onSidebarUpdate` multi-listener fix (Set, not single callback).
- Content-hash render guard in session-panel (`lastSidebarKey`) — idle
  sidebar renders dropped from ~86/min to ~0 (the biggest measured perf win
  of the session; see rust-migration-plan-2026-09-01.md).
- P5 tool display names (labels in TUI) and P7 native addon — unrelated to
  layout, unaffected.

Lesson recorded: "enrich the sidebar" ≠ "everything lives in the sidebar."
State feedback belongs where the eye is (composer); data belongs in the panel.

## Goal

Make the sidebar the single source of truth for session status. Move valuable info from below the editor into the sidebar. Fix tool call display names.

## Change 1: Tool Call Display Names

**Bug**: `ToolExecutionComponent` renders raw tool names (`update_tasks`) instead of labels (`Update Tasks`).

**Root cause**: Upstream `tool-execution.js` lines 107 and 305 use `this.toolName` instead of `this.toolDefinition?.label`.

**Fix**: Patch `tool-execution.js` (via `runtime/build-patch.mjs`) to use `this.toolDefinition?.label ?? this.toolName`.

Two locations:
- Line 107: `createCallFallback()` — fallback header when no custom renderer
- Line 305: `formatToolExecution()` — text formatting for tool execution

**File**: `runtime/build-patch.mjs` — add new P5 patch for tool-execution.js

## Change 2: Move Below-Editor Widgets Into Sidebar

Currently below-editor widgets (in order):
1. `agent-mode` — mode indicator (always present)
2. `vitriol-decode` — engine telemetry (always present)
3. `phase-model` — model phase summary (when active)
4. `plan-mode` — plan mode indicator (when toggled on)
5. `deep-research` — deep research indicator (when toggled on)

**Action**: Modify these 5 extensions to render into the sidebar via `ctx.ui.setSidebar()` instead of `ctx.ui.setWidget(..., { placement: "belowEditor" })`.

**Sidebar layout** (top to bottom):
```
╭────────────────────────────────────╮
│ ◈ Lapis Occultus · qwen3.6-35b    │  coupling
│ "session name"                     │  session title (if named)
│ ▪ BUILD MODE                       │  mode badge
│ ctx ▓▓▓░░░░░ 42% · 34k of 82k     │  context usage
│ eng ▓▓▓▓░░░░ 12.3 tok/s · 2/4    │  engine throughput
│ ing ▓▓▓▓▓░░░ 45.2 tok/s · +2.1k  │  ingestion (when active)
│ ── active status ──────────────────│  separator
│ ▪ BUILD MODE  writes unlocked     │  agent-mode (moved from below)
│ ◇ plan: qwen3.6-27b               │  phase-model (when active)
│ ◆ PLAN MODE (ctrl-q to exit)      │  plan-mode (when active)
│ ◇ DEEP RESEARCH (f2 to exit)      │  deep-research (when active)
│ ◈ VITRIOL  ▓▓░░ 2/4  12.3 tok/s  │  engine telemetry (moved from below)
│ ◈ ~/Projects/VITRIOL              │  session dir
│ ── session ────────────────────────│  separator
│ [>] 2/5 in progress · 3 done      │  task state
│ files: src/foo.ts +12 −3           │  files touched
│ session · ~/Proj  ↑340 ↓120 · 8   │  session stats
│ skills: read edit grep · 4 active  │  active skills
│ /resume · /tree · /history · /mode │  command hints
╰────────────────────────────────────╯
```

**Key decisions**:
- Agent-mode and vitriol-decode move FROM below-editor TO sidebar
- Phase-model, plan-mode, deep-research conditionally appear in sidebar when active
- Below-editor area becomes empty (or minimal footer) — sidebar is the info surface
- All existing env kill switches preserved (`OFFICINA_AGENT_MODE=0`, `VITRIOL_DECODE_WIDGET=0`, etc.)

### Files to modify:

| File | Change |
|---|---|
| `.pi/extensions/agent-mode/index.ts` | Render into sidebar instead of belowEditor widget |
| `.pi/extensions/vitriol-decode/index.ts` | Render into sidebar instead of belowEditor widget |
| `.pi/extensions/phase-model/index.ts` | Render into sidebar instead of belowEditor widget |
| `.pi/extensions/plan-mode/index.ts` | Render into sidebar instead of belowEditor widget |
| `.pi/extensions/deep-research/index.ts` | Render into sidebar instead of belowEditor widget |
| `.pi/extensions/session-panel/index.ts` | Add session title, task state, skills, knowledge rows; coordinate with other extensions |
| `.pi/extensions/task-state/index.ts` | Export `getTaskSummary()` |
| `.pi/extensions/skill-inject/index.ts` | Export `getRecentTools()` |
| `.pi/extensions/knowledge-inject/index.ts` | Export `getLastTopics()` |
| `runtime/build-patch.mjs` | Add P5 patch for tool-execution.js display names |

## Change 3: Session Title in Sidebar

**Source**: `pi.getSessionName()` — already registered by `session-name/index.ts`
**Display**: Below coupling name, gray/muted with quotes
**When**: Only show if session has a name set

## Change 4: Task State in Sidebar

**Source**: `.pi/tasks/<session>.json` via task-state extension
**Display**: `[>] 2/5 in progress · 3 done`
**Colors**: safety-green for done, mode-color for in_progress
**When**: Only show if tasks file has items

## Change 5: Active Skills in Sidebar

**Source**: `skill-inject.recentToolCalls` array
**Display**: `read edit grep · 4 active`
**Colors**: violet for tool names
**When**: Only show if any tool calls recorded this session

## Change 6: Knowledge Refs in Sidebar

**Source**: `knowledge-inject.lastSelectedTopics` array
**Display**: `BFS DFS · 2 injected`
**Colors**: solvent for topic names
**When**: Only show if knowledge entries were injected this session

## Implementation Order

1. **P5 patch** — tool-execution.js display names (standalone, no deps)
2. **Export functions** — task-state, skill-inject, knowledge-inject exports
3. **Session panel enrichment** — add new rows (session title, tasks, skills, knowledge)
4. **Move agent-mode** — from belowEditor to sidebar
5. **Move vitriol-decode** — from belowEditor to sidebar
6. **Move phase-model** — from belowEditor to sidebar
7. **Move plan-mode** — from belowEditor to sidebar
8. **Move deep-research** — from belowEditor to sidebar

Steps 4-8 can be done in parallel since they're independent extensions.

## Coordination Problem

Multiple extensions calling `ctx.ui.setSidebar()` will overwrite each other. Need a **shared sidebar line collector** — similar to how widgetContainerBelow works (each extension adds its widget, container renders all).

**Solution**: Create `_shared/sidebar.ts` with:
- `registerSidebarSection(id, renderFn)` — each extension registers its section
- `renderSidebar()` — collects all sections, sorts by priority, renders full sidebar
- `onSidebarUpdate(callback)` — notify session-panel when sections change

Session-panel becomes the **coordinator** that calls `ctx.ui.setSidebar()` with the combined output from all registered sections. Other extensions just call `registerSidebarSection()` and `requestSidebarUpdate()`.

**File**: `.pi/extensions/_shared/sidebar.ts` (new)
