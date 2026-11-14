---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:39:06+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "bbc08207106188bf1c04def46658a9d3df105302"
supersedes: []
superseded_by: []
related: []
---

# cache-lifetime-rust-assessment

## Purpose

Assess provable cache retirement and the fit of the existing Rust core for growing coordination responsibilities

## Approach and alternatives

Inspect on-disk identity, pruning policy and existing Rust/TypeScript boundary; distinguish proven obsolescence from an advanced session head

- Continue extending TypeScript policy versus consolidate policy incrementally in the existing Rust core. Rewriting the whole application is a larger decision not justified by this audit.

## Scope and documentation

Read-only cache/core audit and this assessment record; no retention or implementation changes

## Verification

- Direct source inspection of server-task.h/server-task.cpp, core-rpc.ts, vitriol-core library and RPC; read-only directory size accounting

## Results and evidence

Assessment at 2026-11-14T23:39:06+01:00 (Europe/Amsterdam), against bbc0820. This is a source-based recommendation, not an implemented migration or live cache reclamation result.

### Cache lifetime findings

- Read-only directory accounting found 16 files / 33,876,582,524 bytes (~31.55 GiB) in the current disk mirror. Counts are a point-in-time observation, not a stable configuration claim.
- `engine/tools/server/server-task.h` defines disk identity in terms of model/build/context/KV layout. The wrapper has no session, branch or generation ownership field. `server-task.cpp` chooses by token prefix, so entries can also be shared by sessions.
- `disk_spill` trims by disk budget and starts a detached writer. `disk_find_best` and `disk_load` skip entries contained in RAM/newer loaded entries; those skips do not establish disk-entry lifetime. Optional `VITRIOL_KV_DISK_PRUNE` handles damaged/incompatible entries rather than session generations. An older prefix snapshot is not automatically redundant for recurrent memory: a newer state may be unable to rewind to the prefix needed after a request changes. The earlier 232-token-overshoot reset is concrete evidence of this rollback constraint, not proof every ancestor must be retained.
- A session advancing alone is insufficient to prove that no branch, rewind, in-flight request or other session can reuse an entry. This engine metadata cannot currently prove the user's proposed obsolescence condition. No files were deleted and no retention setting changed.

### Rust fit and recommendation

- `vitriol-core/src/lib.rs` already owns pure provider-fact, admission and budget logic; `src/scheduler.rs` owns scheduler decisions. `officina/.pi/extensions/_shared/core-rpc.ts` exposes an opt-in stdio integration with TypeScript fallback and golden-vector parity. This is a concrete incremental migration foundation.
- The current sidecar is spawned per Pi process. Its globalThis singleton shares a child among extensions in that process, not across all sessions. Moving more helpers into it does not by itself create one authoritative cross-session owner.
- Recommendation: grow the existing Rust core into the policy authority for retry classification, admission and session/cache-lifetime decisions; keep narrow Pi event/stream adapters. Model ownership explicitly with session+branch generations and references to cache keys. If one service later owns cross-session scheduling, make its authority and failure behavior a deliberate protocol change rather than an incidental language port.
- For reclamation, require a durable accepted replacement and no live branch/rewind/in-flight/read/write references, and preserve explicitly useful recurrent rollback anchors. The detached writer must not republish a retired generation after deletion. These are proposed contracts, not properties already implemented or proved.
- The expected engineering benefit is explicit ownership and transitions, consolidation of currently distributed policies, and reuse of the existing Rust types/tests. No measured CPU/RAM/inference-speed improvement is claimed. Credible alternative: retain TypeScript while adding the same ownership protocol; Rust alone does not establish lifetime correctness.
- Smallest useful next implementation: a pure, tested snapshot-lifetime policy in the existing core, with counterexamples for shared prefixes, branch pins, recurrent rollback and pending writers. Wire deletion only after engine snapshot IDs and lifecycle acknowledgments make those inputs trustworthy. Existing TypeScript/Rust parity vectors provide a migration pattern; avoid indefinite duplicate authorities.

### Validation boundary

Only read-only local primary sources and filesystem accounting were used. No external best-practice survey, benchmark, new implementation or live inference was performed. No migration, cache deletion or engine restart is authorized by this assessment itself.

## Change history

- 2026-11-14T23:39:06+01:00 — Plan created before implementation.
