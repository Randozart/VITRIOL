---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T17:38:48+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "0bbbbe8185a27ed91ae549263c1d2abdb2946402"
supersedes: []
superseded_by: []
related: []
---

# gpu0-decision-kintsugi-slots

## Purpose

Verify GPU0 decision serving, report actual router decisions, and assess multiple resident Kintsugi slots.

## Approach and alternatives

Keep existing cloud scoring transport as-is where applicable, add an explicit loopback chat backend and log proposal, applied order and completed lane independently. Measure the local 0.5B model and attempt bounded slot experiments without stopping the active unit.

- Use deterministic fallback ordering if the local model cannot meet the latency and decision-quality checks. Do not treat generated choices as calibrated probabilities.

## Scope and documentation

Caduceus router, local decision launcher, operator docs, and measured feasibility. Production Kintsugi stays outside experiment scope.

## Invariants

1. With the decision backend off, the router walks the same candidates. Even when enabled, a model can only reorder the eligible fallback tail; a pinned requested lane remains first.
2. The local decision transport is loopback-only, sends no user text or API keys, and may abstain. Invalid replies or timeout preserve the existing candidate order. Capture the allowed and malformed fixture cases.
3. Decision reports distinguish consultation, changed order and eventual successful lane. A failed, abstained or memoized consultation cannot appear to be a fresh applied route. Use fixtures that return success on the pinned lane, then separately on a fallback.
4. GPU experiments have unique ports and process ownership and preserve the active system engine. A two-slot claim requires simultaneous requests and reported slot IDs, not only a successful load.

## Verification

- Router fixtures, real GPU0 constrained response and timings, negative/outage checks, source-bound repository verification and bounded Kintsugi tests where VRAM permits.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T17:38:48+01:00 — Plan created before implementation.

- 2026-11-26T17:48:06+01:00 — GPU0 had 11002 MiB free and GPU1 237 MiB free with the current engine. The Qwen2.5-0.5B Q4_K_M endpoint used about 559 MiB on GPU0. Eight constrained replies per model: Qwen 2/8 expected (118.85 ms median), Gemma 3 1B 4/8 (276.7 ms median). Both models always chose the same index. Do not enable local model-driven routing. A real local backend plus a fixture gateway verified decision vs served-lane reporting. With no drafter, two 8192-window slots were simultaneously busy on GPU0, generated 128 tokens each from 14 and 11 filled prompt tokens, and reached 13.05 tok/s each. The all-GPU draft load failed allocating 1033.21 MiB. The 55-GPU-layer DFlash arm remained HTTP 503 during its bounded readiness window. No GPU1 slot profile change. 49 focused Node tests and 59 TUI tests passed, with zero new Praetor signatures. Logs, full argv and evidence under /tmp/vitriol-local-decision/.
