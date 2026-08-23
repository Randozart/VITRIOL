# REBIS sweep workshop + gateway timeout fix

**Date:** 2026-08-22 11:45
**Status:** implementing

## Problem set

1. **Hermes timeouts through Mercury**: reason-route turns took 134.8 s
   measured — Sol thinks unbudgeted (`enable_thinking:true`, no cap); hermes'
   HTTP client dies long before completion.
2. **CONTROLS clutter**: RunSweep/SweepAndSave duplicated per profile; sweeps
   were designed profile-independent.
3. **Sweeps need a home**: model/GPU/memory selection + cache-pressure-aware
   feasibility, searching for max tok/s — a dedicated tab, not list rows.

## Fixes

### C. Gateway timeout (ship first)

- Sol launch gains `--reasoning-budget 1024`: thinking hard-capped (~50 s
  worst case at Sol speed); env-tunable `REBIS_REASONING_BUDGET`.
- Reason route becomes a **live SSE relay**: chunks stream Sol→client as
  generated, so the connection never idles — no client timeout can fire
  mid-generation regardless of turn length. Bonus: users watch Sol think.
- Fallback: non-streaming clients get the buffered response as before.

### A. CONTROLS declutter

Remove RunSweep/SweepAndSave pushes from `Action::all()` (variants kept for
the sweep tab's internal reuse). Tests updated.

### B. `Tab::Sweep` workshop

Form fields:
- **model path** (text field; defaults scanned from ~/Downloads + ~/.models)
- **GPU target**: Sol-card / Luna-card / split-ratio cycler
- **context size** preset cycler (8k/16k/32k/64k)
- **min free memory** MiB field

Feasibility pre-check before any run: GGUF file size (weights) + estimated
KV (@ctx, 32 KiB/tok conservative) must fit target VRAM minus min-free;
infeasible configs are refused with the arithmetic shown.

Runner: spawns `libvitriol/sweep_controller.py` (existing engine: per-config
llama-server lifecycle + tok/s benchmark) as a subprocess on **port 8290**
(Mercury collision avoided), streaming stdout lines into the TUI; parses the
CSV output into a results table sorted by tok/s, winner highlighted.
Results also appended to the distill store.

## Progress log

- 2026-08-22 11:45 — plan written; implementing C → A → B.

## Progress log (cont.)

- 2026-08-22 12:30 — **ALL THREE SHIPPED.**
  - C: Sol --reasoning-budget 1024 (launcher env-tunable); reason route
    streams SSE live — first chunk 1.15s measured, connection never idles,
    client timeouts structurally dead.
  - A: CONTROLS = start/stop/restart/doctor/setup/launch-rebis. No sweeps.
  - B: SWEEP tab live — model path typing, GPU/CTX/MIN-FREE cyclers,
    feasibility line (✓ fits / ✗ REFUSED with arithmetic), controller output
    pane streaming sweep_controller.py on :8290.
- Tests: 152 pass. Commits: 3b9055f (routing fix), fea5aa3+e78ce34 (earlier),
  plus this session's shim/sweep commits.

## Live-session incident fixes (2026-08-23, user's first hermes shakedown)

Access logs + hermes screenshots decoded the drop chain:
1. hermes aux title-gen (tiny no-tool request) routed to Sol full-depth,
   queued 160-260s behind the streamed main generation → client timeout.
2. Co-tenant killall killed Sol mid-retry → 55s connection-refused window
   (15s supervisor backoff + ~40s model load) → APIConnectionError ×3 → drop.
3. hermes session-only /model switch assumed a 256k context (config entry
   advertised the Mellum GGUF id, not `rebis`) → window mirage.

Fixes shipped:
- hermes config: REBIS-GATEWAY advertises model `rebis` (+ per-head ids) at
  65536 — window truth restored.
- Gateway aux fast-path: no-tools + max_tokens ≤400 + short prompt → Luna.
  Title-class turns: 160-260s → 9.4s (validated).
- Gateway resilience: connection-refused → one 3s retry → clean
  **503 + Retry-After: 30** with human-readable body (validated: 3.2s to
  clean 503 with Sol killed; post-respawn turn 4.6s).
- Reason route respects client max_tokens (removed the 4096 floor that
  turned agent turns into marathons).
