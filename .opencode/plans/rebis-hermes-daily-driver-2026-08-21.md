# REBIS hermes daily driver — experiment ladder

**Date:** 2026-08-21 23:00
**Status:** executing
**Prior:** `.opencode/plans/rebis-phase2-4-agentic-2026-08-21.md` (2a COMPLETE)
**Goal:** Rebis pattern as daily driver under hermes-agent: Mellum2 speed +
Qwen3.8 oversight, with Qwen steering Mellum back on track when it
under-initiates tool calls or stops prematurely.

## Decisions (user-selected)

- Oversight triggers: flagged turns AND every Mellum final answer ("final return
  statements") — premature finals are exactly Mellum's failure mode.
- Streaming through shim: determine empirically in E1 before building proxy.
- Harness scope: hermes primary, opencode providers kept wired (trivial).

## E1/E2 — hermes baselines

- Wire hermes at Qwen :8279 and Mellum :8287 separately; same real task both ways.
- Record: TTFT, wall time, context consumption (harness prompt overhead), streaming
  usage (SSE or not), Mellum tool-call sufficiency failures (baseline rate to beat).
- Task: scratch-crate `push` transaction (sound spec, capacity invariant).

## E3 — Anticipatio probe

Cold vs warm TTFT on identical prefixes against Qwen (`cache_prompt=true`).
Gate: ≥40% TTFT reduction, else drop shadow prefill.

## E4 — rebis_shim.py (:8090)

Transparent dual-model proxy for hermes. Per-turn pipeline:
- route: model-field/size/endpoint heuristics → Mellum (mechanical) or Qwen (planning)
- steer: after Mellum responds, flags — no-tool-call while tools in flight,
  short/empty/repeated output, final answer emitted → Qwen constrained verdict
  `{"complete": bool, "missing_actions": [...], "tool_calls_needed": [...]}`
  incomplete ⇒ shim returns Qwen-authored continuation instead of Mellum's stop
- session state keyed by client session id; journal reuse from rebis.py
- modes: `route` / `steer` / `audit-all`; SSE passthrough complexity gated on E1

## E4b — provider wiring

- opencode.jsonc: `mellum-think` entry (:8287), clone qwen38-mtp pattern.
- hermes profiles: direct-Qwen, direct-Mellum, shim.

## E5 — acceptance battery

3 real VITRIOL Rust tasks × {Qwen-direct, Mellum-direct, shim-route, shim-steer}.
Metrics: completion, tool-call sufficiency (steer must beat Mellum-direct's
under-call rate), wall time, tokens-per-green + kill-recovery drill.
Report → `.opencode/plans/rebis-phase4-report.md` + EXPERIMENT_LOG.md.

## Daily-driver server config

Experimental servers gain `--context-shift --cache-reuse 256`; keep bounded
`--cache-ram`; explicit `n_predict` caps in provider configs.

## Progress log

- 2026-08-21 23:00 — plan written; starting E1.
- 2026-08-22 00:10 — **E1–E4 complete.** Headlines:
  - hermes needs ≥64k → servers relaunched at 65536 (Qwen resident fits; Mellum
    needs --n-cpu-moe 16 hybrid, decode 70→4.98 t/s).
  - E2 confirmed Mellum's under-initiation live (zero tool calls, hallucinated API,
    file untouched).
  - E3 Anticipatio FAILED gate (2.3% TTFT reduction) — fork LCP/checkpoint machinery
    eats prefix reuse. Deprioritized.
  - E4 **full-stack Rebis via hermes bash tool PASSED** (brain-authored Mandatum,
    loop accepted iteration 1, independent verification green). No shim needed for
    daily-driver v1.
  - Second OOM class found: 15 GB RAM box + --no-mmap staging collision during dual
    loads. Fix: mmap weights + staggered startups + --cache-ram 512/1024.
- Next session: rebis_shim steering (flagged/finals) as optimization; test-emitting
  invariants in Mandatum authoring guidance; Phase 4 battery on real VITRIOL tasks;
  opencode mellum provider entry (trivial); consider Mellum window bump now that
  VRAM headroom exists (3.6/8 GiB at 64k hybrid — try --n-cpu-moe 8 for speed).
- 2026-08-22 00:50 — **F2: Mellum PINNED at 64k works** (6.87/8 GiB, 70.2 t/s — SWA
  makes KV tiny; earlier estimate wrong). Best daily config = moe0. **F1:
  test-emitting invariants ACCEPTED** (f1-v7, iteration 3) + negative control caught
  by both layers. Five loop defects fixed (JSON newline sanitize, error digest,
  last-draft feedback on correction turns, id-based invariant checks + fuzzy
  fallback, joint-satisfiability spec lesson). Details EXPERIMENT_LOG.md.
- Next: Phase 4 battery on real VITRIOL tasks (servers currently pinned/64k/bounded);
  shim steering now optional — bash-tool delegation is the working v1.
- 2026-08-22 02:40 — **Battery S1 cell complete** after an incident-driven hardening
  round. Fragment guard + backups now protect workdirs; delta-protocol bake-off ran
  6 configurations; verdict = drafter-selection matrix (guide §0): Mellum for
  new/small files, Qwen for modifications, replace-mode protocol. compiler_only
  verify_mode added after auditor-hallucination observations. S1 accepted via loop:
  152/152 tests, 130s wall. Incident + full matrix in EXPERIMENT_LOG.md.
- Remaining battery: formal timing arms A/C on S1, S2 (vitriol-tui small task),
  H1 (hard task) packets, phase4 report.
- 2026-08-22 01:20 — **Phase 4 scoped** (user-selected): THREE arms — A: Qwen-direct
  via hermes, B: hermes→rebis loop, C: Mellum-direct + steering shim (requires
  building `libvitriol/rebis_shim.py` first). Tasks: 2 safe + 1 genuinely hard.
  Shim v1 scope: single-backend (:8090→Mellum) transparent proxy; flags = no-tool-
  call mid-flight / premature final / empty-short / repeated-content; steering =
  Qwen constrained verdict `{complete, missing_actions}` then nudge-Mellum-once,
  fallback Qwen-authored override. Streaming handled by buffering upstream and
  synthesizing SSE when the client asked for it. Session state keyed by first-user-
  message hash.
