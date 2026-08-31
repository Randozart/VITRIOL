# Officina Context-Efficiency Record — What We Built and Where It Came From

**Date:** 2026-08-31
**Status:** Working snapshot. Owner observation that motivated this record:
**"84k context feels practically limitless."** This document records why —
every mechanism in the pipeline, its provenance, and the research lineage —
so the design rationale survives code churn and future merges.

## 1. The core thesis

**Discard text, never information; and never ask the model for what an
algorithm can decide.** What is evicted from context is almost always
already "spent": a file read that became a successful edit, a test run whose
failures were fixed, a log whose errors were repaired. The pipeline keeps
*decision-relevant* state (recent results, errors, task state, mode) at high
density and pushes everything else to disk, summarizers, or deterministic
checkers. A fixed attention budget over 15k of live context beats 60k that
is 80% debris.

## 2. The context pipeline (layers, in order of action)

| stage | extension | what it does | provenance of the technique |
|---|---|---|---|
| entry filter | `rtk-output` | test/build/install output reduced to exit status + error lines + verbatim tail BEFORE entering context (60–90% reduction); full raw payload parked at `.pi/rtk/<id>.log`, referenced by path | RTK-style output filtering (owner research, OmniRoute §4.1 / REPORT-02 step 8) |
| read cap | pi core read-guard | caps large file reads at the source | upstream pi-coding-agent |
| ingested-content gate | `injection-guard` | filters browser/webfetch content before it enters context (also prompt-injection defense) | ported from trismegistus/hermes-plugins/injection-guard/guards.py @ 237e424 (owner-authored, MIT; OmniRoute §4.5 / REPORT-02 step 21) |
| structural prior | `repo-map` | tree-sitter symbol graph + PageRank; ~500 tok map replaces 5–10K tok of blind reads | Aider's repo-map technique (github.com/Aider-AI/aider) |
| knowledge preload | `knowledge-inject` | scores `skills/knowledge/*.md` (algorithm reference cards) against the prompt; top-k within budget | local knowledge_augment.py (owner) |
| skill cards | `skill-inject` | per-tool skill cards, deduped so identical blocks are never re-sent | original work |
| post-edit checks | `diagnostics-loop` | fast per-file syntax check after every edit; failures re-injected as a ~300 tok tail (check → auto-repair → re-check) | OpenCode §3.3 / REPORT-02 step 11 |
| post-edit checks (2026-08-31) | `format-gate` | canonical formatter per project; on-disk result declared canonical | original work — plan P1 |
| post-edit checks (2026-08-31) | `verify-contract` | JSON parse + duplicate-key detection in-process; TOML/YAML via python3 opt-in | original work — plan P4 |
| post-edit checks (2026-08-31) | `import-lint` | toolchain-free unused-import detection (.py/.js/.ts) | original work — plan P3 |
| loop breakers (2026-08-31) | `edit-churn` | SHA-1 (old→new) pair tracking; loop at 3 repeats, volume warn at 10 edits/file | original work — plan P2 |
| loop breakers (2026-08-31) | `diff-fidelity` | verifies a "successful" edit actually changed the file; flags silent no-ops | original work — plan P5 |
| eviction | `tool-result-clearer` | on the `context` event, tool results older than a keep-window are replaced by one-line stubs; errors and excluded tools (plan/todo/state) never cleared | Claude Code "context editing"/clear_tool_uses pattern (R2.1, REPORT-02 step 6) |
| orientation | `session-ledger` (2026-08-31) | one self-replacing line (msgs/ctx≈tok/edits/files) — replaces in place, never accumulates | original work — plan P6 |
| task memory | `task-state` | task list lives in `.pi/tasks/<session>.json`, re-injected from DISK each turn — survives compaction by construction | Claude Code TodoWrite pattern (R2.4 / REPORT-02 step 9) |
| snapshots | `snapshot` | per-turn git snapshots under `refs/trismegistus/turns/` (worktree untouched) | OpenCode §3.2 / REPORT-02 step 10 |
| compaction | `small-lane` | compaction summarized by mellum2 on :8287 (~11–12 t/s) instead of the 27B master | Crush's small-model-lane architecture (CRUSH-MINING-PLAN-2026-08-31.md) |
| compression | `caveman` | deterministic compressor on sub-coder reports and memory retrieval (never code/prompts/plans; dark by default `TRIS_CAVEMAN=1`) | ported from trismegistus/hermes-plugins/caveman-rules/compress.py (owner-authored, MIT) |
| isolation | `subagent` / `deep-research` | child coders burn their own context, return truncated reports | original work |
| long-term memory | `memory` + `memory-extractor` | owned memory store; regex-rule candidates → human curator queue (never auto-trusted: wrong facts are context poisoning) | hermes memory-extractor concept; port from trismegistus/hermes-plugins/memory-extractor (SS2b) |

### The tail-injection discipline (the KV-cache rule)

All per-turn guidance (skills, knowledge, mode directives, diagnostics)
travels as hidden `role: "custom"` messages at the conversation TAIL, never
as system-prompt edits. Editing the system prompt invalidates the entire
cached prefix — caught in production when 120k tokens re-churned
"for no reason" (cache-hunter finding, `docs/optimizations`). Small models
also weight recency most, so the tail is the *strongest* position, not a
compromise. See `_shared/inject.ts`.

### Mode governance

`agent-mode` (Plan/Build): TAB toggle, research directive rides every plan
turn, write gate blocks non-.md edits at the `tool_call` event, one-shot
build hint on the next turn (never `sendUserMessage` — that fires a turn).
Both states render bold in the widget and mirror as a badge in the
session-panel sidebar (owner request 2026-08-31).

## 3. The layout fork & presentation

- `runtime/hooks.mjs` + `runtime/patched/interactive-mode.officina.js`:
  module loader hook serves a patched `interactive-mode.js` (5k lines) with
  relative imports re-anchored to the pinned package — the whole import
  graph survives without rewriting. Child processes get the hook re-applied
  via `NODE_OPTIONS` (`runtime/register-hooks.mjs`). See
  `docs/LAYOUT-FORK-2026-08-31.md`. Docked layout: `OfficinaSplit`
  two-column render (chat | sidebar), sidebar BOTTOM-ANCHORED (2026-08-31:
  pi-tui's viewport is the bottom N lines of output; a top-anchored panel
  scrolls away once the transcript outgrows the screen).
- `session-panel` v3b: coupling, context gauge (braille), engine truth
  (throughput from `_shared/engine.ts` polling llama.cpp), files touched.
  Upstream bug found and documented: persistent `ui.custom` overlays break
  keyboard routing in pi 0.83.0 (PTY-reproduced) — panel renders as a
  widget/sidebar instead.
- `vitriol-decode` + `_shared/engine.ts`: live decode-rate status from the
  llama.cpp slots (Crush-grade status bar).
- Branding: `ensureBranding()` sets `piConfig.name = "officina"` in the
  pinned runtime's package.json (pi's native fork-rebrand hook, dist/config.js);
  re-applies after reinstall. Fork status also disables first-time-setup.
- `officina-header`: braille logo watermark on empty session (Vitriolum
  tint #1c2634 on #0d1117).

## 4. Runtime substrate

- pi-coding-agent 0.83.0 (Apache-2.0, @earendil-works) as a pinned library
  (First-Party Mandate, 2026-08-31; scaffold sovereignty:
  `docs/SCAFFOLD-SOVEREIGNTY-2026-08-31.md`).
- Engine: VITRIOL llama.cpp fork, Lapis Occultus coupling (qwen38-master);
  certified profiles in AGENTS.md (tq3_0 TurboQuant KV at 3.5 bpw,
  resident weights, VITRIOL_MODE=off for fitting quants).
- Vitriolum palette: single source in `_shared/vitriolum.ts`, mirrored from
  vitriol-tui/src/theme.rs and officina/theme/officina.json; a parity test
  fails the tree on drift.

## 5. Provenance / research lineage (summary)

| source | what was learned (nothing copied verbatim) |
|---|---|
| Claude Code (Anthropic) | context editing / tool-result clearing, externalized task list (TodoWrite), permission-gate UX |
| Aider (Apache-2.0) | tree-sitter + PageRank repo map |
| OpenCode | per-edit diagnostic loop (§3.3), per-turn git snapshots (§3.2) |
| Crush | small-model compaction lane; status-bar grade |
| RTK | entry-side command-output reduction |
| trismegistus/hermes-plugins (owner, MIT) | injection-guard, caveman compressor, memory-extractor (ported to TS, SS2b) |
| pi-coding-agent (Apache-2.0) | runtime; extension/event API (`tool_call` gate, `context` middleware, `before_agent_start` ride-alongs), fork rebrand hook |

House rule: every algorithm-bearing module carries a `PROVENANCE` header
naming inspiration, license, and what was *learned* (see `docs/provenance/`,
`docs/PROVENANCE.md`). Apache-2.0 tree; GPL sources are re-derived only.

## 6. Why 84k feels limitless (the arithmetic)

- Entry filtering removes 60–90% of command output before it costs a token.
- Clearing keeps only a small window of live tool results; everything older
  is a ~25-token stub.
- Per-turn injections are budget-capped (~300 tok diagnostics, ~500 tok
  repo-map, top-k knowledge) and deduped.
- The prefix is cached, so per-turn cost is only the tail delta.
- Compaction runs on the cheap lane before pressure becomes degradation,
  and state that must survive it is on disk, not in history.

Net effect: steady-state context is dominated by *recent, live* material
plus a few hundred tokens of injected state — so the effective horizon is
set by what the session is doing, not by the tokenizer.

Measured data point (owner session, 2026-08-31, late): **~20k live context
against ~200k cumulative offloaded** — a ~10:1 discard ratio, with live
context essentially flat while work continued. This is the single best
empirical validation of the pipeline to date.

## 7. Known limits (honesty section)

- bash is never parsed for mutations in plan mode (unreliable); governance
  there is directive + tool gate, belt+braces not theater.
- `import-lint` only claims "unused" (word-boundary counting), never
  "undefined" — that needs a real resolver.
- Caveman compressor is dark by default (owner arm: `TRIS_CAVEMAN=1`).
- Docked sidebar focus routing remains experimental (`OFFICINA_SIDEBAR_FOCUS_TEST=1`).
