# Optane 900P utilization + bcache conversion — 2026-09-29

Status: EXECUTION. Baselines measured, cleanup partly done; remainder runs from this plan.
Plan author: opencode (VITRIOL agent). Bcache conversion: owner-executed (handoff folded in §4).

## 1. Hardware + wear (S1, 2026-09-29 09:13)

- Optane 900P 480 G `nvme0n1`: `Percentage Used 0%`, 9.85 TB written, 812 power-on h,
  47 C, `Critical Warning 0x00`. Rated ~17 PBW → effectively unlimited.
- Samsung 870 QVO 2 TB `sda6`: Total_LBAs_Written 48.9 TB / 533 TB TBW = 9.2% used,
  Reallocated=0, Wear_Leveling 96, 26 C, ~147 GiB/day host writes.
- Kernel 7.2.5-cachyos (+lts 6.18.50), fish, limine, systemd mkinitcpio, snapper.

## 2. fio baseline — Optane vs QVO (2026-09-29 08:5x, io_uring, O_DIRECT, 4k/1m)

| test | Optane 900P | 870 QVO (SATA) | ratio |
|---|---|---|---|
| 4K randread QD1 | 45.8K IOPS, 179 MiB/s, ~22 us | 7.7K IOPS, 30 MiB/s, ~130 us | **5.9x IOPS, 6x lat** |
| 4K randread QD32 | 172.8K IOPS, 675 MiB/s | 77.5K IOPS, 301 MiB/s | 2.2x (QVO DRAM-cache inflated) |
| 1M seq QD1 (model-load shape) | 1001 MiB/s | 428 MiB/s | 2.3x |
| 1M seq QD8 | 1423 MiB/s | - | - |

Model-load baseline (SATA, engine run 2026-09-28): `loading model` 0.00.484 ->
`model loaded` 1.14.240 = **74.2 s** (incl. kvtier state restore; A/B after move must
compare phases, not totals — kvtier wiped in S2).

## 3. Cleanup done (S1/S2, 2026-09-29 morning)

- `~/Downloads/staging-qwen38fn` (84 G duplicate of /scratch/qwen38fn): 3/3 sha256
  byte-identical -> deleted, +83 G /home. fio temp files removed (+5 G).
- kvtier audit pre-wipe: 53 files / 69.3 GiB, all `v1-unverified`, under old 96 G cap.
- Decision: **allocsize mount option SKIPPED** — XFS default dynamic EOF prealloc already
  adapts; fixed value disables the heuristic, no gain.
- `flashnext.store`: removed during owner's /scratch shuffle (recorded; 8 K placeholder,
  Tier-3 store remains deferred — recreate with `truncate` if wanted).

## 4. Owner bcache conversion (handoff, 2026-09-29 — executed by randozart)

Goal: write-back cache on Optane for QVO (root) → wear down + perf up.

Final Optane layout (backup `~/nvme-sfdisk-backup.txt`):

| part | size | content |
|---|---|---|
| p1 | 64 G | swap pri=10 (untouched) |
| p2 | 120 G | XFS /scratch (recreated; kvtier restored partial 28/60) |
| p3 | 255 G | bcache cache device, write-back, set `ea19d211-0b6f-4a75-968b-7c97a3137dcb` |
| p4 | 8.1 G | reserved future boot (untouched) |

Key steps: GLM 110 G + qwen38fn 84 G deleted from /scratch; /scratch backup -> session
restore -> backup deleted; stale AG superblock at p3 -> `wipefs -a`; backing-SB EBUSY/blkid
workaround = generate SB on loop image, `dd` bytes 4096-8191 onto sda6, register
(SB only, btrfs super at 65536 untouched); mkinitcpio MODULES=(bcache) + bcache hook both
kernels; fstab root -> `/dev/bcache0`, /scratch -> new UUID `63c31f19-...`.
Pre-reboot: bcache registered, clean, dirty_data=0, root I/O still raw sda6 (dormant).

Rollback: limine fallback entry; fstab.bak; revert MODULES/HOOKS + mkinitcpio -P;
undo SB with zero-dd (command in handoff); sfdisk backup; snapper snapshots.

Pitfalls (owner): fish no `<<<`; sfdisk `--no-reread` scoped `-N`; XFS can't shrink;
rsync restore needs sudo + `-S` for sparse; stacked double-mount check findmnt;
stale fs superblocks in freed ranges; bcache-tools from AUR ships udev rule + hook;
cancelled rsync is alphabetical — diff backup vs live before deleting anything.

## 5. This plan (S3+ — current execution)

**Stage A — cleanup + move (engine stopped 10:14, safe window)**
1. Wipe `/scratch/kvtier/*` (dir kept, config path unchanged).
   Rationale: v1-unverified states from pre-provenance build; model path changes
   (symlink) → state identity moot; one cold prefill after next start is accepted.
2. Delete unused GGUFs (43 G /home): `Qwen3.8-27B-UD-IQ3_S` 12 G (breaks
   `qwen38-master`/`qwen38-ontic`), `Qwen3.8-27B-Q3_K_M` 13 G (breaks `mtp-131k*`,
   `distributed*`), `Qwen3.8-9B-Q8_0` 10 G, `Qwen3.8-9B-Q6_K` 8 G (breaks `9b-*`).
   All re-downloadable (host HF CDN needs ranged-resume downloader).
3. Keepers → `/scratch/models` (29 G): BuffedMod Q6K (live), IQ3_S-mtp (blessed
   rollback), Ternary-Bonsai-2 (owner: used elsewhere). `~/Models` → symlink to
   `/scratch/models` → all config/profile/user paths resolve unchanged.
4. Verify: `test -f` live-config path + Bonsai path; lsof empty; df.

**Stage B — report + commit** (this file) + Stage A results.

**Stage C — owner, before reboot**
5. `vitriol.verbose = true` (config key; NOT in fingerprint body — diff-verify anyway)
   for post-reboot f_keep/Phase-1 re-prefill capture.
6. systemd drop-in `Environment=VITRIOL_KV_DISK_PRUNE=1` + `daemon-reload` (owner sudo).
7. B8 reboot → verify: `findmnt /` = `/dev/bcache0`, cache_mode [writeback],
   `findmnt /scratch` = p2; engine auto-starts (enabled) → **Stage D checks**:
   fingerprint unchanged, provenance counters live (`kv_cache_disk_*`), PRUNE active,
   model-load A/B vs 74.2 s (phase-compare), cold session prefill, verbose f_keep.
8. Fase C: pacman `CacheDir = /scratch/pacman` (14 G off QVO), journal → /scratch/journal.
9. Fase D: swappiness 150→100 (`/etc/sysctl.d/60-swappiness.conf`); reduce resident set
   (opencode×5 first — do NOT drop `--cache-ram 12288` below warm-resume need; 0 forbidden).
10. Fase E: 48 h wear compare `~/diskstats.t0` (sda6 sector-10 col ×512); weekly SMART.

**Capacity after Stage A** (/scratch 120 G): ~117 G free − 29 G models − 14 G pacman
− 5 G journal ≈ **69 G** for kvtier regrowth (previously 96 G cap — RECALIBRATE: cap
must be ≤ ~70 G on the 120 G p2, or kvtier alone overflows the fs).

**Deferred (not in this plan)**: Tier-3 flash-next store/streaming (residency rule:
streaming only when weights exceed combined VRAM); re-prefill 2-phase investigation
(plan: `reprefill-cancel-modeswitch-2026-09-29.md`); `--cache-reuse` launcher wiring.

## 6. Results (amend after each stage)

- **Stage A (2026-09-29 11:26) — DONE**:
  - kvtier wiped: 42 G reclaimed, 0 files left, dir kept. /scratch 117 G free.
  - Deleted 4 GGUFs (43 G /home): UD-IQ3_S, Q3_K_M, 9B-Q8_0, 9B-Q6_K.
  - Moved 3 keepers (29 G) -> /scratch/models, sha256 before/after **byte-identical**
    (`dc3cd01b...` BuffedMod, `58fd8267...` IQ3_S-mtp, `53107f53...` Bonsai).
  - `~/Models` -> symlink `/scratch/models`; all 3 paths `test -f` OK; lsof empty.
  - df after: /scratch 88 G free (33 G used), /home 488 G free (+63 G).
- **Stage C-pre (2026-09-29 11:2x) — DONE**: `vitriol.verbose = true` written;
  `serve --dry-run` projected FP **identical** to `fingerprint.last` (verbose not in
  fingerprint body) → no flag drift. BLESSED delta on screen is the known stale
  2026-09-17 blessed point, pre-existing, untouched.
- **Stage B**: this file, committed with Stage A.
- **Stage C/D**: (pending — drop-in + reboot + post-reboot checks)
- **Stage D load A/B**: (pending)
