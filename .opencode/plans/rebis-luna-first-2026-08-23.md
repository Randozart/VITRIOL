# REBIS routing inversion — Luna-first, Sol verifies

**Date:** 2026-08-23 12:50
**Status:** implementing
**Trigger:** user's first live hermes session routed 7 consecutive turns to
Sol (91–261 s each, ~1100 s total), then co-tenant killalls dropped the
session. Access-log evidence in shim-events; bias total — Luna used once.

## Root cause of the bias

The ladder sent kickoff/planning to Sol because Luna-direct kickoffs failed
in E2 — but E2 was Luna **unaudited**. The pipeline audit has since proven it
catches exactly those failures (hallucinated tools, malformed JSON, premature
finals — all caught live). Sol was guarding against a failure class the
pipeline already handles.

## New routing principle

**Luna drafts everything agentic; Sol verifies; Sol authors only on
demonstrated failure or pure reasoning.**

| turn | route | audit |
|---|---|---|
| tools attached — kickoff (first assistant turn) | Luna drafts | **full Sol audit** (gates the session) |
| tools attached — executor continuation | Luna drafts | schema-validation only (tool_calls parse = pass) |
| final answer after tool work | Luna drafts | **full Sol audit** (user rule: finals always) |
| flagged (short/repeated/degenerate) | Luna drafts | full Sol audit |
| no-tools plain chat | Sol direct | unchanged (bare-chat Luna degenerates) |
| escape hatches | forced | unchanged |

## Design decisions (defaults, user-approved direction)

- **Sol down during audit → ship Luna's draft unaudited** + distill marker
  `unaudited_sol_down` (availability-first; the draft is reviewable later).
  Raw connection errors no longer kill agent turns.
- **Kickoff audits always** initially; relax to sampling only if distill data
  shows Luna kickoffs passing consistently.
- Schema validation: every tool_call's `arguments` must parse as JSON;
  failure ⇒ treat as flags (nudge-retry once, then Sol correction).
- Sol audit cost is bounded by construction: constrained schema, temp 0,
  ≤2048 output tokens — vs 2–4k-token full generations on the old route.

## Expected effect

Session profile measured (311e366a): 7 Sol turns 91–261 s ≈ 1100 s total.
Projected Luna-first: drafts 5–15 s + audits 10–25 s where triggered ⇒
~3–7× session speedup, and Sol outages no longer block drafting — only
audits wait (503 path handles gracefully).

## Validation

- Live: kickoff → Luna draft + Sol audit (expect hallucination catch or
  clean pass); executor turn → schema-only fast path; Sol-kill during audit
  → ship-unaudited marker.
- Week of production traffic: correction-rate per route from distill data
  decides whether kickoff audits relax to sampling.

## Progress log

- 2026-08-23 12:50 — plan written; implementing.

## Progress log (cont.)

- 2026-08-23 13:20 — **Inversion implemented + live-validated.**
  - Kickoff: Luna draft → full audit → audit FAILED (2 missing) → Sol
    corrected → shipped with valid tool_calls. 162.8s (vs 228-260s Sol-
    authored; quality gate fired and held).
  - Executor turn exposed Luna's variance: draft alternates between valid
    tool_calls and prose narration. Schema gate + audit handled both safely.
    56-93s observed.
  - Malformed-args nudge removed (capability limit; escalation to Sol
    correction is the recovery).
  - Sol-down → ship-unaudited + distill marker implemented (availability-
    first per plan).
- Known residual: Luna executor-turn consistency is THE quality variable.
  Distill now records correction-rate per route — a week of traffic
  quantifies it, and the harvested corrections are exactly the SFT data
  that would train it out (D2 closure of the loop).
