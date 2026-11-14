---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T20:42:25+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "92eb5f0abd0d00ec4c1f8e39316dbf845184f977"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-14-204001-vitriol-log-diagnosis.md"]
---

# checkpoint-restore-recovery

## Purpose

Stop automatic retry loops for stale KV checkpoint recovery markers while preserving recovery metadata and temporary-failure retries.

## Approach and alternatives

Quarantine recovery markers on explicit invalid/missing checkpoint responses, retain them on network/server/auth/resource failures, and keep successful restore behavior. Handle the engine ambiguous invalid-slot-file response conservatively by retaining a quarantined copy for manual recovery.

- Deleting every failed marker can lose recovery after transient outages; local filesystem preflight cannot generalize to remote engines and violates the HTTP-only checkpoint boundary.

## Scope and documentation

Officina checkpoint extension, pure restore disposition, regression tests, operator guide; retire only the confirmed missing-file runtime marker with a backup.

## Verification

- Focused mocked-HTTP session-start tests over temporary markers: success, stale retry suppression, transient retention, peer/model/session guards, backup preservation; full extension suite, typecheck, Praetor and source-bound evidence.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T20:42:25+01:00 — Plan created before implementation.

## Invariants

1. I1: Successful save/restore, autosave cadence, peer exclusion and session/model checks retain their existing behavior; covered by existing checkpoint tests and session-start integration controls.
2. I2: A checkpoint-specific rejected restore is attempted at most once automatically per marker, with the marker bytes preserved under failed-restores; pinned by two-start and byte-preservation regressions.
3. I3: Network/timeout, authentication, rate-limit, 5xx and unknown errors preserve the active marker and allow a later startup retry; pinned by HTTP/exception matrices.
4. I4: Moving a marker fails safely: no marker content deletion, no engine/session mutation, and an actionable warning; pinned by changed-marker and filesystem tests.
5. I5: Long repaired session filenames remain valid during quarantine; backup directories, rather than filename suffixes, avoid NAME_MAX overflow; pinned by 242-byte marker regression.

## Baseline and constraints

The existing engine returns HTTP 400 with "Unable to restore slot: No available space in KV cache or invalid slot save file" when loading returns zero. Logs separately confirm ENOENT for the observed file, but the HTTP response alone is ambiguous. Quarantine is reversible and does not assert permanent corruption. Existing checkpoint source is clean against HEAD; unrelated context/router/cloud-probe/vitriol-core changes must remain untouched. Evidence and full logs: /tmp/officina-checkpoint-recovery-evidence/. Verification environment previously denied/broke Node subprocess tests; preserve explicit failures if reproduced.

## Implementation and runtime repair — 2026-11-14T20:45:48+01:00 (Europe/Amsterdam)

Implemented checkpoint-specific HTTP 400/404/422 classification and reversible marker quarantine under failed-restores/restore-<unique>/. The actual engine's ambiguous invalid-file/capacity rejection disables automatic retry but retains exact metadata for manual recovery; unknown errors and transient/auth/server failures retain the active marker. Quarantine checks the current marker still matches the rejected request. Backup-directory allocation and rename failures leave it intact. No local engine filesystem access is added to the extension.

Moved only the runtime marker correlated with the confirmed ENOENT log entry; rechecked the KV file was absent before the move. Backup: `/home/randozart/Desktop/Projects/.pi/ckpt/failed-restores/restore-X17DKc/2026-11-11T17-41-00-647Z_01a1cbb2-e266-711b-b9dc-184f0eda71ef.jsonl.repaired-1794492456716.jsonl.patched-1794494530829.jsonl.patched-1794496881251.jsonl.patched-1794497659437.jsonl.patched-1794509459821.jsonl.patched-1794511055834.json`. Original marker now absent; backup SHA256 is `fa82690294bc741ba97016e047be3d34997258c41878b42fc4f14921272dab7d` and matches the pre-move bytes. Runtime receipt: /tmp/officina-checkpoint-recovery-evidence/runtime-repair.json. No engine restart, live restore, KV deletion or configuration edit occurred.

Development checks: checkpoint suite 40 passed/1 live-engine test skipped; typecheck passed; source-scoped Praetor on index.ts/client.ts/recovery.ts passed with zero blocking diagnostics. Full extension run reproduces the two known untouched subprocess test failures. Final gate results and reviews remain external. Live save/erase/restore round-trip is intentionally unrun because it mutates the operator's active slot.

Final validation: verified-development changed-scope runner (including full npm suite/typecheck and other selected existing gates), focused checkpoint tests, Praetor, diff check and source freshness. Required full-suite failures stay explicit; do not mark the patch universally verified or bypass hooks. Preserve concurrent context/router/vitriol-core edits.
