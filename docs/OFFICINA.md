# VITRIOL Officina — the native coding workshop

**2026-08-31, owner decision:** Officina is folded into VITRIOL as the
engine's native programming environment. Source of truth:
`officina/` in THIS repo. The trismegistus repo is a historical archive
(Rule 14) — new work lands here.

## The flow

    vitriol serve          # engine up (any lane)
    cd ~/some/project      # workshop opens for THIS directory
    vitriol officina       # full-screen workshop, Vitriolum theme

`vitriol officina [args]` execs `officina/officina.mjs` (args pass
through to the scaffold CLI: `-p "one-shot"`, `-c` continue, `-r` resume).
OFFICINA_DIR overrides the location for non-standard installs.

## What's inside (officina/)

- `officina.mjs` — entry: pins pi-coding-agent (Apache-2.0 library),
  binds our extensions + theme, claims the terminal (alt-screen +
  OSC 11 background #0d1117, restored on every exit).
- `.pi/extensions/` — 24 extensions: our harness pieces (vitriol-decode
  live engine telemetry widget, small-lane compaction on the mellum2
  lane, rewind, vitriol-checkpoint, permissions-guard, tool pipeline,
  hermes-bridge) + mined load-bearing upstream (subagent, plan-mode,
  deep-research, skill/knowledge-inject, llama-cpp-provider).
- `theme/officina.json` — Vitriolum palette (BG #0d1117, PANEL #161b22,
  sovereignty gold, safety green, solvent cyan, substrate red).
- `skills/` — knowledge, protocols, tools reference cards.

## Session management

- Prior sessions: `/resume` picker, `/tree` navigator (per-project/cwd).
- `/history`: scrollable full transcript (↑↓ pgup/pgdn G q).
- `/panel`: right sidebar — modified files, tokens, model, session id.
- cwd in the terminal title + widget line at all times.

## Design law

docs/officina-archive/SCAFFOLD-SOVEREIGNTY-2026-08-31.md (local copy;
originals frozen in the trismegistus archive repo), First-Party Mandate: upstreams (little-coder, hermes, OpenCode,
Crush) are mining sources only; pi-coding-agent is a pinned library, not
a shipped frontend; the HTTP API is the only engine surface.

## Standing TODO (2026-08-31, owner dispositions)

1. ~~small-lane A/B~~ **DROPPED** by owner — no measurement required; the
   small lane is policy, not a claim.
2. `vitriol lanes` GPU lane arbiter — APPROVED, build next.
3. Per-slot n_kv in /slots — APPROVED, engine-side, cert-gated; makes the
   decode widget's busy truth exact.
4. little-coder fallback removal — APPROVED after a parity ledger entry.
5. Gateway replacement — APPROVED, next major build: memory, chat, and
   injection safety move off hermes-agent into Officina (hermes-bridge,
   tris chat, memory-extractor, caveman-rules, injection-guard are the
   seeds). Last non-VITRIOL runtime in the stack.
6. 27B cert run — APPROVED; owner has validated the harness on 27B and
   loves it ("we have something amazing").
7. Retention/GC + checkpoint consolidation — APPROVED.

## Engine vs Officina boundary (2026-08-31, owner question)

Rule: **VITRIOL owns what must be true of the MACHINE; Officina owns what
must be true of the WORK.** Currently in VITRIOL that should move or be
built Officina-side:

| Item | Where it lives now | Should be |
|------|--------------------|-----------|
| skills/ knowledge, protocols, tool cards | officina (correct) | Officina — agent procedure, not engine |
| unified harness config (safety.permissions, pipeline switches, budgets) | trismegistus archive config.yaml | Officina — policy is harness law |
| session ledger + events (ledger.jsonl, events.jsonl) | ~/.local/state/trismegistus | Officina (rename state dir ~/.vitriol/officina) |
| tok/task accounting, success rates, agent benchmarks (benchmark-profiles, deep_research_runs) | officina | Officina — measures the WORK; engine `bench` (llama-bench) stays |
| small-lane compaction POLICY (when/what to compact, mellum2 choice) | officina ext | Officina — engine only hosts a second server if asked |
| turn-key / rewind mapping, snapshot ext | officina | Officina — engine serves checkpoint bytes; turn semantics are harness (fixes F7 dual-writer) |
| permissions/tool gating, read/write guards, plan mode | officina | Officina — governance over the WORK |
| pymander reference-mind store | scripts/vitriol | candidate for Officina/gateway (memory domain) during #5 |
| autosave boot-restore | vitriol-autosave.service | split: slot serialization stays ENGINE; when-to-save/restore policy moves Officina |

Stay engine-side (never move): llama.cpp fork, DMA executor, KV/cache
managers, server + /slots /metrics /health /props + checkpoint endpoints,
cert suite, profiles, fingerprint emission, GPU lane scheduling, bench.

## Couplings (2026-08-31)

The default coupling is **Lapis Occultus – VITRIOL**: whatever settings
VITRIOL is running with, regardless of which model that is — the endpoint
is the coupling, the model behind it incidental. Alternative providers are
hot-swappable mid-session (pi.setModel preserves the conversation):
`/coupling` lists, `/coupling <id>` swaps, `/coupling lapis-occultus`
returns to the stone. Couplings come from
~/.vitriol/officina/couplings.json (shape: officina/couplings.example.json;
an "ascensus" euro-capped cloud-escalation entry is pre-drafted). This is
the groundwork for a built-in ascensus tool. The session panel's cordoned
top frame shows the current coupling name.

## Layout notes (2026-08-31)

- Composer pinned to the screen floor: the wrapper pre-pushes the render
  origin by rows-8, so a fresh session opens with the typing field at the
  bottom; content scrolls naturally once it outgrows the reserve
  (Crush/OpenCode behavior).
- Panel colors: the cordoned top frame uses Vitriolum accents on values
  only — gold coupling, solvent folder, safety-green token flow, violet
  files, muted labels/keys — never full-screen color wash.

## Bugfix log

- 2026-08-31 (1): editor untypeable at startup — the sidebar overlay
  captured keyboard focus. Fix: OverlayOptions.nonCapturing (the purpose-
  built flag), replacing the unfocus-on-show hack.
- 2026-08-31 (1b): typing STILL dead after nonCapturing — root cause
  found by PTY reproduction: the v2 (widget-based) panel never actually
  shipped; a failed file write left v1's focus-stealing overlay in the
  tree while the commit message claimed v2. v2 re-landed and verified by
  driving the real TUI in a pseudo-terminal — keystrokes now echo in the
  editor. LESSON: TUI bugs get PTY-reproduction tests before "fixed" is
  claimed. The ui.custom overlay approach is abandoned for the sidebar. Panel v2 renders via
  setWidget (structurally incapable of capturing input); /history stays
  an intentional modal. DOCKED-RIGHT sidebar (Crush/OpenCode layout)
  requires a horizontal layout primitive pi-tui lacks — next build item:
  vendor + fork the interactive-mode layout (editor pinned bottom is
  already pi's natural layout; the right column is the fork).
- 2026-08-31 (2): `vitriol officina` opened with a first prompt
  "officina" — the launcher's case never shifted, so the subcommand word
  stayed in "$@" and pi read it as a positional prompt. Fix: shift first.

### Session UI fix + agent modes (2026-08-31, evening)

- TYPING BUG ROOT CAUSE (PTY-proven): ANY persistent extension overlay —
  even with nonCapturing — breaks keyboard routing in pi-coding-agent
  0.83.0. Bisection: panel ext off = typing works; on = dead, with keys
  reaching neither the overlay nor the editor. Upstream finding; the
  session panel therefore stays on the widget system until the layout
  fork. (Earlier "fixed" claims had shipped stale v1 code through a
  failed file write — both mistakes now guarded by the PTY test.)
- agent-mode ext: /mode plan|build. Plan injects a research-first
  directive (cache-safe tail message) and blocks write/edit on non-.md
  targets at the tool_call gate with a reason; bash mutations are NOT
  parsed (documented limit — the directive covers them). Build removes
  the gate and injects a one-shot "writes allowed again" hint.
  Kill switch OFFICINA_AGENT_MODE=0.
- Panel v3b: cordoned, colored (gold coupling / solvent folder /
  safety tokens / violet files), widget-rendered above the editor,
  composer pinned at the screen floor.

### Watermark (2026-08-31)

officina-header ext: the VITRIOL braille logo (assets/braille-logo-80c.txt,
80 cols, 40 rows) renders watermark-style as the session header on an
untouched session — tinted #1c2634, a faint blue lift off the #0d1117
background. First user input starts the chat and the logo scrolls away
naturally (PTY-verified: 3200 braille cells at startup, gone from the
viewport once the conversation takes over). Missing asset = silently no
watermark. Kill switch OFFICINA_WATERMARK=0.

### TAB mode toggle (2026-08-31)

TAB toggles Plan/Build (default Build). PTY-verified both directions:
TAB shows the plan indicator + research directive, TAB again clears it.
Note: this reclaims the editor's autocomplete binding (tui.input.tab) —
deliberate tradeoff, the mode toggle is the more valuable binding here.

### Mode-switch inference bug (2026-08-31, owner-reported)

TAB/mode switch fired a model turn with the directive as its prompt —
the switch used pi.sendUserMessage, which STARTS A TURN. Fixed: mode
directives now ride along invisibly on the user's own next turn via the
before_agent_start event (plan directive rides every plan turn; the
build hint rides exactly once and is consumed). No turn is ever fired by
switching modes. Startup note: a one-time "keybinding conflict" banner
for TAB is informational — the extension binding wins.

### Conflict banner silenced (2026-08-31)

officina.mjs now idempotently ensures ~/.pi/agent/keybindings.json has
"tui.input.tab": [] (TAB unbound from autocomplete — claimed by the
agent-mode toggle). Merge-preserving: existing keybindings survive and an
owner-managed tui.input.tab entry is never touched. PTY-verified: startup
banner gone, typing intact.

### Fold-in completion (2026-08-31)

VITRIOL is now self-contained. The tris CLI moved to officina/cli
(~/.local/bin/tris retargeted; 41 tests green), the unified config
detached from the old repo into ~/.config/trismegistus/config.yaml (real
file), and the canonical design docs live in docs/officina-archive/.
The trismegistus repo (now private) is pure history — nothing on this
machine requires it anymore.

### SS1 + SS5 landed (2026-08-31)

- SS1: little-coder fallback removed from the workshop launcher; validate
  parity retargeted to officina models.json (shipped default merged with
  ~/.config/officina/models.json; legacy little-coder path is read-only
  fallback during transition); hermes custom_providers dropped from port
  parity. User model override migrated to ~/.config/officina/.
- SS5: officina/scripts/selfcheck.sh — external-path grep gate, vitest,
  typecheck, cli pytest, config + keybindings checks, opt-in --live
  engine smoke. Run before every release tag.
- Remaining: SS2 gateway fold-in, SS3 repo-map, SS4 state rename.

### SS2a — gateway fold-in phase one (2026-08-31)

Officina owns memory. New `memory` ext (provenance: successor to
hermes-bridge + hermes memory-extractor concept, trismegistus hermes-plugins
@ 237e424): store ~/.vitriol/officina/memory/{MEMORY,USER}.md, tools
memory_read / memory_write / memory_search (own-session JSONL scan),
per-turn hidden injection of facts via before_agent_start (cache-safe).
LIVE-VERIFIED: the 27B wrote a fact to MEMORY.md through memory_write.
hermes-bridge ext retired (its only consumer was the memory contract);
`tris chat` retired — `vitriol officina` is the conversation surface.
injection-guard ported to TS (provenance header cites the hermes guards.py
origin): log mode default, block opt-in via TRIS_GUARD_MODE, screening
browser/webfetch ingested text. Caveman-rules + memory-extractor ports
recorded as SS2b (dark features, no runtime dependency today). Gates:
typecheck, 407 tests, praetor PASS, live memory write + one-shot OK.

### SS2a-b — memory goes project-scoped (2026-08-31, owner direction)

Memory is programming memory: it belongs to the project. Two stores:
project cwd/.officina/MEMORY.md (default target of memory_write;
versionable, reviewable in diffs) and global
~/.vitriol/officina/memory/USER.md (owner facts that travel across
projects). Injection merges both, labeled. memory_search covers both
stores plus own-session files. Live-verified: project write landed in
/tmp/proj-x/.officina/MEMORY.md.
