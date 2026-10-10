# Cache cleanup -> full prefill from 0: root cause + fix plan

- **Date:** 2026-10-10 11:47
- **Status:** Stage A in progress; B/C pending
- **Symptom:** occasionally the engine "cleans up the cache", afterwards the next
  turn prefills the whole conversation from token 0.
- **Component:** `llama.cpp/tools/server/server-task.cpp` (disk trim / fetch),
  `server-context.cpp` (do_reset), `scripts/vitriol` (cache-reuse wiring).

## Architecture recap (KV residency tiers)

1. slot KV in VRAM (`server_slot::prompt`)
2. L0 host RAM prompt cache, `--cache-ram 12288` MiB (`server-task.cpp` alloc/update)
3. L1 Optane disk mirror `cache-<modelkey>-<tokhash>.bin` under
   `--slot-save-path=/scratch/kvtier` (spill on save, boot `disk_load`, runtime
   `disk_fetch`)

Normal warm turn never touches tiers 2/3: slot holds prompt+KV, LCP computes
delta prefill. Tiers 2/3 only matter after restart or slot wipe - which is
exactly when they fail.

## Root cause (verified live 2026-10-10)

### R1 (primary): silent disk-trim deletes the oldest mirror

- `server-task.cpp:2291` caps the mirror at `limit_size * 8` = **96 GiB**
  (cram=12288). Live usage measured: **32 files, 95.7 GiB** - headroom ~0.3 GiB,
  less than one spill (observed spill sizes 1.1-6.7 GiB).
- `server_prompt_cache_disk_trim` (`server-task.cpp:2265-2281`) then deletes
  oldest-**mtime** files with `remove()` - **zero log lines**. This is the
  literal "occasionally cleans up the cache" event.
- Why the victim is often the ACTIVE conversation: `disk_spill` runs only on
  `prompt_save`, which fires only on divergence (f_keep<0.5) or LRU. A growing
  conversation that keeps matching is never re-spilled -> its mtime stays at
  first save -> oldest -> first to be deleted. Survival across restart then
  depends on a file that no longer exists => prefill from 0.

### R2: mirror mtime is not recency of use

Trim order = oldest write time. A rarely-used session re-spilled recently
outranks the session you resume daily but never re-spill. Fix: touch mtime on
successful fetch (LRU-by-fetch).

### R3: `do_reset` forced full reprocess is TRC-only (invisible)

`server-context.cpp:3499`: `forcing full prompt re-processing due to lack of
cache data` logs at SLT_TRC, not WRN. Fires when `pos_min >= pos_next - n_swa`
and no `common_prompt_checkpoint` covers it. Aggravators:
- checkpoints are NEVER persisted to disk (`disk_write` stores tokens+KV only)
  -> after any restore, checkpoint ring is empty.
- ring depth only `--ctx-checkpoints 4`, erased on overflow (WRN exists).
- sparse-KV ejection raises `pos_min` (`kv_ejected_total`; metrics endpoint
  currently blind - see C3).

### R4: head divergence has no salvage

`server-context.cpp:3338`: n_past = LCP; mode-switch/cancel diverges at token 0
=> prefill from 0. `--cache-reuse N` salvage path exists (`server-context.cpp:
3346-3404`) but `scripts/vitriol` never emits it (rebis-servers.sh does, 256).
Documented: `.opencode/plans/reprefill-cancel-modeswitch-2026-09-29.md`.

### R5: fetch rejection is silent

`disk_find_best` (`server-task.cpp:2389`) rejects candidates with
`f_keep < 0.25` at TRC level only. Near-miss (lcp>0, gate failed) invisible.

## Secondary / amplifiers (found during audit, not fixed here)

- Engine runs bare (no systemd since Oct 3), `oom_score_adj=0`, autosave sidecar
  dead + path mismatch (`slot_save_path=/scratch/kvtier` vs sidecar default
  `~/.vitriol/checkpoints`).
- `/metrics` returns all zeros in the Sep-28 binary -> `kv_ejected_total`,
  `kv_cache_disk_*` counters blind; working tree has +426 uncommitted lines
  (provenance-observability stage of 2026-09-28, implemented, not yet committed).
- Identity mismatch (model file mtime/size touch) rejects all mirrors
  (`VITRIOL_KV_DISK_ALLOW_MODEL_MISMATCH` escape); with
  `VITRIOL_KV_DISK_PRUNE=1` they get deleted at boot. Prune currently OFF on the
  bare process, ON under the systemd drop-in.
- `cache_idle_slots` flag drift: config `off` never emits
  `--no-cache-idle-slots`; fingerprint prints `cis=0` while engine runs ON
  (upstream default true). Nil at parallel=1.

## Stage A (this session) - code + launcher

1. **R1: loud + bigger trim cap** (`server-task.cpp`)
   - multiplier env `VITRIOL_KV_DISK_TRIM_X`, default **12** (144 GiB at
     cram=12288; /scratch has 175 GiB free), range 1..1024.
   - every deletion logs WRN: file name, size, before/after totals, limit.
   - summary WRN when trim pass runs.
2. **R2: touch-on-fetch** - after successful `disk_read_best`, refresh file
   mtime (utimensat, non-fatal) so trim prefers least-recently-fetched.
3. **R5: near-miss WRN** - `disk_find_best` tracks best rejected candidate
   (f_keep<0.25 with lcp>0) + files scanned; on miss logs WRN with those stats
   (plain miss stays TRC with file count).
4. **R3: do_reset SLT_TRC -> SLT_WRN** (`server-context.cpp:3499`) so every
   forced from-0 reprocess is greppable.
5. **R4: wire `--cache-reuse`** - config key `server.cache_reuse` (profile),
   launcher emits `--cache-reuse N` when set, fingerprint `creuse=N`.
   Default unset = current behavior (no silent flag drift).

Verification: build server target, `llama-server --help` smoke, launcher
`--help`/fingerprint dry parse. Live engine restart NOT part of stage A
(restart lands B/C on the live box).

## Stage B (follow-up, needs owner sign-off)

- Persist the lowest-pos `common_prompt_checkpoint` in `disk_write` /
  SLOT_SAVE so `do_reset` has a landing spot after restore (cost: file size).
- Consider raising `f_keep` fetch gate or logging its rejects in `/cache`.

## Stage C (ops, disruptive - do not run without asking)

- Rebuild + redeploy engine; return to systemd unit (`OOMScoreAdjust=-500`,
  Restart=always, keep `VITRIOL_KV_DISK_PRUNE` decision explicit).
- Fix autosave sidecar dir (`VITRIOL_SLOT_SAVE_DIR=/scratch/kvtier`), remove
  orphan `/scratch/kvtier/slot0.tmp.bin`.
- Fix `cache_idle_slots` emission + `cis=` fingerprint.
- Migrate the 95.7 GiB mirror carefully (trim will fire on first spill after
  deploy while old files remain).
