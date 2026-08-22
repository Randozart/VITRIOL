# REBIS Phase 2–4 — agentic durability

**Date:** 2026-08-21 21:30
**Status:** executing
**Prior:** `.opencode/plans/rebis-phase0-plan-2026-08-21.md` (Phases 0–1 PASSED)
**Goal:** make the Rebis loop survive real agentic use — long sessions, crashes,
multi-file edits, no human per turn.

## Decisions (user-selected)

- Anticipatio slot topology: empirical — measure per-slot ctx; `-np 2` @16k if the
  4-slot default splits context; whichever works.
- Agentic surface: opencode provider + subagent recipe first (OFFICINA later).
- Acceptance tasks: three small self-contained Rust changes in VITRIOL itself.

## Phase 2a — rebis.py v2 (loop hardening)

1. **Verifier strictness**: verdict = `{pass, checks:[{invariant, holds, evidence}],
   delta[]}`. `pass=true` with any invariant missing from checks ⇒ coerced fail.
   Closes shallow passes (Box::from_raw provenance slip, 2026-08-21 21:05 log).
2. **Multi-file Mandatum**: `file_slices[]` (single `file_slice` still accepted);
   drafter emits one `### <path>` header + fenced block per file; extractor splits,
   writes all. Single-slice fallback when no headers found.
3. **Journal + resume**: `/tmp/opencode/rebis-journal/<task_id>.jsonl`; every event
   appended; `--resume <task_id>` reconstructs iteration/delta state and continues;
   terminal event short-circuits.
4. **Fault tolerance**: per-call timeout + 1 retry on timeout/5xx; health check on
   repeated failure; optional `--drafter-spawn` / `--verifier-spawn` commands run
   when a server is down; wall-clock budget `--budget-s`.
5. **Accounting**: capture `usage` from both models per call; TurnRecord carries
   tokens; final report JSON (`--report`) totals drafter/verifier spend and wall time.

## Phase 2b — Anticipatio

6. Slot topology measurement on live server (/props, probe >8k prompt through one
   slot); relaunch decision: keep 4-slot vs `-np 2` @16k/slot. Data decides.
7. Async shadow-prefill thread fires stable-prefix (`max_tokens=1`,
   `cache_prompt=true`) at Mandatum send; `probe-ttft` mode measures cold-vs-warm
   TTFT delta on identical prompt.

## Phase 3 — opencode surface

8. `mellum-think` provider in `~/.config/opencode/opencode.jsonc`, cloning the
   qwen38-mtp pattern, baseURL http://127.0.0.1:8287/v1.
9. Rebis subagent recipe doc: planner=Qwen provider, worker=Mellum provider,
   compiler gate via bash tool; agents invoke `rebis.py --task`.

## Phase 4 — acceptance battery (real VITRIOL tasks)

10. Three self-contained Rust tasks in this repo (picked at execution: libvitriol
    estimator edge case, gguf_reader fix, scripts helper) + drills:
    - injected-known-bad draft ⇒ measure verifier false-pass rate (must be 0)
    - kill -9 both servers mid-loop ⇒ journal resume must recover
    - long-session growth run (≥12 consecutive turns)
11. Metrics: iterations-to-green, false-pass rate, recovery yes/no, tokens-per-green.
    Report → `.opencode/plans/rebis-phase4-report.md` + EXPERIMENT_LOG.md.

## Risks

- Per-slot ctx split may cap Mandatum prefix size at 8k until relaunch (2b.6).
- opencode provider for a Thinking model may surface `<think>` in content — agent
  prompts must tolerate reasoning text (strip_thinking already exists in rebis.py).
- Real VITRIOL tasks touch GPL code the drafter will rewrite wholesale — review
  diffs manually; loop output is draft-grade, not commit-grade.

## Progress log

- 2026-08-21 21:30 — plan written; starting 2a implementation.
- 2026-08-21 22:30 — **Phase 2a COMPLETE.** rebis.py v2: strict evidenced verdicts,
  json_schema-constrained `/completion` verdicts (150–570 tok vs 8192 ramble),
  multi-file Mandatum, journal + `--resume` (drilled live), budget-bounded calls,
  retry/respawn, usage accounting + report JSON. Five bugs found+fixed in drills
  (details in EXPERIMENT_LOG.md). Operational requirement discovered: servers under
  agentic load MUST set `--cache-ram` (fork prompt cache default 8192 MiB is an OOM
  vector — killed Qwen server during drill). Sound smoke task accepted iteration 1,
  runtime-test verified.
- Next: Phase 2b Anticipatio (slot topology check → async shadow prefill → TTFT probe);
  then Phase 3 opencode provider; then Phase 4 battery on real VITRIOL tasks.
