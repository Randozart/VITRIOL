# REBIS day-long readiness — rolling windows, compaction, resilience

**Date:** 2026-08-22 08:45
**Status:** plan approved for execution (user requirement: day-long deep
systems-engineering sessions are the normal operating condition)

## Requirement analysis

A day-long session at sustained agent traffic produces rough token budgets:

| source | rate | 8h estimate |
|---|---|---|
| harness prompt re-prefill | ~19k tok/turn cached post-H1 | bounded by reuse |
| new conversation content (turns + tool results) | ~2–8k tok/turn | **100k–400k tokens** |
| tool results (file dumps, logs) | dominant share of above | grows super-linearly on debugging |

Against a 64k window this means: overflow is not an edge case but the steady
state. Three mechanisms must cooperate:

1. **Rolling window** (`--context-shift`): graceful degradation when full —
   oldest context silently dropped. Necessary safety net; insufficient alone
   because dropped context is *lost*, not remembered.
2. **Compaction**: semantic preservation — old turns replaced by an LLM-written
   digest so their *meaning* survives while their *tokens* are freed.
3. **RAM discipline**: long sessions multiply checkpoints (~150 MB each on big
   contexts, up to 32/slot) and prompt-cache states; unbounded growth was a
   proven OOM vector.

## Considerations and reasoning

### Why compaction lives in the gateway, not the harness or servers
- Servers only see tokens; they cannot decide what is *semantically* worth
  keeping. The gateway sees whole conversations and owns Sol — the smart head
  that can write digests.
- Harness-side truncation (hermes' own window handling) discards without
  summarizing; it also happens after the shim, so distill records would lose
  the compacted view.
- Doing it in the gateway keeps both heads synchronized on the same compacted
  view, which matters because Luna's continuation quality depends on seeing
  the same context Sol reasoned over.

### Threshold and placement
- Trigger: estimated history > `--compact-threshold` (default 48000 ≈ 75% of
  the 65536 window). Estimation via `/tokenize` (exact), not chars/4.
- Keep-window: most recent `--keep-recent` (default ~10000 tokens) stays
  verbatim — active work must never be summarized.
- Digest placement: injected as a `system` role message directly after hermes'
  own system prompt; fallback to prefixing the first user message if any head
  mishandles multi-system histories (verified during testing).
- Recursion guard: digests carry a marker; later compactions merge into the
  newest digest rather than summarizing summaries of summaries.
- Cache cost honesty: compaction rewrites history → exactly one cold full
  re-prefill afterward (~60–90 s at threshold size). Infrequent by design;
  the alternative at 64k is death.

### Tool results inside digests
File paths, edit outcomes, and command exit codes are preserved verbatim
inside digest text even when prose is compressed — future edits anchor on
paths and results, and losing them causes repeat-tool-call loops.

### Rolling vs compaction interaction
`--context-shift` may drop tail tokens before the gateway ever sees a
threshold crossing. Mitigation: compaction triggers well below the window
(75%), so under normal operation shift never fires; it remains purely as
overflow insurance.

### Respawn resilience
The other tenant on this box kills llama-servers between runs (observed).
Day-long sessions therefore need the gateway to detect dead backends and
relaunch them from spawn-command templates (`--sol-spawn`, `--luna-spawn`),
mirroring rebis.py's ensure_server pattern. Health surfaced on the gateway's
own `/health`.

### Checkpoint RAM bound
VITRIOL creates up to 32 checkpoints/slot (~150 MB each at large ctx).
For day-long runs: raise `--checkpoint-every-n-tokens` (fewer, larger gaps)
and investigate lowering the 32-slot cap if host RAM creeps. Watch `free -m`
during the long-session simulation.

## Work items

| item | kind | notes |
|---|---|---|
| M1 rolling flags restored | config | both launches; safe post-H1 |
| M3a canonical launcher script | scripts/rebis-servers.sh | blessed flags encoded once |
| M3b checkpoint spacing | config/flag | reduce frequency; measure RSS drift |
| M2 gateway compaction | feature | tokenize → threshold → Sol digest → splice; CLI knobs; distill events |
| M3c gateway /health + respawn | feature | backend status; spawn templates |
| validation | test | scripted >64k synthetic-session simulator through :8280 |

## Risks

- Digest quality is Sol-dependent; bad summaries corrupt downstream reasoning
  more subtly than truncation. Mitigation: keep recent window generous,
  record compactions in distill for inspection.
- Post-compaction cold prefill (~60–90 s stall). Mitigation: anticipatio-style
  warm of the NEW compacted history before shipping the next real turn.
- Interaction bugs between shift and compaction firing together. Shift should
  never fire if compaction thresholds are correct; simulator must verify.
