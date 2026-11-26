---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T14:56:44+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "9c880b2f6b229754e2fb8489fb9dcd06b2d6e951"
supersedes: []
superseded_by: []
related: []
---

# officina-model-catalog

## Purpose

Discover configured providers' models automatically, add model-first and provider-first pickers with recent choices, and route the exact same model across eligible providers.

## Approach and alternatives

Use the live Pi registry and cached Caduceus catalog, add standard extension dialogs and persistent recent model IDs, and register virtual same-model choices over verified physical routes.

- Use existing Pi RPC extension dialogs instead of adding another independent Rust picker. Refresh the available registry in the background with a TTL and a single in-flight refresh. Unauthenticated or unsupported listings keep the cached data.
- The legacy Caduceus gateway route may substitute a different model. Register a direct, exact gateway catalog for verified provider-owned model IDs on the local gateway. Keep the broad Caduceus route and Lapis/Nimbus unchanged. Unknown custom proxy endpoints stay on the existing path.

## Scope and documentation

Extend the Caduceus and model-catalog Pi extensions and update the standalone Officina guide. Recent model and route IDs are user-local. No prompts, provider account edits, or provider-key discovery are needed. Auto-route uses the current registry, exact IDs, established cost and cooldown policy, and Pi's retry lifecycle.

## Invariants

1. Existing `/model`, Lapis, Nimbus and broad gateway fallback work as before. Verify unchanged extension and router tests.
2. Explicit gateway selections send only a verified provider-owned model ID to the direct local gateway. Auto-route cannot broaden the selected identity. Verify negative controls with different versions, variants and similar names.
3. Only connected and eligible routes participate in Auto-route. Preserve budget controls and provider/model denials, skip known cooldowns and already-tried routes, and never use an unknown price as evidence of free service. Verify with injected catalog and policy fixtures.
4. Pickers read an existing snapshot immediately. At most one discovery pass runs at a time per Pi process, failed or empty discovery preserves the last good data, and recent IDs cannot resurrect unavailable routes. Verify with mocked registry and picker sequences.

## Verification

- Exercise catalog refresh, both picker orders, MRU persistence, exact gateway selection, eligible same-model retries and existing picker parity.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-26T14:56:44+01:00 — Plan created before implementation.

- 2026-11-26T15:09:34+01:00 — Implemented background registry/catalog refresh, both menu orders, recent route IDs, strict local Caduceus selections and same-model virtual routing. Pi RPC verified registration, chooser order, recents and Auto-route selection without live inference or provider mutations. Typecheck, 175 targeted tests and focused Praetor --warn passed. The scoped evidence runner and integration fixtures use /tmp/officina-model-catalog/.

- 2026-11-26T15:10:44+01:00 — The full test run flagged a new cross-extension import. Move the pure Caduceus model catalog helpers into _shared, retaining the old feature import path for existing callers. Rerun the boundary test and model picker coverage.

- 2026-11-26T15:12:50+01:00 — Shared helpers retain their existing import path. Typecheck, 44 focused tests including the extension boundary, focused Praetor, and the Pi RPC picker smoke passed after the move. Seven unrelated theme, dream-gen and permissions assertions also fail at the baseline. The sandbox prevents localhost listeners used by the full suite. Final source evidence is recorded under /tmp/officina-model-catalog/verification/.
