---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T00:25:57+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "3ed6459b6c25191562e7022c90770ddd023c1ed1"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-234347-rust-cache-lifetime-core.md"]
---

# managed-kv-ownership

## Purpose

Connect the Rust lifetime policy to engine-owned publication, references and bounded reclamation for new KV snapshots

## Approach and alternatives

One Rust store per engine/directory, protected by a process mutex and exclusive OS lock; persistent generations and publication tickets; durable atomic publication; read leases and active/rollback pins; policy-based bounded GC with epoch/incarnation recheck and tombstones

- Filesystem age scans and detached unlink cannot fence late publishers. Legacy anonymous entries stay on their existing budget policy. A separate per-session sidecar cannot be the shared owner of engine writers.

## Scope and documentation

Rust managed-store runtime and FFI, engine cache integration, local session/branch tags, protocol model, race/crash tests and isolated build artifacts

## Verification

- Finite P schedules with late-writer negative control; Rust durability/recovery/read/write/shared-owner tests; FFI/host integration and adapter branch tests; existing Rust/TS gates, source-scoped Praetor, C++ build and isolated server tests

## Invariants

1. Feature disabled or missing ownership tags preserves legacy cache names, reads, writes and limits (legacy parity tests). Anonymous files never enter managed reclamation.
2. A single engine-owned Rust store holds the directory's exclusive OS lock. All managed state transitions, publication and deletion run under its process mutex; a second owner cannot open it (lock/concurrency tests).
3. Generations and publication IDs are persisted before use. Publishing requires a valid ticket, durable bytes and a still-current owner generation; an old detached writer cannot republish after supersession (late-writer test and negative-control model).
4. Every managed read acquires a lease before opening the file; reads, active task references, branch heads and two most recent acknowledged durable rollback states prevent deletion (shared/read/pin tests).
5. Advancing a branch requires successful task completion and a durable published state. Unacknowledged, failed and aborted generations cannot retire its earlier durable head (abort/recovery tests).
6. Reclamation uses the existing Rust lifetime predicates, is bounded by files/bytes and defaults to dry-run. Under the same lock it rechecks epoch/incarnation, persists a tombstone before unlink and removes retired publication rights (race/crash/replay tests).
7. Restart recovery obtains the OS lock before discarding dead process leases or incomplete publications. Unknown files remain unowned; legacy state formats and identity checks remain intact (crash-window tests).
8. Ownership metadata is request-local and local-provider-only; branches survive extension reload and navigation creates a distinct reference frontier. No prompt/tool output is altered or truncated (adapter tests and existing provider suite).

## Implementation and activation

1. Rust managed store runtime in a separate crate depending on the pure core, with transactional JSON metadata, native FFI and adversarial filesystem tests. The pure policy stays free of I/O. Commit the completed runtime stage.
2. Execute a finite P model of publish/read/retire ordering and the late-writer negative control, with source mapping and limitations outside the repository.
3. Integrate the optional Rust static library with engine cache publication/restoration, exposing managed statistics in /cache. Managed payloads use a private namespace, separate from root legacy trim/prune. Add local session/branch tags and retain old clients' behavior. Commit completed integration stages.
4. Build and test isolated artifacts without changing the active server or CUDA/config fingerprint. Runtime ownership and physical reclamation use distinct opt-ins (`VITRIOL_KV_OWNED_CACHE`, `VITRIOL_KV_OWNED_RECLAIM`); new managed storage stays bounded. Activation of a rebuilt engine requires a separately identified service restart; no live request is interrupted for development.

Every metadata error disables managed effects rather than guess ownership; inference can continue with the existing cache path. A manifest accepted by the engine is authoritative only within its managed namespace, never for user-created or legacy cache files. Filesystem durability assumes local POSIX rename/fsync and an exclusive cooperating owner; external tampering and failing storage cannot be proved away.

## Baseline and falsification

Starting commit 3ed6459, clean tree and private resumption checkpoint match. Prior Rust27 tests passed; Node has two established subprocess sandbox failures, and existing Praetor/ShellCheck findings remain baseline. Production localhost/GPU telemetry remains denied in this sandbox. The deployed build directory is outside writable roots; isolated host/CPU verification and a staged host rebuild against existing GPU artifacts will be investigated without changing deployed libraries. Reject the implementation if a stale writer becomes visible, a pinned/read snapshot is deleted, a failed task advances retirement, a source scan claims ownership for legacy data, or opt-outs change payloads/cache behavior. No performance improvement is asserted without measurement.

Final evidence and model traces: `/tmp/vitriol-managed-kv-evidence`.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T00:25:57+01:00 — Plan created before implementation.

- 2026-11-15T00:46:53+01:00 — Rust runtime stage implemented in vitriol-cache-store with private POSIX namespace, exclusive OS owner lock, durable publication IDs and generation/epoch reservation, owner/reader/task pins, acknowledged two-generation branch rollback anchors, bounded policy-driven GC and tombstone/recovery handling. Thirteen filesystem tests pass, including delayed writer revocation, shared borrowers, failed requests, epoch restart, budget reservations, publication collision preserving unknown files and injected crash-window recovery fixtures. Clippy -D warnings and explicit source/test Praetor pass (0 blocking findings). P tcSerialized sampled1000 schedules seed42 with no counterexample; tcMissingFence reproduces the intended late-writer assertion with replay artifacts. Model scope is finite serialized owner/reader/newer publisher/stale writer, not a filesystem durability/conformance proof. Traces and source-bound mapping will be retained in /tmp/vitriol-managed-kv-evidence. Runtime remains unconnected to engine until the next stage; no live payload/cache/config changes.

- 2026-11-15T01:25:39+01:00 — Officina adapter implemented and verified: persisted non-message session/branch markers survive reload, tree navigation forks the ownership frontier, independent jiti instances share the registry, and only registered local requests receive ownership fields. Metadata-write failure falls back without breaking sessions. Ownership and prefill opt-outs are independent. Provider suite: 72 tests pass; TypeScript check passes; explicit adapter Praetor scan has zero blocking findings. Evidence: /tmp/vitriol-managed-kv-evidence/owner-tests.log and adapter-praetor.json. Engine integration remains under development; no service/config changes.
