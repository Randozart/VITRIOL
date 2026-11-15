---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T11:11:44+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "35423b22a1195b5fac6cb8465ef13842547646fd"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-15-102907-optimization-lab.md"]
---

# lab-acceptance-tests

## Purpose

Test the installed optimization laboratory beyond examples and establish actual local emulator/decision-model capability

## Approach and alternatives

Read-only installed-CLI acceptance tests with fixed-seed synthetic stress cases, independent event/cache safety checks, bounded worker negative controls, numerical oracle cross-checks and explicit optional-model availability results. Repair only demonstrated defects.

- Repeat only toy examples: too weak to assess event ordering. Claim absent local model/emulator passed: invalid; report unavailable. Hosted Jev calls or live GPU experiments are not a substitute for a CPU decision-model test.

## Scope and documentation

Acceptance evidence and a repository results note; source regression/fix only if a concrete test exposes a defect. No active engine, GPU configuration, service or routing changes.

## Verification

- Repeated deterministic outputs; no future-state reuse or slot overlap; token/time accounting; protected and tier budget bounds; heterogeneous capacity rejection; wrong-answer/malformed/timeout/unavailable CLI outcomes; independent numerical reference answers; checkpoint/source freshness

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T11:11:44+01:00 — Plan created before implementation.

- 2026-11-15T11:15:02+01:00 — Installed-CLI acceptance completed:60 deterministic fixed-seed workloads,240 candidate reports,11520 request outcomes,36 independent Python numerical comparisons and four CLI negative controls all pass. Synthetic example favors disk-tier110us vs baseline240us declared prefill and excludes over-budget two-slot candidate. No code defect found. Laya/AnyJev packages and weights absent; Python3.12 has Torch/Transformers but PyPI/HuggingFace probes fail DNS. Actual decision-model/emulator execution unavailable, not passed. Research/result boundaries are recorded in .opencode/plans/2026-11-15-lab-acceptance-results.md. Private durable test script/evidence: ~/.local/state/vitriol-lab/acceptance/2026-11-15. Final docs/secrets/link/source review checks will be stored under /tmp/vitriol-lab-acceptance/final. No runtime configuration, routing or active engine changes.
