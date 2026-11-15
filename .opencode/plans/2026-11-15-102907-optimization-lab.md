---
schema: "codex-work-plan/v1"
created_at: "2026-11-15T10:29:07+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "c3be904973f209028663b0271837094d8b814163"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-15-002557-managed-kv-ownership.md"]
---

# optimization-lab

## Purpose

Document GPU emulation findings and ship a deterministic CPU optimization laboratory with real governor replay and bounded emulator correctness adapters

## Approach and alternatives

Independent Rust CLI/library with versioned private trace inputs, discrete-event slot/prefix/tier simulation, candidate comparison with memory feasibility checks, actual governor policy reuse, and isolated external kernel-check protocol. Document assumptions and never equate CPU/emulator time with hardware speed.

- Full cycle-level simulation first: expensive setup and target calibration, no demonstrated mixed-GPU accuracy; keep Accel-Sim selective. Copying governor policy would drift; reuse its library. Existing Spagyric uses live fixed-prompt sweeps and does not substitute for replay.

## Scope and documentation

Research dossier, vitriol-lab crate and fixtures, optional correctness adapter, vitriol command dispatch and operator documentation; no live services/configuration/GPU builds changed

## Verification

- Deterministic replay golden/negative tests, temporal publication and pin/budget invariants, heterogeneous memory feasibility, existing governor parity, timeout/output/error/numerical adapter tests, Rust fmt/Clippy/tests, shell baseline comparison, Praetor source/test checks, offline docs/examples and frozen source-bound evidence

## Results and evidence

Record actual outcomes, affected contracts/docs, commits and evidence links as work completes.

## Change history

- 2026-11-15T10:29:07+01:00 — Plan created before implementation.

## Invariants

1. Existing runtime behavior is unchanged without an explicit lab invocation; replay never initializes NVML, sends HTTP, changes profiles, deletes real snapshots or restarts processes (offline CLI and governor parity tests).
2. Equal validated input yields byte-identical replay reports; equal-time completions precede arrivals, and future publications cannot satisfy earlier requests (determinism/event-order tests).
3. Pinned entries and active read references are never budget-evicted; overflow skips admission and reports pressure. Simulated eviction conveys no engine retirement authority (pin/budget tests).
4. Cache reuse requires matching model/layout and a usable token prefix; overshoot beyond the declared rewind bound is a miss. User content is never truncated to improve a metric (identity/divergence tests).
5. GPU capacity is checked per device including fixed allocations, slot allocations and external reservations. These supplied estimates are not physical-fit proof; incomplete telemetry remains unknown (heterogeneous and missing-data tests).
6. Governor replay calls the existing Policy::decide and records intended actions without executing them (policy parity and engine-down tests).
7. Kernel checks have bounded execution/output, numerical tolerances, executable provenance and explicit unavailable/failed outcomes. Emulator wall time never becomes a GPU-performance estimate (negative/timeout/malformed/NaN tests).
8. Reports bind trace/config/executable bytes with SHA256, label synthetic versus recorded inputs and measured versus assumed costs, and leave hardware inference/accuracy unverified unless separately supplied (provenance/report tests).

## Baseline and falsification

Baseline HEAD c3be904; clean source matches prior checkpoint. Existing full Officina suite has two subprocess sandbox failures; shell launcher has baseline ShellCheck findings. No active engine restart/GPU inference authorized by this layer. Reject a policy ranking if replay uses future state, exceeds retention/budget constraints, silently treats unknown memory as free, or misses held-out measured ordering outside configured tolerance. A performance model that cannot distinguish candidates above its own error must report them unresolved.

## Implementation stages

1. Commit a durable research dossier and lab design/contract before code. Keep machine registries/raw traces external.
2. Ship Rust discrete-event cache/slot replay, cost-aware policy comparison and real governor sample replay with synthetic fixtures. Commit passing stage.
3. Add bounded external correctness backend protocol, a CPU oracle, optional NVIDIA Numba simulator adapter and held-out prediction validation. Missing emulator dependencies remain unavailable. Integrate vitriol lab, build/install the CLI, verify examples, document and commit.
4. Freeze source and run Rust suites/Clippy/fmt, explicit Praetor including normally skipped tests, launcher syntax/baseline ShellCheck, docs links/secrets and source-bound review/evidence. Store full evidence outside the repo under /tmp/vitriol-optimization-lab-evidence.

## Tradeoffs and affected contracts

Use existing vitriol-governor library instead of policy duplication. A separate small crate keeps emulation/cost models out of hot inference paths. First replay models whole snapshot entries and declared linear prefill/restore costs, not tensor instruction timing, dynamic KV allocation or cross-GPU overlap; report these omissions. NVML Injection and Accel-Sim are documented optional integrations, not bundled dependencies or deployed services. Kernels changed since a captured trace need representative new validation; no third-party implementation is copied.

- 2026-11-15T10:39:56+01:00 — Stage 1 implemented: independent Rust library/CLI, deterministic completion-before-arrival replay, FIFO/free-slot affinity, bounded two-tier LRU/value cache with protected/reader exclusion, explicit rewind/model/layout gates, per-device memory estimate checks and policy sweeps. Governor replay calls the actual existing library and executes zero actions. Fifteen tests pass, Clippy -D warnings passes and explicit source/test Praetor has zero blocking findings. Both documented synthetic examples execute successfully. Evidence: /tmp/vitriol-optimization-lab-evidence/stage1-*. No runtime engine or governor changes.

- 2026-11-15T11:02:44+01:00 — Stage 2 implemented: independent matvec/dot/softmax oracle, bounded Unix external backend execution with unreaped-child process-group cleanup and 1 MiB output limits, executable/adapter hash integrity, unavailable/error/timeout outcomes, and held-out prediction error/ordering validation. Added synthetic fixtures, actual CLI exit/provenance tests and optional Numba CUDA-Python adapter. Thirty lab tests and full existing governor policy tests pass; Clippy -D warnings, explicit source/test/adapter Praetor, Ruff and Bandit pass. Built and installed ~/.local/bin/vitriol-lab; all five vitriol lab examples execute successfully. Actual optional Numba check returns exit2/unavailable because the package is absent; a package metadata DNS probe failed, so no package install or emulator execution is claimed. Launcher syntax passes; ShellCheck retains68 existing findings, with two diagnostic messages differing only in referenced line positions. Research/operator docs cover schemas, limits and Jev/Laya/AnyJev as evaluated follow-up candidates, not integrated models. Final source-bound evidence will live in /tmp/vitriol-optimization-lab-evidence/final; final checks include scoped evidence.py --base c3be904, lab fmt/Clippy/tests, full governor tests, explicit Praetor with test copies, all operator examples and optional-backend capability result. No active engine/GPU configuration changes.
