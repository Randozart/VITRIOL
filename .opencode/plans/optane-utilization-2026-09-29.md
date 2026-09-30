# Optane 900P utilization + bcache conversion — 2026-09-29

Status: EXECUTED — Stages A/C done, bcache rolled back + p3 repurposed /scratch2,
engine relaunched + verified (§6). Deferred: B9 reboot validation, Fase E (10-01),
pacman CacheDir, rag unit-file removal.
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

## 6. Results (2026-09-29/30 final)

- **Stage A (09-29 11:26)**: kvtier wiped (42 G), 4 unused GGUFs deleted (43 G),
  3 keepers moved sha256-verified (`dc3cd01b…` BuffedMod, `58fd8267…` IQ3_S-mtp,
  `53107f53…` Bonsai), `~/Models` → symlink `/scratch/models`, all paths resolve.
  df: /scratch 88 G free, /home 488 G free. Committed `011a2a5`.
- **Stage C-pre**: `vitriol.verbose = true`; dry-run FP identical to `fingerprint.last`
  (verbose not in fingerprint body → no drift).
- **Bcache incident (09-29, owner)**: first reboot after conversion failed inside
  initramfs (boots absent from persistent journal = root never mounted); fixed by
  fstab root → raw `/dev/sda6`. **Final disposition 09-30: full rollback + repurpose.**
  - `limine.conf` root= path: durable fix lives in `/etc/default/limine`
    (`root=UUID=863f49fd…` = sda6 live btrfs UUID, verified via `/sys/fs/btrfs/`);
    `/boot/limine.conf` is rewritten every ~5 h by `limine-snapper-sync` — direct
    edits there are ephemeral.
  - Block A: set `ea19d211…` unregistered (NOTE: `/sys/fs/bcache/unregister` does not
    exist on kernel 7.2.5 — write goes to `/sys/fs/bcache/<set>/unregister`; a plain
    `>` to the missing path fails EACCES via O_CREAT), module removed, `MODULES=()`,
    bcache HOOK dropped, both initramfs rebuilt clean by `limine-mkinitcpio`,
    3 `.bak` files (mkinitcpio.conf, default/limine, limine.conf).
  - p3 (255 G): bcache SB erased (`16 bytes … offset 0x1018 (bcache)`), mkfs.xfs
    `-L optane2` → mounted `/scratch2` (fstab `UUID=c91f6fe7…`, **nofail**), 250 G avail.
  - Fase C: journal → `/scratch2/journal` (52.4 M via symlink, journald restart OK);
    parked/re-homed around the mount (`/home/randozart/journal-xfer`, cleaned).
  - Fase D: `vm.swappiness = 100` (`/etc/sysctl.d/60-swappiness.conf`, was runtime-only
    150). `mongod-vitriol` user unit disabled (203/EXEC loop, pkg gone);
    `vitriol-rag` stopped by owner decision (Pymander stack superseded).
- **Model-load A/B (phase compare `loading model` → `model loaded`)**:
  **74.2 s (SATA baseline) → 13.4 s (first boot after move) → 12.7 s (09-30 09:24
  relaunch) = 5.5×**. Causes: Optane QD1 seq 1001 vs 428 MiB/s + 4K random 6× +
  empty kvtier (no state restore). n=2 post-move, direction unambiguous.
- **Stage D verification (09-30 09:24, post-Phase-1 relaunch)**:
  - FINGERPRINT identical (`ts=27,21 c=262144 …spec=mtp:3 par=1 ssp=1 cram=12288`)
  - health ok, metrics: all **5 provenance counters live** (`kv_cache_disk_restored/
    fetched/miss/rejected/pruned_total` — HELP text of pruned cites `VITRIOL_KV_DISK_PRUNE=1`)
    + `llamacpp:kv_ejected_total`
  - `VITRIOL_KV_DISK_PRUNE=1` present in llama-server `/proc/PID/environ` (drop-in →
    launcher → child propagation confirmed); full argv embedded in §5 flow
    (`--slot-save-path /scratch/kvtier … -ts 27,21 --spec-draft-n-max 3`)
  - verbose semantics: `vitriol.verbose=true` exports `VITRIOL_VERBOSE=1` + banner;
    server stays `-lv 3` — sufficient (f_keep/f_sim lines appear at lv 3, proven 09-28)
  - kvtier: `slot0.tmp.bin` only (spill machinery running, PRUNE armed)
- **Deferred**: B9 reboot validates bcache-free initramfs; Fase E 48 h wear compare
  2026-10-01 (`~/diskstats.t0` — now also measure p2/p3 vs sda6); pacman
  `CacheDir → /scratch2/pacman` (14 G off QVO, pending owner go); `vitriol-rag`/
  `mongod-vitriol` unit FILES still on disk (stopped/disabled — removal command issued,
  owner to run); re-prefill Phase-1 capture on next natural occurrence.

## 7. Postscript 2026-09-30 — /scratch2 merged back into /scratch (single 375 G XFS)

Supersedes the two-partition layout in §4/§6: with bcache abandoned, p3's sole
purpose vanished. Merged online; `/scratch2` no longer exists.

- Target table == pre-conversion `~/nvme-sfdisk-backup.txt` (p1, p2=786432000, p4;
  no p3). Written with `sfdisk --no-reread` (swap+p2 busy; kernel reread cannot run).
- **Incident (live-resize ordering):** `partx -u` implements a changed partition as
  *delete + re-add*. It **deleted kernel p2** and could not re-add it (375 G now
  overlaps the still-mapped kernel p3) → `error updating partition 2`; p2 device and
  its XFS by-uuid vanished. No fs data written.
- **Recovery:** `partx -d --nr 3 /dev/nvme0n1` (drop stale p3 mapping; p3 unmounted)
  → `partx -a /dev/nvme0n1` (add-only; p2 @786432000 reappears, by-uuid returns) →
  `mount /scratch` → `xfs_growfs /scratch` (`data blocks 31457280 → 98304000`).
  `partx -a` prints "error adding partition 1/4" (already exist) — benign.
- Journal re-homed `/scratch2/journal` → parked (`/scratch/journal-park`) around the
  swap → `/scratch/journal`; `ln -sfn`; journald restarted. Stale `/scratch2` mountpoint
  removed.
- Final: `/scratch` 375 G XFS on nvme0n1p2 (model tier + kvtier + journal + pacman),
  p1 swap 64 G, p4 reserved 8.1 G. fstab single `/scratch` line; no `nofail`.
- Rollback artifacts: `~/nvme-sfdisk-current-2026-09-30.txt` (pre-merge),
  `~/nvme-sfdisk-backup.txt`, `/etc/fstab.bak-merge`.
- **Lesson:** on a disk with any busy partition, a live grow must target an *unmounted*
  target and sequence delete-stale-mapping → add-only; `partx -u` is unsafe for
  resize-in-place.

## 8. Hardening batch (2026-09-30, "no-brainers")

Details + evidence: `hardening-2026-09-30.md`. Applied: smartd (both disks monitored),
journald `SystemMaxUse=1G` drop-in, pacman `CacheDir=/scratch/pacman` + paccache.timer
(−7.31 GiB), `xfs_scrub_all.timer`, `earlyoom`, OOM sysctls
(`watermark_scale_factor=100`, `min_free_kbytes=262144`), Pymander unit files removed.
