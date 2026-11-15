---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T13:27:20+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "6a3203df61bc7149e47a0fed30296b3ac50205e9"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-15-125010-officina-input-settings-chat.md"]
---

# officina-checker-warning-cleanup

## Purpose

Separate checker execution failures from invalid code, make child termination tests diagnostic and bounded, and eliminate Officina Rust warnings where behavior can be preserved

## Approach and alternatives

Stage checker outcome contracts and subprocess-test fixes first; remove dead code and apply reviewed compiler/Clippy repairs in coherent commits; retain all diagnostic checks and honest unavailable outcomes

- Suppressing lints or marking denied execution successful would hide real failures. A baseline records unresolved findings but does not eliminate them.

## Scope and documentation

Shared Officina diagnostics/subagent tests and canonical Rust TUI; no engine/model changes or live-session restart

## Verification

- Focused checker and subprocess regressions; full TypeScript and Rust suites; clippy all-targets with warnings denied; format, Praetor baseline/delta, release build/install and isolated terminal check; final frozen evidence

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T13:27:20+01:00 — Plan created before implementation.

## Invariants

1. I1: A checker reports passed only after successful execution; a nonzero syntax-check exit remains failed, while launch/permission/timeout failures are unavailable, never repair instructions. Pin with error classification and diagnostic rendering matrix.
2. I2: Successful checks remain silent; execution unavailability is reported to the operator without repeated model injections. Pin with extension event tests and existing token-budget tests. Existing explicit syntax failures still reach the model within budget.
3. I3: Force-kill escalation still uses actual exit state, never signal dispatch state. Tests bound startup/exit, retain real SIGKILL assertions, and clean up children. Denied capabilities must stay explicitly unavailable.
4. I4: With no new setting/input, Rust rendering, RPC payloads, session locking, Unicode input, compaction continuation and the new popup contracts remain unchanged. Full 250-test baseline plus relevant rendering/serialization tests pin parity.
5. I5: No new lint allowances, warning suppression, Praetor configuration changes, or test weakening. Warnings are removed by behavior-preserving cleanup or reported as unresolved with evidence. Kani/protocol declarations remain meaningful.

Baseline HEAD: 6a3203d. Prior full sweep: 250 Rust tests pass; 1220 extension tests pass, two subprocess tests fail under this execution environment. Exact original/final Clippy comparison at /tmp/officina-input-settings-evidence/clippy-baseline-delta.json shows 246 unchanged warnings. A valid Python child prints its result but Node execFileSync throws EPERM with status 0; this must not be misreported as a syntax failure. Preserve this counterexample. Stage order: checker/subprocess contracts, warning cleanup, then final frozen validation/install. The cleanup hypothesis is falsified by altered serialized commands, UI regression, lifecycle regression, added suppressions, or unavailable checks claimed clean.

### Checker stage — 2026-11-15 Europe/Amsterdam

Added explicit passed/failed/unavailable checker outcomes, preserving real nonzero syntax failures. Unavailability is recorded additively in per-tool facts, shown neutrally in the Rust TUI, notified once per checker outage and excluded from model repair injections. Pure classification, event/facts/notification and TUI regression tests cover these distinctions. Subagent signal verification now uses a bounded file readiness handshake rather than stdout delivery, preserves real SIGKILL assertions and cleans up with a child self-expiry fallback. Focused TS result: 34 passed, one real Python-through-Node check explicitly unavailable (EPERM); the real signal test passes. No capability check is treated as clean merely because execution was denied. Baseline source/Praetor and all evidence: /tmp/officina-checker-warning-evidence.

### Warning cleanup decisions — 2026-11-15 Europe/Amsterdam

The RPC command enum intentionally models the entire pinned Pi wire contract and is exhaustively golden-tested. Do not delete commands simply because the TUI currently constructs a subset. Expose that existing standalone protocol module as the crate's library boundary and have the TUI reuse it; this retains the complete wire schema and makes it usable by transport clients without pulling in terminal UI. Remove genuinely dead private helpers/reserved glyphs, keep test-only fixtures test-only, and retain the session-lock File as an explicitly owned lifetime guard. Prefer coherent request/render context records to long positional parameter lists. Rust's automatic fix command cannot bind its local lock listener in this environment (EPERM); apply reviewed MachineApplicable diagnostic edits directly instead of weakening sandbox policy or lint checks.

### Mechanical cleanup stage — 2026-11-15 Europe/Amsterdam

Retained every pinned RPC command/golden in a reusable protocol library; removed unused frontend wrappers/glyph reservations and made existing fixture helpers test-only. Kept the session lock's owned File lifetime intact. Applied machine-applicable iterator/saturation/derivation fixes, removed redundant branches and initialized test fixtures directly. Kani's existing external cfg is explicitly declared to Cargo's check-cfg while keeping unexpected-cfg diagnostics at warn; no harness is removed. Full Rust suite passes (protocol tests now execute in the library target). The last intermediate Clippy run fell from 246 to 22 warning instances; remaining work is coherent render/request contexts and compact tool metadata/cache storage. No lint suppression was added.

### Named contexts and compact storage — 2026-11-15 Europe/Amsterdam

Clippy now succeeds with `--all-targets -- -D warnings`: zero Rust/Clippy warnings, without lint allowances. Kept the full protocol schema and introduced named snapshot/panel/diff/table-field records to remove ambiguous positional arguments. Boxed optional tool JSON/check/cache payloads so each ordinary transcript entry no longer reserves their full inline size; cache invalidation and rendering tests remain green. Rust results: 5 protocol-library tests + 247 frontend tests passed (252 total). The inline storage regression test enforces a 224-byte upper bound versus the 352-byte baseline; measurement log is /tmp/officina-checker-warning-evidence/entry-memory.log. Existing broad Praetor non-comment findings decreased from 252 to 242 across the full Rust source copy. These are separate legacy complexity findings, not compiler warnings; no blanket clean claim is made.

### Final hygiene scope — 2026-11-15 Europe/Amsterdam

Remove the unused reqwest dependency (the native TUI makes no HTTP calls through it), reducing the dependency graph instead of ignoring the unused-dependency finding. The extension sweep's Vite warning comes from constructing a dynamic vendor import as `file://${path}`; use Node's pathToFileURL to preserve paths containing spaces/URL-special characters and remove the invalid-URL transform warning. Keep missing-vendor fail-open behavior and add a positive import regression using a local fixture, without any gateway call. This small follow-up is within the requested warning elimination; no runtime gateway deployment or credentials change.

### Hygiene results and final verification — 2026-11-15 Europe/Amsterdam

Unused-dependency check is clean after removing reqwest: Cargo.lock removes 115 package versions and adds none. Vendor module URLs now use pathToFileURL; the transform warning is gone. Eleven lane-health tests pass, including an actual Node-loader import from a local path with spaces and `#` (Vite's resolver cannot execute that positive URL case, so the regression invokes the real Node loader and bounds the child). No vendor API/network connection occurs in that fixture. Measured ChatEntry inline size is 208 bytes versus the 352-byte baseline (144 bytes less per vector slot); this is an inline-storage observation, not a total heap or speed benchmark.

Final freeze will follow this stage's commit. Required final commands: full evidence runner against baseline 6a3203d (Rust all-target Clippy/build/tests, TypeScript typecheck and extension suite, secrets, documentation links and advisory unused-code/dependency scans); strict Clippy `-D warnings`; cargo fmt check; scoped native Praetor baseline/delta; release rebuild and atomic install of ~/.local/bin/officina; isolated installed-binary PTY with no active-session or engine restart. Full logs, source-bound review and final manifests live outside the repository at /tmp/officina-checker-warning-evidence/final. The Python-through-Node capability test must remain explicitly unavailable if EPERM persists. Existing whole-frontend Praetor complexity findings and dynamic-extension unused-code analysis remain distinct from eliminated compiler warnings and must not be presented as clean.

### Final sweep amendment — 2026-11-15 Europe/Amsterdam

The frozen sweep passed 9 gates with one advisory but the unchanged llama-cpp-provider owner test exceeded Vitest's 5s limit during the loaded run (the local-provider case took ~9s). Its six-test file immediately passes in isolation in <1s. No ownership/streaming production source was modified; retain the real assertions and timeout rather than weakening the test. Rerun the full gate after correcting two source-comment placements found in final review. The prior failed manifest remains at /tmp/officina-checker-warning-evidence/final/20261115T124954Z-c9e57f2e/evidence.json and is superseded for current-source validation by the new frozen run. All 252 Rust tests and zero-warning strict Clippy remain the required gates.

### Provider-fixture diagnosis — 2026-11-15 Europe/Amsterdam

The ownership case's first stream call lazily imports the complete OpenAI SDK (`pi-ai/api/openai-completions.lazy.js` imports `openai-completions.js`). Under parallel suite startup that import can consume its unrelated 5s ownership-assertion budget. Load the same public provider module during test module setup so ownership assertions measure mocked-stream behavior, preserving their timeout, both provider cases, real payload checks and absence of network requests. This is a test-fixture repair, not a production retry/performance change.
