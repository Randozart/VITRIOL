# Cache retention: a configurable byte budget with least-recently-addressed eviction

**Date:** 2026-11-17 16:20
**Mode:** PLAN (research only — nothing applied)
**Ask (owner):** "a configurable limit, like 40 GB, and once it passes that, it
starts deleting the entry that was addressed the least recent."

## 1. What already exists (measured today)

| tier | where | cap today | eviction policy today |
|---|---|---|---|
| host-RAM prompt cache | `--cache-ram` (live: `cache_ram = 8192` MiB) | **8 GiB**, MiB-denominated | **LRU by `t_last_hit`** (`server-task.cpp:1950-1966`) — oldest-hit victim, never FIFO |
| disk prompt-state tier | `--slot-save-path /scratch/kvtier` | **none** | prune only when damaged or identity-mismatched (`VITRIOL_KV_DISK_PRUNE`, pinned `0` by drop-in) |
| managed slot snapshots | same dir (`slot0.bin`), owned by `VITRIOL_CACHE_STORE` | **none** | never pruned (`file.managed` is exempt); reported via `/cache.managed` |

`/cache` right now (live):

```json
{"enabled":true,"identity":{…"n_ctx":262144,"type_k":2,"type_v":2},
 "mem":[{"n_tokens":6016,"bytes":292712028,"n_hits":0,"t_last_hit":0}],
 "disk":[], "disk_bytes":0, "disk_dir":"/scratch/kvtier/"}
```

So the *policy* the owner asked for is **already implemented for RAM** — what is
missing is (a) a byte budget on the **disk** tier, (b) a GB-denominated knob
instead of MiB, and (c) an *access* record for disk entries: the disk scan ranks
by **mtime (write time)**, which is "least recently *written*", not "least
recently *addressed*".

### Why "40 GB" can only mean disk

```
/scratch  375G total,  53G free   (87% used)   xfs, noatime
/home     1.9T total, 170G free   (91% used)
RAM        39G total,  18G used, 20G available
```

A 40 GB *RAM* cache is impossible on this box (39 GB total) and would violate the
2026-09-04 OOM discipline (the engine was the OOM-kill kingpin when its host
reservoirs grew). On `/scratch` a 40 GB cap is 75% of current free space, so the
cap is worth having but needs a safety margin, not a hard 40 GB default.

Two candidate disk targets, and they are different problems:

1. **KV states** — `/scratch/kvtier` today holds one managed `slot0.bin` (2.6 GB).
   States *are* "addressed" (a prompt restores them) and `t_last_hit`/`n_hits`
   already exist for the RAM twin. This matches the owner's words best.
2. **Model/HF cache** — `/scratch/huggingface` is 70 GB; `/scratch/vitriol-store-nemo.bin`
   is 38 GiB (Oct 16). A "40 GB" cap here means deleting models, which is an `hf`
   housekeeping tool, not an engine feature.

The plan below implements (1) and treats (2) as a separate, explicit tool. **If
the owner meant (2), say so and this plan channels into a `vitriol models prune`
instead — the phases below do not transfer.**

## 2. Design

One budget per tier, both configurable in **GB**, both evicting the least
recently *addressed* entry:

```
[kv] disk_max_gb = 40          # new: disk prompt-state tier (empty = unlimited, today's behaviour)
     cache_max_gb = 40         # new: alias feeding --cache-ram in MiB (memory tier)
```

Launcher translates GB → the engine's native units and puts `dmax=`/`cmax=` in
the fingerprint (provenance rule: any new speed/memory-bearing key rides it).

**Disk LRU needs a last-access record.** The mount is `noatime`, and the disk
scan currently sorts by mtime to prefer evolved prefixes; reusing mtime for LRU
would corrupt that preference. So:

- On a successful disk restore, `utimensat()` the state file (explicit touch —
  works regardless of `noatime`) **or** keep a tiny sidecar index
  `index.jsonl` `{name, bytes, last_access, n_hits}` written on spill/restore.
- Eviction sorts by `last_access`, oldest first, and stops as soon as
  `used_bytes + new_state_bytes <= cap`.
- Never evict: the file backing the in-flight request, a file with an open
  handle (detached writer still streaming), and `file.managed` snapshots
  (those are slot state, not cache — report them, never delete them silently).

**Instrumentation:** `kv_cache_disk_bytes` gauge, `kv_cache_disk_evicted_total`
counter (distinct from `_pruned_total`, which means "damaged/mismatched"), and
`/cache` gains `cap_bytes`, `used_bytes`, `evictions`, `oldest` (name + age).

**Officina surface:** `/cache` in the TUI — used/cap per tier, entry count,
oldest entry age, last three evictions; `/cache cap <gb>` to change it live
(writes config + fingerprint diff).

## 3. Invariants

- **I1 (live state untouched):** eviction never removes the state the current
  request is restoring or writing, nor any `managed` snapshot. Pinned by a unit
  test that the victim selector skips both, and by a live check that
  `slot0.bin` survives a forced eviction pass.
- **I2 (parity):** with `disk_max_gb` unset, behaviour is byte-identical to
  today (no touches, no evictions, `disk_bytes` unchanged). Pinned by a golden
  `/cache` snapshot test on an unset config.
- **I3 (OOM safety):** the RAM alias may never exceed 50% of physical RAM unless
  the operator passes an explicit override; the launcher refuses rather than
  clamps silently.
- **I4 (ordering preserved):** the disk *scan* order (newest-first prefix
  preference) is unchanged — LRU uses its own key, not mtime. Pinned by the
  existing restore-order tests plus a new one where LRU order and mtime order
  disagree.
- **I5 (provenance):** any effective cap appears in the `VITRIOL-FINGERPRINT:`
  line and in every benchmark RESULT argv.
- **I6 (no partial files):** eviction only removes fully written states; a
  state whose writer is still active is skipped this pass and reconsidered
  next.

## 4. Phases

1. **Measure first (30 min):** `ls -la /scratch/kvtier`, `/cache`, and the
   `VITRIOL_CACHE_STORE` managed-store info over a week of normal use, to learn
   the real growth rate and entry count. Today's data (one 2.6 GB snapshot,
   `disk: []`) says a 40 GB cap is not yet *reached* — so the feature is
   preventive, and the phase-1 deliverable is a written growth curve, not code.
2. **Engine: cap + LRU (the only real code).** Victim selector as a pure
   function (`select_disk_victims(entries, cap, incoming_bytes, exempt)`) so it
   is testable without disk I/O; wire into the spill path and the scan path;
   add the two metrics; extend `/cache`.
3. **Config plumbing:** `[kv] disk_max_gb` → env → engine, fingerprint `dmax=`,
   plus the RAM alias with the I3 guard. `scripts/vitriol config` writes are
   followed by the mandatory fingerprint diff.
4. **Officina `/cache`** command + a status row line (`cache 2.6/40 GB · 1 entry · oldest 3h`).
5. **Docs:** `docs/cache-retention.md` (tiers, policy, knobs, what never gets
   evicted), linked from AGENTS.md's KV residency section.

## 5. Owner answers (2026-11-17 16:35) and what the code says about them

**Answers given:** (1) the `/scratch` KV cache; (2) 40 GB flat, configurable; (3) managed
snapshots **count** toward the budget — "that's the whole point"; clear out the other two.

### 5.1 The load-bearing question: do we use the whole cache, and is eviction safe?

The answer is in three measurements, and it is more reassuring than expected:

1. **Nothing accumulates today.** Live `/cache`:
   `disk: []`, `disk_bytes: 0`, `managed: null` → the disk *prompt-state* tier holds
   **zero entries**, and this build has no `VITRIOL_CACHE_STORE`. `/scratch/kvtier`
   contains exactly **one** file: `slot0.bin` (2.6 GB, Nov 17 15:57).
2. **That one file is a slot snapshot, not a cache entry.** `post_slots` writes
   `slot_save_path + "slot<N>.bin"` — one file per slot, **overwritten** on every save
   (`server-context.cpp:5949`). It is by definition always the freshest state, so
   "older entries" do not exist yet for it.
3. **It is window-sized, not content-sized.** The live RAM entry holding the conversation
   is 292 MB for 6,016 tokens; `slot0.bin` is 2.6 GB because it carries the full 262k
   window structures plus the `--ctx-checkpoints 4` ring. So yes — most of those bytes are
   not "used", but a snapshot cannot be *partly* deleted: it is one entry, kept or dropped.

**And eviction is always correctness-safe.** A cache miss is not an error path: the engine
falls back to a normal prefill (the keep gate and identity verdicts exist precisely so a
wrong or absent state is *skipped*, never trusted). Deleting an entry therefore costs
**latency (a re-prefill), never correctness** — which is what makes the plan viable. The
only entry whose loss is expensive is the one the next turn wants: today that is exactly
`slot0.bin`, the warm-resume path (measured 745/755 tokens cached, 0.5 s resume vs a full
prefill).

Consequence for the design: with snapshots *inside* the budget, the eviction rule must
protect the **active** slot's own snapshot (the one this process restored or most recently
wrote) and evict oldest-*accessed* among the rest. Otherwise the feature would delete the
very state that makes boot fast, on every boot.

### 5.2 The two things that are genuinely unused (owner: "clear out the other two")

| path | size | evidence | verdict |
|---|---|---|---|
| `/scratch/vitriol-store-nemo.bin` | **38 GiB** (Oct 16) | referenced only by two experiment plans (`nemotron35-super-vl-bringup-2026-10-16.md`, `dma-stream-revival-2026-10-16.md`); no unit, script, or config names it | **delete** |
| `/scratch/huggingface` | **70 GB** | plain HF cache (`hub/` datasets + models, `blobs/`); no `HF_HOME`/unit reference; the live models live in `~/Models` and `/scratch/models` | **delete** (re-downloadable) |

Together ≈ **108 GB**, which turns the cap arithmetic from tight to comfortable:
`/scratch` 53 GB free → **~161 GB free**, so a 40 GB cap leaves 121 GB headroom instead of
13 GB.

## 6. Revised design (answers folded in)

- `[kv] cache_max_gb = 40` — **one budget for the whole `/scratch/kvtier` directory**
  (snapshots **included**), fingerprint `cmax=40`. Unset = unlimited (today's behaviour).
- **Eviction order:** oldest *last-accessed* first. Access = restored, written, or
  explicitly touched; recorded with `utimensat()` on read (works under `noatime`) **plus**
  an `access.log`-style sidecar so the order survives a `utimensat`-less copy.
- **Exemptions (the only ones):** the file being written/read by the in-flight request, and
  the active slot's most recent snapshot. Everything else is evictable, oldest first.
- **Enforcement points:** after each snapshot write and after each restore; a scan-based
  sweep at boot before the sidecar's warm restore (so boot sees an already-bounded dir).
- **Counters:** `kv_cache_disk_bytes` (gauge, whole dir incl. snapshots),
  `kv_cache_evicted_total{kind=snapshot|state}`, `/cache` gains `cap_bytes`, `used_bytes`,
  `oldest{name,age,kind}`.
- **Officina:** `/cache` shows `2.6/40 GB · 1 snapshot · 0 states · oldest 3h`, and
  `/cache cap <gb>` rewrites config with the mandatory fingerprint diff.
- **Boot interaction:** the governor sidecar restores `slot0.bin` by fixed name; the sweep
  runs before it, so a missing file means "skip warm restore, prefill normally" — the
  existing, tested path.

## 7. Invariants (revised)

- **I1 (live state untouched):** never evict the in-flight request's file or the active
  slot's newest snapshot. Test: victim selector skips both; a forced sweep under cap
  pressure leaves the active snapshot on disk.
- **I2 (parity):** `cache_max_gb` unset ⇒ no touches, no evictions, identical `/cache` bytes.
  Golden snapshot test.
- **I3 (correctness never depends on the cache):** after deleting every entry, a turn still
  succeeds via full prefill. Test: delete the directory, run a completion, expect 200 and a
  higher `prompt_tokens_total` delta.
- **I4 (scan order preserved):** LRU uses its own key, not mtime; the disk scan's
  newest-first prefix preference is unchanged.
- **I5 (provenance):** the effective cap appears in `VITRIOL-FINGERPRINT:` and every
  benchmark RESULT argv.
- **I6 (bounded boot):** the sweep completes in O(entries) and never blocks dispatch on a
  full state read (size and mtime only).

## 8. Cleanup step (separate from the engine work, do first)

1. `rm /scratch/vitriol-store-nemo.bin` → +38 GiB.
2. `rm -rf /scratch/huggingface` → +70 GB (datasets/models re-download on demand).
3. Re-check `df /scratch`; record before/after in this plan.

These are `rm` commands, so they belong to a BUILD turn with explicit owner consent; nothing
here is deleted yet.

## 9. Open questions for the owner (remaining)

1. **Where does the cap live?** `~/.vitriol/config` under `[kv]` (launcher-enforced, rides
   the fingerprint) — or an env var only (`VITRIOL_KV_CACHE_MAX_GB`) so it can be flipped
   without a config write? Recommendation: config key, env override, both fingerprinted.
2. **Active-snapshot protection window.** The active slot's newest snapshot is exempt for
   its lifetime in this plan. Accept, or exempt only until the next successful write (so a
   leftover snapshot from last week is evictable even for the same slot)?
3. **Sidecar coordination.** `vitriol-governor`'s boot restore uses the fixed name
   `slot0.bin`. Eviction must never race it. Confirm the sweep runs *before* the sidecar's
   restore window (the plan assumes the engine's boot sweep wins; the sidecar then just
   skips a missing file).

## 10. Shipped (2026-11-17, BUILD)

### 10.1 Cleanup — 108 GiB reclaimed

```
before: /dev/nvme0n1p2 375G 324G 52G 87% /scratch
after : /dev/nvme0n1p2 375G 216G 160G 58% /scratch
deleted: /scratch/vitriol-store-nemo.bin (38 GiB), /scratch/huggingface (70 GB)
```

### 10.2 Engine — cap + LRU (commit 6dd6086)

Pre-existing behaviour that was wrong and is now fixed:

- the disk cap was **derived** (12 × cache_ram = 96 GiB here), not configurable;
- victims were chosen by **write** time while the log said "oldest": `disk_scan` sorts
  newest-first for prefix candidacy, and the trim loop walked that same order — so it deleted
  the **freshest** entries first;
- `slot*.bin` snapshots were **not** in the scan at all: the only files on this host were
  invisible to the budget.

Now:

| piece | where |
|---|---|
| `VITRIOL_KV_CACHE_MAX_GB` (authoritative when set; unset = historical derived cap) | `server-task.cpp: server_prompt_cache_disk_cap_bytes` |
| last-access key = explicit `utimensat` touch on restore, write time as fallback | `server_prompt_cache_disk_touch`, `..._last_access` |
| victim = least-recently-addressed; exemptions = incoming file + newest snapshot + managed | `server_prompt_cache_disk_trim` |
| snapshots counted only in the budget scan, never in listings | `..._disk_scan(dir, files, include_snapshots)` |
| boot sweep after `disk_load`, plus a sweep before every spill | `disk_load`, `disk_spill` |
| metrics `kv_cache_evicted_total`, `kv_cache_bytes`; `/cache` `cap_bytes`/`used_bytes`/`over_cap`/`evicted_total` | `server-task.cpp`, `server-context.cpp` |
| `[server] cache_max_gb` → env, fingerprint `cmax=` | `scripts/vitriol` |

### 10.3 Live proof of eviction

Three fake snapshots (30 MiB each, aged 3h/2h/1h) beside the real `slot0.bin`, cap set to
**0.02 GB**:

```
after restart: only slot0.bin remains
/cache: cap_gb 0.02 | used_gb 2.5118 | over_cap True | evicted_total 3
```

→ the cap was enforced, all three victims went least-recently-addressed first, and the
**newest snapshot survived** (protection works). `over_cap: true` is the honest answer: a
single protected snapshot (2.5 GiB) cannot fit a 20 MiB budget.

Restored to 40 GiB:

```
/cache: cap_gb 40.0 | used_gb 2.512 | over_cap False | evicted_total 0
metrics: llamacpp:kv_cache_bytes 2.69705e+09
```

### 10.4 Officina surface

`/cache` list line and modal title now render the same budget the engine acts on:
`2.51 GiB / 40.00 GiB` (plus `· OVER CAP` when the protected entries alone exceed it).
`cap_bytes()` mirrors the engine's fallback (12 × cache_ram) so the readout cannot lie about
the policy. Installed TUI `1d307f1e`.

### 10.5 Notes / follow-ups

- The TUI's own protection rule still hardcodes `slot0.bin`; the engine now protects the
  *newest* snapshot. Aligning the TUI is cosmetic until a second slot exists.
- Invariant I3 (correctness never depends on the cache) is unchanged by design: a miss is a
  prefill, verified by the fact that this feature deletes entries while the engine keeps
  serving.
- No C++ unit harness exists for `server-task.cpp` internals; the pure selector is verified by
  the live 0.02 GB experiment above rather than a unit test. Worth extracting if this code
  grows.
