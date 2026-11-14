---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T23:43:47+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "a31249722af46d3b66a4a4b5f48e51b9afef5317"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-233906-cache-lifetime-rust-assessment.md"]
---

# rust-cache-lifetime-core

## Purpose

Start the Rust coordination migration with one authoritative, fail-closed cache-lifetime policy and a usable audit boundary

## Approach and alternatives

Represent immutable snapshot identity, owner generations, explicit retirement and pins in Rust; require a durable same-owner successor and complete reference coverage; expose read-only decisions through existing stdio RPC and a cache-lifetime audit command

- Deleting by age or later session head cannot prove obsolescence. Replacing the whole engine/gateway or adding a second global daemon before an ownership contract exists expands the risk without supplying trustworthy lifecycle facts.

## Scope and documentation

vitriol-core lifetime policy and RPC, thin Officina audit integration, regression tests, documentation, staged commits

## Verification

- Rust unit/property-style counterexamples and RPC schema tests; adapter fail-closed and audit fixtures; cargo test/fmt/clippy, focused TS tests/typecheck, Praetor, final repository evidence

## Invariants

1. Unknown ownership, incomplete reference coverage, pending writers, active readers and any branch/rewind/rollback pin retain a snapshot (negative matrix).
2. Advancing a session head is insufficient: every exact session/branch/generation owner must explicitly retire its reference to a durable newer same-owner snapshot with matching model/layout identity (shared-owner and successor counterexamples).
3. Replacement targets remain retained in the same plan, so a batch cannot delete its own durability witnesses (chain/cycle regressions).
4. Decisions bind to the supplied inventory epoch and snapshot incarnation. The planner never authorizes physical deletion or performs I/O; any future executor must revalidate under authoritative synchronization (serialization contract and source review).
5. Bad versions, duplicate IDs/owners/retirements, dangling retirement references and accounting overflow fail closed (schema/validation tests).
6. Existing admission/probe/error RPC contracts remain unchanged. The new policy has one Rust implementation; legacy disk audits cannot fabricate owners or retirement facts (existing vectors, RPC and real-directory fixtures).

## Stages and contracts

1. Pure Rust lifetime module, adversarial lifecycle tests, API documentation; commit after Rust gates.
2. Typed read-only RPC method and native audit command (`vitriol cache-lifetime`), existing-disk inventory and fixture tests; build/install the native audit/RPC binaries and commit.
3. Combined repository verification and checkpoint. Existing anonymous disk entries remain retained by this interface. This stage does not claim automatic reclamation or activate an engine deletion endpoint: engine ownership tags, complete cross-session references and synchronized publication/removal are still required for an executor. The audit reports that boundary directly rather than guess from file age or later prompts.

The provided authoritative inventory is a precondition to policy eligibility, not proof the current engine supplies it. Treat pure planning and a read-only disk inventory separately. No new concurrent deletion protocol is implemented, so an executable P ownership/deletion model is deferred until that protocol has real source transitions. Simple reference predicates and finite counterexample cases are checked in Rust now.

## Baseline and falsification

Baseline Rust cargo tests passed offline before edits; existing golden vectors remain mandatory. Caduceus interruption was fixed separately in23ec528 and remains in final combined validation. If an ancestor becomes eligible merely because a longer snapshot exists, if a shared owner/pin can be lost, or if the live anonymous mirror audit reports reclaimable bytes, reject the implementation. No GPU benchmark, engine rebuild/restart or cache deletion is part of these stages. Final evidence will live outside the repository at `/tmp/vitriol-cache-lifetime-evidence`.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T23:43:47+01:00 — Plan created before implementation.

- 2026-11-14T23:58:04+01:00 — Stage1 implemented pure Rust inventory/ownership retirement policy with strict schema, exact owner generations, durable same-layout successors, pins/read/write protection, batch successor protection, epoch/incarnation binding, checked accounting and fail-closed legacy handling. Ten adversarial lifecycle tests added; existing golden vectors retained. Rust test/fmt/Clippy gates passed. Existing Clippy warnings in provider-fact default-field initialization and golden-vector push construction were repaired with equivalent initializers; golden vectors unchanged. Source-scoped Praetor includes policy and lifecycle tests; full final evidence remains pending. No deletion executor or engine changes.

- 2026-11-15T00:03:19+01:00 — Stage2 complete: strict cache_retirement_plan RPC, native vitriol-cache-lifetime audit binary, vitriol cache-lifetime dispatch, declared-inventory fixture and four real subprocess/CLI/RPC tests. All27 Rust tests (including10 lifetime and4 wire tests) pass; fmt/Clippy pass; scoped Praetor over policy, RPC, CLI and new tests has0 failures. ShellCheck baseline/candidate comparison found0 new diagnostics; existing launcher findings remain baseline. Release binaries built offline, installed under ~/.local/bin, and SHA-256 matched build artifacts. Installed command audited the real mirror:15 published snapshots,30,713,587,500 bytes,0 eligible bytes, physical deletion unauthorized. This is expected fail-closed behavior for unowned legacy entries; no files removed and no engine restarts/requests. Fixture audit correctly reported100 conditionally eligible bytes while still denying physical deletion. Combined final gates pending in /tmp/vitriol-cache-lifetime-evidence; automatic synchronized reclamation remains a future engine ownership protocol, not an implemented claim.
