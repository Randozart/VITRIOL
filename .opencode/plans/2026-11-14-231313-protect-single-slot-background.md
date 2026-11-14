---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:13:13+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "b5bf29b1433cd0de7604369fc2fec77d82a008f2"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-230715-caduceus-shared-model-capacity.md"]
---

# protect-single-slot-background

## Purpose

Prevent background code reviews from displacing the foreground recurrent KV cache on a single-slot engine

## Approach and alternatives

Require confirmed multi-slot capacity for local background reviews before draining patches; retain existing always/idle policies on multi-slot engines

- Idle-only gating still replaces the single slot cache. Increasing engine slots needs a measured memory budget and a restart; not authorized by this fix.

## Scope and documentation

Background-lane launch gate, tests and operator guide

## Verification

- Zero/one/unknown-slot negative cases, multi-slot parity matrix, focused extension tests, typecheck, Praetor and final repository gates

## Invariants

1. Unknown, zero and single-slot telemetry never launch a local background review, even when idle or configured `always` (gate matrix and extension-level HTTP spy).
2. With two or more known slots, the existing `always`/`idle`, patch-size and engine-up decisions remain unchanged (existing matrix with explicit dual-slot fixtures).
3. Suppressed review patches remain pending and can be reviewed if multi-slot capacity later becomes available (extension-level transition test).
4. No engine configuration, KV contents, foreground request or running stream is modified by the guard. The gate runs before patch drain and HTTP submission (source review and no-fetch test).

## Evidence and mechanism

Observed engine task 164838 completes with 160540 resident tokens. Short task 165341 then restores a 336-token disk entry, processes 484 prompt tokens and generates 320 tokens. Task 165384 restores 156327 tokens (3515.4 MiB) but overshoots the matched prefix by 232 tokens; no usable checkpoint exists, so the engine explicitly forces full prompt reprocessing. Background events show a 1416-character patch review launched at 2026-11-14T22:09:58.545Z and findings at 22:10:16.094Z; its card mtime agrees and the review's default generation limit is exactly 320. This strongly identifies the short interloper as background review, though engine logs do not carry session/request correlation IDs. It does not prove the origin of the 232-token mismatch. Restored recurrent snapshots cannot in general rewind to an earlier matching prefix; preventing avoidable displacement reduces this failure opportunity without unsafe engine rollback changes.

The existing default background gate is `always`, not the idle-only behavior claimed by its header. It has no slot-count check, despite being designed for a second slot. Idle-only gating is insufficient: replacing idle foreground KV still loses its checkpoint ring on restore. Hypothesis is falsified as a complete explanation if misses persist without background requests; other sessions and prompt divergence can still cause them. Live behavior after reload cannot be established via this sandbox's denied localhost access.

Final combined evidence, including prefill/cloud changes since 95bc7b3, will live in `/tmp/officina-cloud-capacity-evidence`.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T23:13:13+01:00 — Plan created before implementation.

- 2026-11-14T23:16:31+01:00 — Implemented the slot-count guard before patch drain/HTTP and corrected the stale idle-only header. Unknown/zero/one-slot matrix plus extension-level transition tests prove suppressed patches remain pending and are submitted once after a second slot appears. Combined local-provider/Caduceus/routing/background/provider-health tests: 175 passed across 14 files; typecheck passed. Explicit Praetor scan over all 14 changed TypeScript sources since 95bc7b3, including tests copied out of hidden .pi directories: zero failures. Full final evidence is pending at /tmp/officina-cloud-capacity-evidence. No engine restart, slot change, foreground interruption or live requests; running extensions need reload. This prevents an observed cache-displacement contributor and does not establish that all recurrent-state mismatch causes are solved.
