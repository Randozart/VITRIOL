---
schema: "codex-work-plan/v1"
created_at: "2026-11-14T19:54:27+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "e36db1f9a0e854260564b16afc6147c8410799e8"
supersedes: []
superseded_by: []
related: [".opencode/plans/officina-multisession-hardening-2026-11-11.md"]
---

# officina-durable-context

## Purpose

Reduce redundant local prefill by persisting scratchpad, task and instruction snapshots at safe conversation boundaries.

## Approach and alternatives

Use before_agent_start messages and turn_end custom-message drafts for durable deduplicated state; request-only recovery after compaction with non-triggering persistence. Keep clearing threshold unchanged and document controlled tuning.

- Request-only tails relocate each inference; more aggressive clearing can invalidate long cached suffixes. A Rust runtime rewrite does not resolve this contract mismatch.

## Scope and documentation

Shared context helper; scratchpad, task-state and agents-md extensions; regression tests and operator guide. Preserve concurrent routing/small-lane edits.

## Verification

- Exercise installed pi runner and session boundary persistence with multi-step positive/negative tests; full Officina extension suite, typecheck, Praetor, source review and evidence freshness.

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-14T19:54:27+01:00 — Plan created before implementation.

## Invariants

1. I1: With unchanged state and an extending transcript, the previous outgoing message sequence is an exact prefix of the next, pinned by installed-pi multi-step regression tests.
2. I2: Changed state appends a durable snapshot after complete tool results; previous entries remain unchanged and no additional inference is requested, pinned by boundary draft tests.
3. I3: Compaction/resume restores current state without relying on process-local dedupe; request-only recovery is retained until the next safe persistence boundary, pinned by projection-reset tests.
4. I4: Kill switches, external state formats, clearing keep-window and 92% default threshold remain unchanged; existing suites plus threshold tests pin parity.
5. I5: Empty/uninitialized state injects nothing; separate sessions cannot inherit snapshots through helper state, pinned by negative tests.

## Baseline and falsification

Installed pi 1.5.0 clones context-event messages and does not persist their additions. A direct runner probe produced user/custom then user/assistant/toolResult/custom, preserving only the user prefix. The optimization is falsified if unchanged-state tool continuations still relocate snapshots under the installed runner. No live speedup or token savings will be claimed without engine measurements. Concurrent routing/small-lane/vitriol-core work is excluded from commits; repository-wide gates may expose its failures or drift. Evidence: /tmp/officina-context-evidence/.

## Implementation notes — 2026-11-14T20:01:10+01:00 (Europe/Amsterdam)

Implemented one shared durable-state registration for scratchpad, task-state and AGENTS.md. It returns persistent before-agent messages, composes turn-end drafts without setting continuation, and retains request-only recovery after compaction. Explicit reset markers supersede cleared state; inactive/uninitialized stores emit nothing. No Rust/TUI change or inference calls were added. Clearing frequency and keep-window are unchanged; the operator guide documents an optional 85% threshold experiment and its measurement boundary.

Focused installed-runtime tests: seven new regressions, including the old request-only negative control, real in-memory compaction, changed/cleared state, multiple boundary contributors and session isolation. Related scratchpad/task/AGENTS/jiti suite: 48 passed. Initial full suite: 1089 passed, 21 skipped, two failures in untouched subprocess tests. Node spawn probes show EPERM for synchronous spawning and missing stdout from an asynchronous child; both failing subprocess tests reproduce independently here. This is a verification environment limit, not a green full suite. Typecheck and source-scoped Praetor were clean during development; final evidence must rerun after source freeze.

Final commands: npm run typecheck; npm test; focused installed-runtime tests; git diff --check; source-scoped Praetor validate --warn --json (four production files copied with hashes to an external scan directory); verified-development changed-scope evidence run/status. Full logs and review artifacts stay under /tmp/officina-context-evidence and /tmp/officina-context-*. No live engine benchmark was run, so wall-time savings remain unmeasured.

## Amendment — 2026-11-14T20:03:08+01:00 (Europe/Amsterdam)

Final review found the session ledger is another request-only moving tail. Expand scope to persist ledger snapshots at the same boundaries; otherwise it still forces replay of the latest assistant/tool exchange. Calculate ledger statistics only at boundaries, excluding system messages and prior ledger snapshots, to avoid self-induced snapshot updates. Retain old ledger entries in history rather than relocating/deleting them. Add an installed-runner regression with the actual ledger extension enabled. Prior final evidence is superseded by a fresh run after this amendment. I1 now explicitly includes the enabled ledger; no claim is made for arbitrary optional extension injections or provider tokenization.

## Final implementation freeze — 2026-11-14T20:04:37+01:00 (Europe/Amsterdam)

Ledger migration is included. Removed the obsolete relocateLedger implementation and its misleading prefix tests; retained rendering/type-recognition tests and added a stronger installed-runner test with the real ledger extension. Boundary statistics omit system messages and ledger snapshots, and refresh only at boundaries. Prior snapshots remain until compaction: this trades a small amount of retained context for avoiding re-evaluation of intervening generated text and tool output. Net GPU time is still an unmeasured hypothesis, not a claimed benchmark result.

Final validation reruns the full suite and typecheck through the evidence runner plus focused context/ledger/loader tests and source-scoped Praetor. Existing subprocess environment failures remain explicit; no test or gate was weakened to obtain a green result. Source review, exact copied-source hashes, full logs and evidence manifests live outside the repository in /tmp/officina-context-evidence/. No commit is considered verified while the required full suite remains failed.
