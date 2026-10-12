# Restored-State Overshoot Fix: Session-Switch Full Re-Prefills

**Date**: 2026-10-12 (evening)
**Status**: APPROVED (owner confirmed the 16:00-17:40 hangs; "write a plan
and execute")

## Incident

Owner report: long subagent run "wiped" the main agent's cache. Evidence
(gen log, uptime-stamped, boot 15:15): five full re-prefills of
127k-164k tokens (16:01, 16:12, 16:37, 16:51, 17:36), 13-16 min each:

```
restored state diverged beyond rollback depth (tail = 132391), trying checkpoint ring
forcing full prompt re-processing due to lack of cache data
```

`/cache` proves nothing was lost: RAM tier holds both session states
(main 144k tok / 6.2 GB, subagent 127k tok / 3.0 GB).

## Root cause

`restored_covered` (committed today as inner `9b11ccb27`, logic inherited
from prior WIP) gates on the WRONG quantity:

- it computes `tail = prompt.size() - n_past` = **prompt growth** (new
  client content), and demands `tail <= n_rs_seq (3)` for recurrent
  memories;
- but growth needs NO removal: a shorter restored state is a strict
  prefix of the new prompt; the engine appends the tail, which
  recurrent states do natively (recurrent `seq_rm` with `p0` past the
  state end is a no-op returning true);
- the quantity that requires removal/rollback is **state overshoot**:
  how far the restored state extends PAST the matched prefix (only when
  the state is longer than the match does `seq_rm` need to rewind).

Consequence: every tier-restored state lagging the new prompt by >3
tokens - i.e. every main<->subagent switch - failed the check; the
checkpoint ring (min step 8192) could not cover small gaps; do_reset
re-processed the ENTIRE prompt. The subagent's long run made each switch
catastrophic.

## Fix

In the restored-state branch, compute both quantities and gate on
overshoot only:

- `overshoot = max(0, llama_memory_seq_pos_max(mem_tgt, slot.id) - n_past + 1)`
  (cells needing removal; pos_max across hybrid components is the
  conservative value, and restored whole states end at the same position
  in every component anyway)
- `growth = prompt.size() - n_past` (pure append - always safe)
- coverage: `PART: always` / `RS: overshoot <= n_rs_seq` /
  `FULL|NO: overshoot == 0`

Expected effect on tonight's five events: subagent turn 1 stays one
honest 127k prefill (genuinely new content); every subsequent switch
becomes delta-only (132k -> ~5k, 136k -> ~4k, the 207-gaps -> ~1 s).

## Verification

1. Build + restart via unit; fingerprint diff (config unchanged).
2. Regression: identical-resend triple (A/A/A) still 200/200/200 (that
   path is a true overshoot: state longer than prompt after decode;
   ring miss -> honest full re-prefill, cheap).
3. Switch probe: two alternating chat "sessions" with distinct large
   prefixes; on each return confirm `cache_n` ~= previous session total
   (delta-only) and NO `forcing full prompt re-processing` lines for
   pure-growth cases in the gen log.
4. Docs: report + EXPERIMENT_LOG + AGENTS behavior-contract line update
   ("history rewrites re-process honestly" -> growth appends; only
   overshoot is rollback-bounded).

## Non-goals

- 2.25x state-serialization bloat (42.9 vs 19.1 KiB/token observed) -
  separate workstream, noted.
- Subagent slot isolation, cram sizing - not needed once switches are
  delta-only.

## Result (same evening)

Fixed + verified (inner `f5fbc4e44`, fingerprint 18:02:57 config
unchanged):

- Switch probe: S1t2 cache_n=378/prompt_n=26, S2t2 cache_n=338/prompt_n=26
  - full state match, 26-token delta appends across session switches.
- True-overshoot cases still honest: S2t1 (overshoot 373, new session)
  and A2 identical-resend (overshoot 12) fell back to full re-prefill;
  A/A/A regression 200/200/200, health OK.
- Log contract: "restored state overshoots the match (overshoot = N,
  growth = M)" INF line on fallback; covered growth cases proceed
  silently (TRC).
- Net effect on tonight's incident class: every main<->subagent switch
  is now delta-only (the five 127-164k re-prefills become ~1-5k deltas;
  subagent turn 1 remains one honest prefill).

Serialization bloat (42.9 vs 19.1 KiB/token) noted as separate
workstream. AGENTS.md behavior contract updated.
