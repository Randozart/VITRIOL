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

docs/SCAFFOLD-SOVEREIGNTY-2026-08-31.md (in the trismegistus archive),
AGENTS.md First-Party Mandate: upstreams (little-coder, hermes, OpenCode,
Crush) are mining sources only; pi-coding-agent is a pinned library, not
a shipped frontend; the HTTP API is the only engine surface.

## Standing TODO (native-programming-environment gap list)

1. small-lane A/B numbers (compaction on mellum2 vs 27B) — Rule 3 gate.
2. `vitriol lanes` GPU lane arbiter (crush-small :8287 vs master :8279).
3. Per-slot n_kv in /slots — makes the decode widget's busy truth exact
   (engine-side, cert-gated).
4. little-coder fallback removal (after parity ledger entry).
5. Gateway replacement: memory + chat currently ride hermes-agent
   (hermes-bridge, tris chat) — last non-VITRIOL runtime in the stack.
