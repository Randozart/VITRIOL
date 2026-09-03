# Context lifecycle: sparse eviction restore + preservation contract

**Date:** 2026-09-03 15:30 UTC
**Owner:** "I'm pretty sure it might be hoarding too much context" →
  investigation → **heresy found**: the daily driver lost sparse KV
  eviction entirely, silently. Owner clarifications: cadence 4 was
  REJECTED (ejected too aggressively); certified 16 is the restore value;
  ejection itself is wanted ("irrelevant context is removed"); the OLD UI
  had an ejected-tokens counter — lost in the Ratatui port; scratchpad +
  todo are the preservation layer that makes ejection safe ("The
  Scratchpad was created to help keep context around"); ejection should
  dump context "the moment it's certain not to be needed."

---

## The heresy (evidence trail)

| date | event |
|---|---|
| 2026-08-24 | lull certification: sparse+probe certified (96,836 tok @ 11.32 t/s), VITRIOL_KV_SCORE=probe |
| 2026-08-24 17:55 | lull build dir binary — PREDATES the probe code (strings: 0 hits) |
| 2026-08-31 | F2: launcher unit overrides REMOVED ("profile [kv] is canonical") — VITRIOL_KV_SCORE=probe dropped from launches |
| 2026-08-31 10:37 | main-tree rebuild (post ggml-org consolidation) — libllama.so.0.3.0 contains ZERO VITRIOL_KV_MODE/VITRIOL_KV_SCORE strings |

**Net effect since Aug 31:** config says `[kv] mode = sparse`, the
fingerprint prints `sparse=/score=`, but the running engine has no sparse
code at all — a SILENT NO-OP. The KV cache fills to the brim with zero
ejection; only pi's threshold compaction (window − 16k) relieves it.
Classification: silent flag drift — the exact failure class AGENTS.md's
vendor-patch rule warns about. The sparse/probe implementation survived
ONLY in the `VITRIOL-lull` worktree (`llama-kv-cache.cpp`: evict_sparse +
attention-probe scoring; 935-line divergence vs main).

**The pendulum, explained:** "ejected too aggressively" = the blind
middle-eviction era (VITRIOL_KV_SCORE off → arbitrary ordering), plus
cadence-4 experiments. "Holds on to too much" = no ejection whatsoever.

## Mechanism (lull source)

- `evict_sparse(n_needed, n_sinks=4)`: fires ONLY when the cache is full;
  evicts globally-lowest probe scores among evictable middle cells;
  preserves attention sinks; probe scores = q·K softmax per GQA-representative
  head, exponential-decay accumulated (λ=0.90/step), every SCORE_EVERY
  decode steps (default 16)
- "Certain not to be needed" = decayed score below a floor, sustained —
  currently there IS no floor: ejection waits for cache-full pressure

## Stages

1. **Restore**: rebuild lull engine (sm_61;86) → install into the
   launcher path → verify `VITRIOL_KV_SCORE: probe active` + eviction
   stderr lines at cadence 16.
2. **Launcher**: `[kv] score = probe` profile key → env export
   (+ SCORE_EVERY=16 default); **guard**: config requests sparse + engine
   lacks VITRIOL_KV_MODE strings (binary AND libllama.so) → refuse to
   launch, loudly.
3. **Ratatui ejected counter**: parse diag stderr eviction lines →
   cumulative per-boot `⤓ Nk ejected` on the ctx sidebar row (old-UI
   parity).
4. **Eviction contract**: the task-state tail (re-injected every turn)
   gains one line — the engine ejects unattended context; anything needed
   later must live in scratchpad or tasks. Converts scratchpad/todo from
   sidebar decoration to motivated use. Preservation plumbing verified:
   scratchpad re-injects every turn (facts/context/leads/dead); task tail
   injects 15/40 with dedup.
5. **Eager floor sweep (engine, lull)**: after each scoring pass, evict
   cells with decayed score < VITRIOL_KV_FLOOR (default OFF), protecting
   sinks + VITRIOL_KV_PROTECT recent tokens (default 2048). Holes feed
   existing defrag. Gate: observe Stage 3's counter first, then enable.
6. **Port lull-kv → main** (standing): llama-kv-cache divergence (935
   lines) + vitriol-kv-probe.* + context plumbing + the launcher guard —
   so consolidation can never strand the feature again.

## Drift guard

Any future engine rebuild: the launcher guard (Stage 2) fails launches
when sparse is configured but unsupported. AGENTS.md gains a pointer to
this report if the port (Stage 6) lands.
