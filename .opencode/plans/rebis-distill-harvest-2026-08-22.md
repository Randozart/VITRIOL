# REBIS distillation harvest — D1 collection mechanism

**Date:** 2026-08-22 06:00
**Status:** implementing
**Goal:** continuously capture every Rebis run — clean and poked alike — as
training-grade data. Conversion to training (SFT/DPO) deferred; storage format
chosen so both are derivable later.

## Design

- Storage: `~/.vitriol/distill/<task_id>.jsonl`, one line per event,
  flushed immediately (crash-safe partial records still usable).
- Local-only policy: records embed repo code — never commit or sync.
- Recorder: `DistillRecorder` in rebis.py; `--distill-dir` (default
  `~/.vitriol/distill`) + `--no-distill` opt-out.

### Events

| event | payload |
|---|---|
| run_open | task_id, ts, packet digest (objective/invariants/constraints/output_contract/slice paths+content), draft_mode, verify_mode |
| draft | iteration, full drafter text, token usage |
| files_before | path → content snapshot of touched-candidate paths |
| files_after | path → content post-apply |
| gate | compile_ok + error digest |
| verdict | pass/wellformed/delta[] (llm mode) |
| fragment_rejected / patch_failed / replace_failed | paths/reason/report |
| run_close | accepted, wall_s, per-model token totals |

Captured everywhere: rebis_loop (all exit paths incl. pauses),
baseline_run single-shots, shim steered turns (`shim-events.jsonl`:
flags, judge verdict, original vs final response).

## Tasks

1. DistillRecorder class + CLI wiring
2. Event hooks across rebis_loop branches (file/patch/replace)
3. baseline_run capture
4. Shim capture on flagged/steered turns
5. Selftest: emit sequence → parse-back assertions
6. Live verify: fresh S1 replace-mode run → inspect produced JSONL

## Progress log

- 2026-08-22 06:00 — plan written; implementing.
- 2026-08-22 07:30 — **D1 COMPLETE.** DistillRecorder wired through all three
  delta-protocol branches + all exit paths (accept/pause/cap); baseline_run
  captures single-shots; shim logs shim_judged/steer_nudge/steer_override to
  `shim-events.jsonl`. CLI: --distill-dir/--no-distill.
  Live-verified: failure trajectory (22 events w/ full 13-15KB drafts) AND
  success trajectory (15 events incl. 8177B→8500B before/after snapshots,
  gate green, run_close accepted; 152/152 tests after loop re-applied S1).
  Known wrinkle: same-task-id reruns append to one JSONL — split records at
  run_open boundaries when converting to training format.
- Data location: ~/.vitriol/distill/ — LOCAL ONLY (embeds repo code).
