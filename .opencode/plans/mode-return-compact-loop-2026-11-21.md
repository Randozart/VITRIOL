# Mode return-stack, compact unification, loop suspend/resume — plan (2026-11-21)

## Problems (traced)

1. **PIPELINE never compacts**: grind's `agent_settled` handler gates on
   `readMode() !== "grind"` → resets state and returns before reaching
   `compactOnFinish`. In pipeline mode the facts file says "pipeline", so the
   entire grind loop (including compact) is skipped.
2. **Grind compact inconsistent**: `shouldCompactOnFinish` requires
   `fillTokens >= 80 000` + 10 min cooldown. Short runs silently skip.
3. **Steer/INQUIRE don't bounce back**: no return-stack concept exists; the
   operator manually switches back. And grind loops **stand down** on a
   steer/inquire interloper (`mode !== "grind"` → state = null), losing round
   count.

## Stages

**S1 — `_shared/mode-return.ts`** (pure): return-stack state machine.
- `pushReturn(state, from, originMode)` — records origin when switching INTO
  steer/inquire from a work mode.
- `popReturn(state, current)` — returns the origin mode to restore, or null.
- Clear rules: ESC/manual pick of a work mode clears the stack; session_start
  clears it.
- Persistence: same facts file the agent-mode extension already writes
  (`agent-mode.json` gains a `returnTo` field) — jiti-safe via the file, not
  module state.

**S2 — wire bounce-back** in agent-mode extension: on `agent_settled` while
mode is steer/inquire and a returnTo exists → `applyModel`-equivalent
`setAgentMode(returnTo)` + notify. Never fires after ESC or a manual work-mode
pick (the pick overwrites returnTo).

**S3 — grind loop suspend/resume.**
`grind/index.ts`: replace the `mode !== "grind"` early-return with:
- mode is steer/inquire → **suspend**: keep `state` (round count, turns,
  signature), set `suspended = true`, paint "↻ grind suspended (steer)".
- mode returns to grind → resume: `state` intact, the loop continues from
  `decision.round`. Pipeline: same treatment with `readMode() === "pipeline" ||
  "grind"` admitted to the loop (S4 makes pipeline a first-class loop mode).

**S4 — compact unification.**
- Grind handler admits `pipeline` mode (`readMode() === "grind" ||
  readMode() === "pipeline"`).
- `shouldCompactOnFinish`: replace the hard `minTokens` floor with
  `minTokens = 0` default + keep cooldown; make the floor opt-in via
  `OFFICINA_COMPACT_ON_FINISH_MIN_TOKENS` (already exists — default changes
  from 80k to 0). Rationale: the floor made compaction unpredictable; the
  cooldown still prevents loops. I-r: compact requests always stamp.
- Compact fires for BOTH grind-done and pipeline-done (same call site, reason
  prefix distinguishes).

**S5 — dream-lane trigger at the sleep moment.**
`sleep-lane`: expose `maybeRunNow(reason)`; grind/pipeline compact call sites
invoke it (fire-and-forget) — the sleep run is the natural "idle after run"
moment and dual-slot makes it free.

## Invariants

- **I1 — bounce-back safety**: bounce-back NEVER fires after the operator
  pressed ESC or picked a mode manually. Pinned: ESC after entering steer → no
  bounce-back.
- **I2 — bounce-back only from work modes**: entering steer/inquire from
  build/grind/plan/pipeline/research sets returnTo; from another steer/inquire
  or at session start it does not. Pinned by push/pop unit tests.
- **I3 — grind resume keeps state**: suspend on steer + return to grind
  resumes with the same round count and continues the loop (not a restart).
  Pinned by a rules-level test: suspended state + mode grind → continue with
  original round.
- **I4 — pipeline compacts**: a pipeline-mode DONE triggers compactOnFinish
  identically to grind. Pinned by handler-level test with facts mode =
  "pipeline".
- **I5 — short runs compact too**: fillTokens floor defaults to 0; cooldown
  still applies. Pinned by policy test (default minTokens === 0).
- **I6 — sleep lane never runs mid-grind**: the dream trigger fires only after
  compact (idle moment), and shouldRun's busy gate still blocks. Pinned by
  trigger-order test.
- **I7 — jiti isolation**: returnTo persists via the facts file, not module
  state. Pinned by two-instance test.

## Named tests
- `mode-return.test.ts`: `push_sets_returnto_from_work_modes`,
  `esc_clears_returnto`, `steer_into_inquire_no_returnto`,
  `pop_returns_origin`.
- `grind/index.test.ts`: `pipeline_mode_compacts_on_done` (I4),
  `suspend_preserves_round` (I3), `resume_continues_loop`.
- `compact-on-finish.test.ts`: `default_floor_is_zero` (I5), `cooldown_still_blocks`.
- `sleep-lane/lane.test.ts`: `trigger_after_compact_only` (I6).
