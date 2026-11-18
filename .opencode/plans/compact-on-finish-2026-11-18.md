# Compact when a run finishes (not on a fill threshold)

**Date:** 2026-11-18 17:45
**Owner:** "specifically after a grind or plan is finished."

## Why this boundary

A finished loop is the one moment where nothing is in flight — no tool mid-run, no stream to
interrupt, no partial output worth keeping — and the context is at its fattest, because the run just
produced thousands of tool results. A numeric threshold fires repeatedly as the fill wobbles; the end
of a piece of work happens once.

`ctx.compact(options)` is the pi API for it (`types.d.ts:269`), with `onComplete` / `onError`, so no
RPC plumbing was needed.

## Shipped

- `_shared/compact-on-finish.ts`: policy + guards, as pure functions where possible.
  - `resolveCompactPolicy()` — `OFFICINA_NO_COMPACT_ON_FINISH=1` kills it;
    `OFFICINA_COMPACT_ON_FINISH_MIN_TOKENS` (default 80 000) and
    `OFFICINA_COMPACT_ON_FINISH_COOLDOWN_MS` (default 10 min) tune it.
  - `shouldCompactOnFinish()` — the four guards: enabled, fill known, fill ≥ floor, outside cooldown.
  - `compactOnFinish(ctx, reason)` — stamps `~/.vitriol/officina/compact-on-finish.json` (0600) so a
    re-entered settled handler cannot fire twice for the same run, emits `lc-compact/finish`, and
    calls `ctx.compact()` with an `onError` that surfaces the failure.
- `grind/index.ts`: after the loop's terminal decision (done / blocked / max-rounds), one guarded
  request, with the notice "compacting the finished run".
- `events.ts`: `lc-compact` registered as an event source.

Tests (10): policy defaults and env overrides; the four guards individually; the stamp prevents a
second request inside the cooldown; a refused guard writes no stamp (and never calls `compact`); a
host without `compact()` still reports the request.

## Not done, and why

**PLAN mode has no loop to end.** GRIND ends on an explicit marker (`GRIND: DONE` / `BLOCKED`); a PLAN
turn is just a turn, so "the plan is finished" is not an event an extension can observe. The honest
equivalent is **leaving PLAN mode** (a mode switch out of `plan`), which needs a watcher on
`~/.vitriol/officina/agent-mode.json` — worth doing, but it is a different trigger with a different
failure mode (the operator might toggle modes to peek).

Second caveat worth remembering: the finished run is the *deepest* context, so a **local** summary
runs at the ~1.2 tok/s decode rate measured on 2026-11-17 — 30–60 quiet minutes. That is acceptable
precisely because it happens when work is done, and it is the case `small-lane`'s cloud summariser
exists for.
