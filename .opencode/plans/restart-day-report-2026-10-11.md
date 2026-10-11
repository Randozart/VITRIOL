# Restart-day report: governor canary + persistence chain live

- **Date:** 2026-10-11 12:4x CEST
- **Executor:** opencode, restart authorized by owner ("safe to restart and test now")
- **Engine:** systemd `vitriol-server.service`, fingerprint unchanged except
  `cxr=1` (launcher FLAG-DELTA confirms: `cxr: <new> -> 1` vs previous launch)

## Deployed state (all systemd, all active)

| unit | state | note |
|---|---|---|
| vitriol-server | active | binary 07ca8191d+ (hybrid fan-out, restore-bypass), `VITRIOL_KV_DISK_PRUNE=0`, `VITRIOL_CTX_RESIZE=1`, OOMScoreAdjust -500 |
| vitriol-autosave | active | `VITRIOL_SLOT_SAVE_DIR=/scratch/kvtier` via drop-in; restores slot0.bin at boot (1533 tok in 350 ms), 60 s autosave |
| vitriol-governor | active, enabled | config `~/.vitriol/governor.toml` (absolute paths - systemd has no HOME), phase=Full, watching |

## Live canary results

1. **Fingerprint discipline**: exactly one key added (`cxr=1`); launcher
   FLAG-DELTA confirms; no other drift.
2. **KV residency**: warm pair cache_n=1002/1002 (34 t/s MTP decode).
3. **/ctx/resize live**: 262144 -> 212992 -> 262144, slot spooled each way,
   session warm-restored (cache_n=1002 post-resize). **VRAM returned to the
   driver: 22653 -> 21061 MiB (1592 MiB freed), reclaimed on restore.**
4. **/kv-policy live**: floor/protect set + echo + undo works;
   `buffer_bytes=4831838208` after the hybrid-iswa fan-out (was 0).
5. **Crash-resume chain** (the original complaint):
   - sidecar boot restore: 1533 tokens / 71-350 ms
   - exact-prefix request after crash+restart: **cache_n=1388** (was 0
     before today's work - full re-prefill)
   - diverged request (tail 20 > n_rs_seq 3): honest re-prefill, logged,
     NO abort (first blanket version aborted in common_memory::seq_rm -
     core dump, systemd recovered, fixed by the rollback-depth clamp)
6. **Governor live detection**: free=[798,303] MiB, foreign=false (desktop
   residents correctly below the 768 MiB tenant floor), phase=Full.
7. **kvtier smoke harness: ALL PASS (5/5)** incl. the new T4 kv-policy and
   T5 resize round-trip tests.

## Bugs found and fixed during the canary (inner 07ca8191d)

- kv-policy helpers missed `llama_memory_hybrid_iswa` (the daily driver's
  memory type) -> RELIEVE tier was a silent no-op live. Fixed with
  `vitriol_kv_caches()` fan-out (plain / iswa / hybrid / hybrid-iswa).
- do_reset gate discarded whole-state restores on hybrid/recurrent:
  `seq_pos_min()` = max over components can never return to 0 once the
  session exceeds the SWA window, so no checkpoint can cover the front.
  Restores now mark the slot and the gate accepts them as coverage,
  clamped to rollback depth (RS: tail <= n_rs_seq; FULL/NO: exact
  continuation only; else honest re-prefill).

## Owner-visible behavior contract

- Crash/restart: slot checkpoint restored at boot (<400 ms); the next
  request continuing the saved conversation warms (cache_n ~= full).
  Requests that REWRITE history re-process honestly (logged
  "restored state diverged beyond rollback depth").
- Foreign tenant arriving: RELIEVE (sweep floor) within ~10 s, RESIZE
  (live window swap, ~1.6 GiB freed) after 60 s sustained, YIELD (208k
  profile + restart + oom demotion) after 240 s sustained. Walks back on
  sustained recovery. Ladder thresholds in `~/.vitriol/governor.toml`.

## Notes / follow-ups

- `/usr/local/bin/vitriol-governor` is the 12:29 build; the serde(default)
  fix (1c988d0) rides the next installer run (partial TOMLs only - the
  shipped full TOML parses on the old binary too).
- `/metrics`: `prompt_tokens_cached_total` stayed 0 despite cache_n>0
  turns - metrics plumbing for that counter needs a look (pre-existing).
- `VITRIOL_KV_SCORE: probe active` announce absent live: probe scoring is
  gated by `min_fill=0.5` -> dormant until the cache passes 131k tokens of
  262144. By design; the eager sweep floor is equally dormant until then.
- The install-vitriol-dropins.sh output still references unit restarts as
  manual steps; no action needed.
