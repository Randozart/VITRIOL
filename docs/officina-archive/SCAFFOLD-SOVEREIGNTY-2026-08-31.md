# Scaffold Sovereignty — replacing little-coder — 2026-08-31

**Status:** PROPOSED to owner (raised by owner in the 2026-08-31 session:
"I don't want to USE little-coder, I want to MINE from it"). This doc is
the decision record + execution plan. Amends THESIS-2026-08-31 (goal list
already included little-coder) and CRUSH-MINING-PLAN-2026-08-31 (mining
now has THREE upstreams: Crush, hermes-agent, little-coder).

## The decision

Trismegistus' daily driver becomes a **first-party scaffold**. The three
wrappers we currently drive through — little-coder, hermes-agent, and
(upstream) Crush — are demoted to mining sources. What we keep as engines:
VITRIOL (non-negotiable, the scheduling authority) and pi-coding-agent
(Apache-2.0 runtime; see §2 — the load-bearing decision).

## Why this is cheap now (dependency reality, verified 2026-08-31)

little-coder v1.18 is a thin distribution over `@earendil-works/
pi-coding-agent` 0.83.0 (Apache-2.0): pi owns the agent loop, tool
execution, extension host, session manager, model registry, LSP hooks,
skills loader. little-coder's own code is mostly the 46 extensions in
`.pi/extensions/` — and **11 of them are OURS already**, including every
piece the harness depends on:

| Ours (port directly — we hold the copyright) | Upstream's (mine selectively) |
|---|---|
| small-lane, rewind, vitriol-checkpoint, permissions-guard, hermes-bridge, tool-result-clearer, rtk-output, task-state, snapshot, repo-map, diagnostics-loop, context-relay, _shared (events/inject/turnkeys) | context-watchdog, subagent, deep-research, llama-cpp-provider (model probing), evidence/evidence-compact, plan-mode, thinking-budget, shell-session, quality-monitor, read/write-guards |

The architecture already routes around little-coder: the unified config
drives everything, `tris` owns lifecycle, the cockpit owns observability,
and events flow to OUR data plane. little-coder's remaining value-add over
raw pi is: bundling + a few small-model adaptations (whitepaper scaffold
work) — all mineable, none load-bearing.

## Target shape (name: alkahest — the solvent that binds the layers)

    trismegistus/
      scaffold/
        package.json        # depends on @earendil-works/pi-coding-agent (Apache-2.0)
        extensions/         # our 13, vendored from little-coder 1a6ee8b
        mined/              # ported upstream exts, each with provenance header
        index.mjs           # 100-line entry: load config -> register -> run

- `tris code` / `tris go` point at OUR scaffold, not `~/Projects/little-coder`.
- pi stays a LIBRARY dependency (Rule 9 fork policy: we depend on an
  Apache-2.0 upstream and pin it — no fork unless pi breaks us; a fork
  would be recorded per Rule 9 if it ever happens).
- little-coder, hermes-agent, Crush: mining sources only. New work (like
  today's M1) lands in trismegistus/scaffold/ directly.

## Honest cost accounting

- Porting ours: mechanical (import paths + config keys), 1 session.
- Mining upstream's: per-ext decision, watchdog/subagent/llama-cpp-provider
  first (they carry the small-model adaptations + auto n_ctx probing we
  rely on). The 812-test suite does NOT transfer automatically — pi's own
  tests + our per-ext tests are the new floor; budget a parity pass.
- Hermes (gateway) replacement is a SEPARATE, later decision — its plugin
  surface (vitriol-bridge, memory-extractor, caveman-rules) is ours but
  the gateway loop is upstream MIT. Not in this plan's scope; recorded so
  the endgame list is complete (thesis goal: replace Crush, hermes-agent,
  OpenCode, little-coder).

## Execution phases

1. **P1 — vendor:** trismegistus/scaffold/ + our 13 exts + entry; `tris
   code` switches via a config flag (`scaffold: alkahest | little-coder`)
   so the old path stays runnable during parity checks.
2. **P2 — mine the load-bearing three:** context-watchdog (+ the #68/#91/
   #108 compaction-loop guards), llama-cpp-provider (live n_ctx probe),
   subagent (dispatch surface). Each with provenance header + its tests.
3. **P3 — mine the rest opportunistically:** evidence*, plan-mode, guards,
   phase-model — only what dogfood proves missed.
4. **P4 — cutover:** `tris smoke` runs against alkahest; little-coder
   dependency deleted from docs/config defaults; ledger records the
   fingerprint parity run (same model, same tasks, tok/task compared).

Acceptance ("little-coder is replaced"): `grep -r little-coder ~/.config/trismegistus`
returns nothing; tris code/chat/go all run on alkahest; a full task set
shows tok/task parity or better vs the little-coder run (Rule 3 A/B).

## Relationship to the rest of the plan

- M1-M9 (Crush mining) continue unchanged — M1's logic ports with our exts.
- Tier A audit fixes are scaffold-agnostic and continue first (they guard
  measurement, which the parity A/B in P4 depends on).
- P4 cutover joins Tier C as a pre-dogfood gate: the dogfood day grades the
  harness we will actually keep, not a wrapper we plan to delete.
