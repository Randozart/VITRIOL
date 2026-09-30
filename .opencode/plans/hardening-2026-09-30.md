# PC hardening + Optane merge — 2026-09-30

Scope: resilience/monitoring hardening after the bcache rollback and the
`/scratch2 → /scratch` merge (merge details: `optane-utilization-2026-09-29.md` §7).
Engine not restarted (owner-deferred); nothing here required it.

## Baseline (pre-change, audited 2026-09-30 ~20:05)

Already good:
- `ufw` active + enabled; no failed systemd units; **0 OOM kills in 7 days**
- root btrfs: `compress=zstd:3,ssd,discard=async,space_cache=v2`; `fstrim.timer` +
  `btrfs-scrub@-.timer` active; snapper root config present (pre/post, NUMBER_LIMIT 50)
- vitriol-server: `Restart=always`, `OOMScoreAdjust=-500`, PRUNE env, SYSTEM scope
- zram 31.3 G zstd; swappiness 100 (set earlier)

Gaps found:
- `smartd` installed but **disabled** → zero automated SMART alerting
- journald uncapped in our config (main conf had 50 M; no drop-in)
- 14 G pacman cache on QVO, never trimmed (`paccache.timer` disabled)
- new XFS `/scratch` had no periodic scrub
- no userspace early-OOM daemon
- OOM sysctls at defaults
- dead Pymander user units on disk

## Applied (verified)

| item | command summary | verification |
|---|---|---|
| **A1 smartd** | `/etc/smartd.conf` → `DEVICESCAN -a -o on -S on -n standby,q -s (S/../.././02\|L/../../7/03) -W 4,55,65`; `enable --now` | active; "Monitoring 1 ATA/SATA … 1 NVMe"; temps 29 °C (sda) / 49 °C (nvme0). NVMe self-tests unsupported → schedule ignored for it (harmless) |
| **A2 journald cap** | `/etc/systemd/journald.conf.d/99-vitriol.conf`: `SystemMaxUse=1G`, `SystemMaxFileSize=128M`; journald restarted | `cat-config` shows drop-in effective; usage 52.4 M |
| **A3 pacman cache** | `CacheDir = /scratch/pacman` (+`.bak`); moved 14 G; `paccache -rk1`; `paccache.timer` weekly | cache 6.6 G on `/scratch`; `/var/cache/pacman/pkg` empty; timer armed (Mon). Removed 702 pkgs / 7.31 GiB |
| **B1 XFS scrub** | `enable --now xfs_scrub_all.timer` | armed Sun 2026-10-04 03:10 |
| **C1 earlyoom** | `pacman -S earlyoom`; `enable --now` | **pending start** (initial `enable --now` did not execute); default `EARLYOOM_ARGS="-r 3600 -n --avoid '(^\|/)(init\|systemd\|Xorg\|sshd)$'"` |
| **C2 OOM sysctls** | `/etc/sysctl.d/70-oom.conf`: `vm.watermark_scale_factor=100`, `vm.min_free_kbytes=262144` | applied (`=100`, `=262144`) |
| **E2 Pymander units** | removed `~/.config/systemd/user/vitriol-rag.service`, `mongod-vitriol.service`; `daemon-reload` | files gone; both inactive |

Timer state after: `xfs_scrub_all` Sun 10-04 · `paccache` Mon 10-05 · `fstrim` Mon 10-05 ·
`btrfs-scrub@-` Wed 10-07.

## Still open

- **D boot recovery**: no fallback initramfs (`MKINITCPIO_FALLBACK` unset); limine snapshot
  boot entries blocked by `/boot` usage (`/boot` 253 M/511 M, one snapshot entry ≈ +238 MiB
  vs `limine-snapper-sync` 85 % limit). `/scratch` fstab line has no `nofail`. → discuss.
- **G backup**: `/home` (1.4 T) has no snapshot/backup; snapper covers only `/`. Need
  selective backup target + scope. Models re-downloadable; configs/docs are not.
- **VITRIOL**: engine restart deferred (owner) → applies merge pickup, verifies provenance
  counters/load; unit `StartLimit*`/`TimeoutStopSec` hardening also waits for that restart.
- **B9 reboot**: validates bcache-free initramfs + merged GPT in one shot.
- **Fase E**: 48 h wear compare vs `~/diskstats.t0` (2026-10-01); now measure sda6 vs p2.
- **GPU**: 790 `NVRM: Xid` events / 7 d, all from `gemm_h_harness` / `ptx_gemm_bench`
  (Xid 13/43 SM exceptions). Engine logs show none — investigate/quarantine those benches.
