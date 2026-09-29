# Full re-prefill after cancel / plan-build switch — diagnosis (2026-09-29 08:21 CEST)

Status: documented, NOT yet acted on (user: restart deferred until other work lands).

Symptom (user report): after cancelling a task, or switching between plan and
build mode, the engine re-prefills the ENTIRE context instead of the delta.

## Evidence — engine log `/tmp/opencode/vitriol_gen.log`

Session of unit started 2026-09-28 19:54 CEST (fingerprint
`ts=27,21 c=262144 kv=q4_0/q4_0 spec=mtp:3 par=1 ssp=1 cram=12288
mode=off sparse=sparse score=probe`). 318 requests with `prompt eval` timings.

Prefill-size distribution (tokens actually RE-evaluated per task, i.e. the
delta after `n_past`; extracted with `rg "prompt eval time"` + sort):

```
n=318 min=199 p50=507 p90=1790 max=171671
```

Worst events:

| task | re-prefilled | wall time | preceding log context |
|---|---|---|---|
| 20671 | **171,671 tok** | **683.4 s (11.4 min)** at 251 t/s | 15x `W srv stop: cancel task` (ids 20634, 20672-20690, ~5 s apart) + `disk fetch: restored 3251.6 MiB state (144075 tokens, keep 0.36) from /scratch/kvtier/cache-c8ae38ad0c185be2.bin` |
| 4814 | 52,609 tok | 147.0 s | `selected slot by LRU, t_last = 280061279590` (similarity path failed: f_sim < 0.100) |
| 1030 | 29,648 tok | 81.9 s | same class |
| 140 / 531 / 804 / 15134 | 12-17 K tok | 30-41 s | same class |

Slot-selection health across all 318 requests
(`selected slot by LCP similarity, f_sim_best = X (> 0.100 thold), f_keep = Y`):

```
304 requests f_sim = 0.9x (healthy delta turns)
  2 requests f_sim = 0.1  (f_keep = 0.034 and 0.009 -> stored context ~unusable)
  1 request  f_sim = 0.2
  1 request  f_sim = 0.3  (f_keep = 0.306)
  2 requests f_sim = 0.6
  2 requests f_sim = 0.7
  2 requests f_sim = 0.8
  5 requests fell through to LRU (head divergence or empty slot; 4 had non-(-1) t_last)
```

L0 RAM-cache pressure (`srv alloc: making room for prompt cache entry, removing
oldest entry`) — 6 evictions of states sized **150 MiB, 780 MiB, 2309 MiB,
2973 MiB, 3882 MiB** against `cache_ram = 12288` MiB -> the pool holds only
~3 states; exact-match states get evicted mid-session.

Only one disk-mirror fetch in the whole log (the 171 K event above): Optane
restore DID work (`keep 0.36` accepted, > gate) but 64% of that state was
non-common with the new prompt -> 171 K tokens recomputed anyway.

Deep contexts were live (checkpoint warnings at pos 141K-197K; slot release
`n_tokens = 218136`) — states of 144 K tokens serialize to 3.2 GB
(~22 KB/token, the known `--kv-unified` serialization bloat).

## Code path (inner fork, main @ 29d371878 + uncommitted provenance work)

1. Task pickup (`llama.cpp/tools/server/server-context.cpp:1771-1791`):
   `prompt_save(slot -> L0 + Optane spill)` -> `prompt_load(new tokens)`.
2. Restore gate (`server-task.cpp:1835-1865` RAM, `2309-2408` disk):
   candidate accepted only if `f_keep = lcp(cand, new) / cand.size >= 0.25`
   ("don't trash large prompts"), and only replaces the slot baseline if
   strictly better. No candidate -> slot keeps ITS OWN old prompt (par=1 does
   not clear between turns: the `cache_idle_slots` save+clear at
   `server-context.cpp:2555-2568` only hits OTHER idle slots).
3. Prefix reuse (`server-context.cpp:3336-3344`):
   `n_past = slot.prompt.tokens.get_common_prefix(input)` -> everything after
   the first divergence is re-prefilled.
4. **`--cache-reuse N` (KV-shift salvage past divergence) EXISTS but is NOT
   WIRED**: implementation at `server-context.cpp:3346-3404` (scan cached
   prompt beyond `n_past`, match chunks >= N, `seq_rm` + `seq_add` shift into
   place); flag parsed in `common/arg.cpp:3603`, `common.h:626`
   (`n_cache_reuse`); launcher `scripts/vitriol` never emits it; `~/.vitriol/config`
   has no key; live argv has no `--cache-reuse`. Disabled only for multimodal /
   non-shiftable memory (`server-context.cpp:1256-1271`); text hybrid memory
   reports can_shift=true (`llama-kv-cache.cpp:1290`, hybrid delegates).
   Launcher `--verbose` (debug/TRC logging) exists at `scripts/vitriol:1664`.
5. Slot selection: `slot_prompt_similarity = 0.100` (log line) -> similarity
   path; miss -> LRU (`server-context.cpp:1695-1768`).
6. Cancel: client disconnect logged as `W srv stop: cancel task` (15x);
   child-task release clears prompt (`server-context.cpp:562`);
   decode-error path clears entire slot (`server-context.cpp:3843-3846`).
7. `frozen_prompt` config key -> launcher env `VITRIOL_FROZEN_PROMPT` ->
   consumed ONLY by `libvitriol/vitriol_shim.py` (port-5010 proxy). The shim is
   NOT in the live path (opencode `baseURL` = `http://127.0.0.1:8279/v1`
   direct) and the shim's frozen logic is a no-op ("hash, not used yet").
   Treat `frozen_prompt` as a dead knob today.

## Root-cause hypotheses (ranked, need TRC to confirm)

- **H1 head/middle divergence without chunk salvage**: any request whose new
  prompt diverges from the slot's prompt early (mode switch changing system
  prompt; opencode rewriting mid-history after cancel; alternate conversation
  in the single slot) gets `n_past` = tiny common prefix and NO salvage path
  because `--cache-reuse` is off. Explains f_sim 0.1-0.3 events + LRU falls.
- **H2 L0 eviction**: 3-4 GB states vs `cache_ram=12288` -> exact state lost
  -> disk fallback returns only a partial ancestor (`keep 0.36`) or nothing
  -> full prefill. Explains the 171 K event even though the mirror worked.
- **H3 opencode behavior on cancel**: if cancel/rewind mutates messages away
  from the tail (mid-conversation edit), `lcp` collapses to the edit point and
  `f_keep` can fall under 0.25 -> reject -> full prefill. Unknown whether
  opencode does this — TRC candidate-lcp lines will show the divergence point.
- **H4 mode switch**: opencode plan/build agents (`plan` agent has no own
  `system` in the binary; mode rules also arrive as in-turn
  `<system-reminder>`) — IF the system prompt is byte-stable, mode switch
  should be tail-only; the observed cost implies either system prompt differs
  or the reminder is injected away from the tail. Confirm via TRC on repro.
- **H5 poisoned partial states**: a cancel during prefill saves
  `slot.prompt` (full input tokens) with only partially-computed KV cells
  (`prompt_save`, `server-context.cpp:301-329`). If restored later, `n_past`
  claims more than the cells present. Not yet observed as garbage output, but
  unverified — spot-check a `cache-*.bin` if TRC shows suspicious hits.

## Plan (deferred per user)

**Phase 1 — capture (one restart, bundle with Optane S4 restart):**
1. Add `--verbose` to the unit/launcher launch (TRC: `looking for better
   prompt, base f_keep...` + per-candidate `lcp` + `n_past`).
2. User reproduces both symptoms once (plan<->build switch; cancel mid-prefill
   AND mid-generation), notes approximate timestamps.
3. Correlate in `/tmp/opencode/vitriol_gen.log`: divergence point, candidate
   rejected by the 0.25 gate, L0 eviction vs gate rejection, restore hit.

**Phase 2 — fixes (choose from data):**
- F1 (main, small): wire `--cache-reuse 256` through `scripts/vitriol` +
  config key + fingerprint (speed-bearing key -> provenance). Makes mode
  switch re-prefill only the changed head, salvaging the history body via KV
  shift.
- F2: L0 pressure — measure first with the new provenance counters
  (`kv_cache_disk_*`, `n_kv_cache_mem_entries`); then either raise
  `cache_ram` (vs OOM discipline: engine is OOM-hardened, `oom_score_adj=-500`)
  or lean on the Optane mirror with exact-key spill retention. The 22 KB/token
  state bloat is the known `--kv-unified` serialization issue (separate,
  large project — do not start here).
- F3: cancel path — TRC decides whether opencode mutates mid-history (H3) or
  the gate/eviction alone explains it; verify no poisoned partial states (H5).
- F4 (only if TRC proves it): opencode-side — keep system prompt
  byte-stable across modes / move mode rules fully to the turn tail.

**No code changes until Phase 1 data exists.**

## Open questions for next session

- Timestamps of today's worst stalls (to line up with the 15 cancels).
- `--verbose` bundled into the pending Optane restart, or its own restart?
- Does opencode's outgoing request keep the system prompt byte-identical
  across plan/build? (TRC candidate lcp answers this indirectly.)
