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
