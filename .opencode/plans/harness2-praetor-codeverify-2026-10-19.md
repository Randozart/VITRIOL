# Harness² × Praetor: deterministic self-improving coding harness

**Created**: 2026-10-19 11:50
**Status**: approved (user: "wire it in, to make our super harness")
**Repo**: `~/Desktop/Projects/harness2` (venv `.venv`, `.env` wired to local engine)

## Goal

Turn harness2 from "tool without a signal" into a measurable, local, zero-API-cost
improvement loop for coding harnesses, with Praetor as the deterministic scorer and
the enforceable layer. Output: adopted harness edits + evidence-backed lessons that
flow back into officina/opencode.

## Decisions (defaults taken; user may override)

- **Task suite**: start with **Praetor's own repo** (small, Rust, `cargo test` +
  self-verifying + Praetor-validatable), then add VITRIOL tasks (C++, daily work).
- **Metric**: tests pass = hard gate (correctness); `praetor validate --json`
  `total_diagnostics` = optimization target (slop/quality); tokens + wall time =
  telemetry only.
- **Scope**: M1 now (bench + deterministic judge + smoke), M2 designed (enforcement
  A/B), M3 last (transfer into officina).

## Architecture

```
harness2/
  benchmarks/codeverify.py         BenchmarkAdapter "cv"  (this plan)
  benchmarks/codeverify_runner.py  per-rollout runner (stages repo, calls opencode)
  cli/_common.py                   BENCHES += "cv"; make_adapter branch
  cli/setup.py                     choices += "cv" (verifies praetor + task suite)
~/.local/share/harness2-cv/tasks/<task_id>/
  task.json    {prompt, domain, test_cmd, praetor_targets[], timeout}
  repo/        the workspace the agent edits (bug-injected or feature-gap state)
  solution/    optional reference patch (never exposed to the agent)
```

Scoring (`adapter.score(run_dir, task_id)`, deterministic, no model call):
1. `test_cmd` in the post-rollout workspace → pass/fail (hard criterion)
2. `praetor validate --warn --json --target <praetor_targets>` → `total_diagnostics`
   count vs baseline (second criterion: no new diagnostics)
3. `n_criteria=2`, `n_passed = tests_ok + praetor_ok`; raw carries both numbers

Anti-gaming: run Praetor **post-hoc with a fixed config**, on a clean copy, ignoring
in-repo suppression files; count suppressed diagnostics separately if present.

## Interfaces used (verified in repo)

- `BenchmarkAdapter` protocol — `core/spec.py:196` (tasks, domain_of, task_prompt,
  execute, delivered, trajectory, base_harness, validate_tree, score)
- `BenchSpec` — `core/spec.py:132` (name, runs_root, install_mode, prompts, channels,
  domain_word, leak_globs, obs_trunc>=10k)
- Adapter dispatch — `cli/_common.py:45` `make_adapter`; registry `:30 BENCHES`
- Judge plumbing — `core/judge.py:61` `judge_one` → `adapter.score`; CLI `cli/judge.py`
- Substrate — `substrate/opencode.py` (`OPENCODE_SURFACE`, install modes, launcher)
- Praetor CLI — `praetor validate --warn --json --target DIR` → pure JSON on stdout
  (`{"passed","total_diagnostics","failures":[...]}`); `praetor report`; `praetor verify`

## Milestones

**M1 — bench + deterministic judge (this session) — DONE**
- `codeverify.py` + `codeverify_runner.py` + registry wiring + task-suite loader
- Wiring: `cli/_common.py` (BENCHES + adapter), `cli/setup.py` (opencode-only install),
  `cli/check.py` (choices), `preflight.py` (cv branch: praetor + opencode, no bench repo)
- OpenCode toolchain installed (`dependencies/opencode` @ pin 23fb5e05, bun 1.3.11)
- Smoke task `py-ledger` (unittest, bug = positive-only filter) + `_base_harness` tree
- **Verified** (stub agent, no model): stage → run → diff → grade end-to-end.
  untouched 1/2 (tests fail) · solution 2/2 · delete-test gamed 1/2 (tamper detected) ·
  suppression-registry gamed 1/2 (registry stripped on clean copy) · harness render
  (AGENTS.md + .opencode/skills) lands in the workspace · `harness2-check --bench cv`
  returns Ready
- Remaining for a real rollout: local engine up on :8279 (GPU currently held by the
  user's own server), then `harness2 --bench cv --k 1 --mode parallel` → judge → report

**M2 — enforcement A/B**
- Two harness variants: baseline vs Praetor-aware (LSP active, pre-commit gate,
  clean-diagnostics instruction). Probe on the same tasks; adopt on measured delta.
- Answers the AGENTS.md doctrine question (enforcement vs advice) with numbers

**M3 — closure into officina**
- Adopted harness + lessons → officina skills/AGENTS.md; lessons in
  `cue/lesson/evidence/confidence` form

## Files

- `~/Desktop/Projects/harness2/harness2/benchmarks/codeverify.py` (adapter, judge)
- `~/Desktop/Projects/harness2/harness2/benchmarks/codeverify_runner.py` (rollout)
- `~/.local/share/harness2-cv/tasks/{_base_harness,py-ledger}/` (suite)
- `.gitignore` added to the clone (dependencies/venv/runs/env excluded); local commit
  `f54900c`

## Risks

- **Gaming**: agents can suppress diagnostics (`suppressor.rs`) → judge runs a clean
  fixed-config pass (see above)
- **Suite quality decides everything**: 5-10 tasks, ground truth from existing tests;
  too easy saturates, too hard adds noise
- **Compute**: 27B @ ~25 t/s; 10 tasks × 3 variants = hours → overnight, per-rollout
  timeout
- **Praetor early-stage**: expect noisy diagnostics on C++26 initially; start with
  Praetor's Rust repo (most mature target)
- **Executor network-unrestricted**: isolate benchmark runs

## Provenance

harness2 is Apache-2.0 (compatible). Praetor is fork-owned (Apache-2.0). No new
third-party code. All new files carry the Apache-2.0 header style of harness2.
