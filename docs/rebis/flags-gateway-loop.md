# REBIS flags — gateway & loop

Mercury gateway (:8280) and the Mandatum loop (`rebis.py`) knobs.

## Gateway modes

`--mode gateway` (default) routes every turn: kickoff/planning → Sol,
executor continuations → Luna fast path, finalizing turns → the draft-audit
pipeline. `steer` is legacy watch-only; `passthrough` a dumb proxy.

## Routing ladder

1. tools attached + no assistant tool activity yet → **Sol** (planner turn;
   catches Luna's under-initiation before it ships)
2. last message is a tool result → **Luna** fast path
3. assistant finalizing after tool work → **pipeline** (draft-audit)
4. no toolset at all → **Sol** (quality-first; bare-chat Luna drafting
   degenerates without harness structure)

Escape hatches: request `model: rebis-qwen` / `rebis-mellum` to force a head.

## Draft-audit pipeline

Luna drafts while a warm thread feeds `stable prefix + draft-so-far` into
Sol's cache every ≥1024 new characters — audit prefill becomes nearly free
(measured 46.95s → 0.06s for full-prefix reuse). A constrained JSON verdict
gates the answer; on failure Sol authors a correction natively in OAI
tool-call format.

Caveat: warming only survives on single-client endpoints. Interleaved
conversations from other clients evict cached states.

## Loop protocols

| draft_mode | use when |
|---|---|
| file | target files ≤~250 lines (whole-file emission fits budget) |
| replace | real-file modification: SEARCH/REPLACE blocks anchored verbatim |
| patch | unified diffs; requires verbatim hunk context discipline |

`verify_mode compiler_only` skips the LLM auditor when the compile gate
enforces every invariant (test-emitting invariants). `llm` adds Sol's
evidence-or-fail audit for semantic invariants.

## Loop safety knobs

- `--budget-s`: wall-clock ceiling; aborts pause resumably via journal
- `--resume TASK_ID`: continue after pause/crash
- fragment guard: drafts under 25% of an existing file are rejected as
  truncations; first overwrite leaves a `.rebis-bak`
- `--drafter-spawn/--verifier-spawn`: auto-respawn dead heads

## Packet keys that change behavior

`draft_mode`, `verify_mode`, `draft_budget` (token cap override),
`max_iterations`. Authoring rules: `libvitriol/REBIS-GUIDE.md`.

PROVENANCE: implemented in libvitriol/rebis.py + rebis_shim.py this repo;
measurements EXPERIMENT_LOG.md 2026-08-22 entries.
