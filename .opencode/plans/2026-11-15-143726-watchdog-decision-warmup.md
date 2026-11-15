---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T14:37:26+01:00"
timezone: "Europe/Amsterdam"
status: "planned"
baseline_commit: "e5cd25b714ec046b005cb13648241381592aece4"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-15-decision-efficiency-research.md"]
---

# watchdog-decision-warmup

## Purpose

Fix watchdog blind spots and false ALIVE verdicts, retain monitoring independently of semantic judges, and define an opportunistic decision-model preparation path

## Approach and alternatives

Correct structured-output activity tracking, unknown verdicts, bounded/in-flight judgment lifecycle and per-session facts first; then add an opt-in typed decision adapter and conservative preparation admission using fresh resource evidence

- Replacing deterministic health/timeouts with unqualified semantic output would lose observability. GPU utilization alone is not VRAM headroom or a preemption guarantee. Preserve abstention and CPU-first fallback until a decision runtime is available.

## Scope and documentation

Canonical shared hang-watchdog extension, targeted Rust status UI if necessary, focused tests and research/operator docs; no active-model swap, service restart or destructive recovery

## Verification

- Regression matrices for real partialResult shapes, constant-length tails, rejudge budgets, unknown/unavailable, cancellation and stale completions; typed-adapter and foreground-priority preparation tests; full typecheck and extension suite with baseline failures preserved; Rust strict Clippy/tests/fmt and installed-terminal check if changed; source-bound final evidence

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T14:37:26+01:00 — Plan created before implementation.

## Invariants

1. I1: New tool output counts as activity for string, content-array and normal {content:[...]} partial results, including bounded equal-length rolling tails. Tests compare actual event sequences, not only lengths.
2. I2: Invalid, absent, timed-out or abstaining judgments never become ALIVE. They remain UNKNOWN/unavailable and do not stop deterministic monitoring. Test parse and runtime matrices.
3. I3: One in-flight judgment per call; completions after output, tool end, session switch or shutdown cannot publish stale verdicts. Actual judgment calls remain bounded and rate-limited. Test delayed promises and teardown.
4. I4: No new auto-kill/abort authority: existing Rust hard recovery and user Esc stay separate from advisory semantic classification. Unconfigured decision backends preserve the existing free-lane path. Pin event/RPC negative assertions.
5. I5: Background preparation is opt-in and bounded, never swaps the main model or consumes its single slot. GPU work requires fresh headroom/admission and stops admitting on foreground work; unknown telemetry blocks it. Test idle/half-idle/active/stale/memory matrices. CPU remains the initial low-interference option.
6. I6: Cache reuse must match model/revision/tokenization/schema and unchanged evidence. A warm encoder is not the actor's KV cache; no unverified cache-hit or GPU preemption claims. Evidence changes invalidate pending work.

Baseline: e5cd25b; Rust compiler/Clippy is warning-free, 252 Rust tests pass. Previous broad TS run retains jiti/grind 5s startup timeouts under high host load; do not weaken these checks or infer watchdog correctness from them. Actual watchdog facts repeatedly record unparseable replies as ALIVE, including done/no-next-check after three attempts. Source currently ignores object-shaped tool partial results, measures truncated-tail length rather than activity, shares one facts file across sessions, and does not cancel judges on tool/session completion. Research capabilities: no qualified decision-model runtime/checkpoint was installed in prior probes; verify availability before any live model claim.

### Stage 1 implementation — 2026-11-15 Europe/Amsterdam

Fixed content-envelope extraction, bounded revision tracking (including changing equal-length tails), early activity timestamps, rearmed episode budgets, rate limiting after any attempt, in-flight deduplication, bounded cancellation even for a non-cooperative HTTP promise, and stale-result guards for output/end/session changes. Unknown/unavailable replies stay UNKNOWN; three paid attempts no longer stop deterministic monitoring. RPC notifications no longer depend on a hasUI flag when a notify surface exists. Session-scoped files and per-entry boxed snapshots prevent cross-session/call verdict reuse; explicit bounded leases keep five-minute rejudge countdowns visible without extending legacy freshness. The smoke test now uses only injected facts rather than reading the operator's watchdog file. Tests: 17 focused TS cases pass; 255 Rust tests pass (5 library + 250 frontend), strict all-target Clippy -D warnings and typecheck pass. No model/service/GPU changes have been made. Evidence: /tmp/officina-watchdog-evidence.

### Source-backed decision constraints

Inspected on 2026-11-15: [Laya model card](https://huggingface.co/convaiinnovations/laya), [agent result/coverage source](https://github.com/NandhaKishorM/laya/blob/main/laya/agent.py), [joint encoder/tokenization source](https://github.com/NandhaKishorM/laya/blob/main/laya/common.py), and [CUDA stream-priority documentation](https://docs.nvidia.com/cuda/cuda-programming-guide/02-basics/asynchronous-execution.html). The documented serving route is POST /v1/systemone. Preloading removes checkpoint startup; it is not reuse of the actor's KV or proof that changing-state encoder features can be reused. Current Laya source distinguishes entropy-style confidence from answer_confidence, reports abstention/action and input/question truncation. Inspect these fields rather than assuming a confident label is correct. The model card reports poor base-model zero-shot typed-decision results, so no watchdog quality is qualified here. CUDA stream priority is a hint, not kernel preemption. Low utilization alone does not justify loading another model or promise foreground latency. These are source findings and engineering inferences, not VITRIOL model/GPU measurements.

### Stage 2 scope amendment — 2026-11-15T15:18:16+01:00 Europe/Amsterdam

Implemented an opt-in Laya-compatible typed judge with bounded evidence, answer/action score checks, explicit abstention/coverage rejection, cancellation and no cloud retry after a configured endpoint fails. The documented Laya action is an object containing act_probability, and confidence gates report passed/abstained/unevaluated strings; tests pin these actual source contracts. No model-serving dependency is added. The unconfigured path retains the free-cloud judge.

The preparation work is deliberately an offline qualification policy and test matrix, not an enabled GPU scheduler. Default Python has no laya/torch/transformers/onnxruntime, and this execution environment cannot communicate with NVIDIA via nvidia-smi. Therefore actual model latency/accuracy, per-device headroom, bounded work duration and foreground interference remain unverified. No GPU/model downloads, slot claims, loads or service changes were performed. Before enabling live preparation, collect fresh engine/queue and per-device memory facts, qualify a pinned checkpoint and bounded work on this hardware, then falsify the slack hypothesis with foreground prefill/decode latency under concurrent preparation. A slow foreground tail, stale telemetry or changed checkpoint invalidates the proposed admission. CPU-first independent serving remains the least coupled initial experiment. Identities are required but no feature/result cache is implemented or claimed.

Focused stage 2 result: 57 watchdog/client/admission tests pass; full typecheck passes. The Rust release/installed-terminal check and frozen final gates are required next. Final commands: verified-development evidence.py plan/run/status with --base e5cd25b, external evidence root /tmp/officina-watchdog-evidence/final; focused native Praetor comparison includes changed sources and retains unchanged legacy findings. Existing broad-suite startup timeout failures are preserved if reproduced, not hidden.

Installed-release result: release build passed; installed and build hashes match (ab0a245891827f84ebd6f9d38386e3ad462922b7692e676424260c5892011d37). Isolated 120x40 PTY verified a session-scoped UNKNOWN card with its own command/reason, fire/shimmer/sidebar menus, persistence and clean quit. Active operator sessions were not restarted. Native Praetor reports zero blocking findings for the watchdog directory after separating wire coverage/score validation and per-device admission responsibilities. Final source freeze follows the stage 2 commit.

### Review amendment — 2026-11-15 Europe/Amsterdam

The per-session file still allowed queued notifications for two calls to read the same last-written snapshot. Carry each snapshot inside the notification with a versioned internal delimiter; the Rust TUI consumes metadata separately from visible prose, validates freshness/session/call identity and never substitutes a newer file for malformed embedded evidence. Keep the file fallback only for legacy notifications. Add a delayed/overwritten-file regression and malformed/foreign metadata negative checks. This supersedes reliance on file timing for canonical notifications; rerun release installation, PTY and all final gates after the fix.

Review-fix result: 256 Rust tests (5 library + 251 frontend), strict all-target Clippy, 57 watchdog tests and typecheck pass. Release rebuilt/reinstalled; isolated PTY renders embedded UNKNOWN evidence with no facts file and passes menu/persistence/quit checks. Native Praetor has no diagnostics on the new wire decoder/coalescer or watchdog TS directory. Whole Rust scan retains 244 non-comment diagnostics versus baseline 242: new function-length advisories on the revised entry hash and ownership test; existing apply_notify cognitive complexity improves 30 to 22, chat_lines metrics unchanged. No configuration/threshold changes or metric-only splits were made. These legacy findings are not a clean native Praetor outcome; strict compiler/Clippy remains clean. Final source-bound evidence supersedes earlier plan outputs.
