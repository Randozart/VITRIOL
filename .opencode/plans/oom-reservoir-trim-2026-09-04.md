# OOM kill investigation + host-memory reservoir trim — 2026-09-04

Owner: "Why does Linux keep killing it? It didn't before."

## Findings

**5 OOM kills today, all llama-server** (12:43, 14:37, 15:34, 16:36, 16:50).
The kernel journal only retains ~9h (first entry 08:38) — "no OOM before
today" is unprovable from it; AGENTS.md documents Aug-31-era OOM deaths
(of unprotected serves). What IS new: the protected unit engine itself
got killed 5× in one day.

**Not the delta**: the process set is stable (Discord/zellij/plasmashell
since Aug 30; two opencode since Sep 2). The engine config was stable
for days.

**The engine is the system's dominant zram-swap holder** — 1.5GB right
after a restart, ~10GB at the 15:34 kill (that drove its oom_score to
767). Its host-RAM reservoir: the lull **checkpoint ring**
(`n_ctx_checkpoints`, default **32** in-RAM full-KV copies per slot;
confirmed common.h:597) at 82K ctx, plus `--cache-ram 1024`, plus
`--cache-idle-slots` host KV backup. Under `swappiness=150` on a
zram-only box (16G RAM + 16G RAM-backed swap, no disk swap), the kernel
eagerly swaps those cold pages; swapped pages dominate OOM badness.

Kernel caveat: this box's oom_score accounting is NOT the classic
RSS+swap formula — a 3MB process scored 803 (adj 200). cgroup-charged.
The oom-shield's +200/300 on big consumers matters less than intended
because base scores are huge. Regardless, the actionable metric is
actual reclaimable footprint (swap + resident), which the trim cut hard.

## Trim applied (plan A, root-cause)

1. **`kv.ctx_checkpoints` 32 → 4** (new launcher config key + flag
   `--ctx-checkpoints`; fingerprint gains `ckpts=`). The ring is the
   biggest swappable host allocation. 4 suffices for MTP draft
   rollback; 0 stays forbidden (heap corruption, AGENTS.md).
2. **`server.cache_ram` 1024 → 256**.
3. **`server.cache_idle_slots` off** — drops the host-KV backup path
   (single-slot setup; the idle KV now stays warm in VRAM).

Measured: **VmSwap 1.5GB → 51MB** after restart. oom_score 771 (below
the shielded node's 803 — the engine is no longer the guaranteed
victim). Smoke test: 17.5 t/s decode, MTP live. Journal flagged all
three deltas (`ckpts: new→4`, `cram: 1024→256`, `cis: new→0`); blessed
updated.

## Completion (B + C done 2026-09-04)

**B. 16G /swapfile active** (btrfs NOCOW via chattr +C before writing;
fallocate holes = "swapon Invalid argument" on btrfs — fixed in the
script). /proc/swaps: /dev/zram0 (15.6G, prio 100) + /swapfile (16G,
prio -2). Disk swap gives the kernel real reclaim headroom.

**C. SYSTEM-scope units live and verified:**
- engine pid cgroup = `/system.slice/vitriol-server.service`,
  `oom_score_adj = -500` (real negative — last OOM victim)
- vitriol-autosave.service also system scope; slot restore verified
  (301ms) under the new scope
- user units retired: stopped, disabled, unit files moved to
  `*.service.disabled`; the root script's original `systemctl --user`
  calls were a silent no-op (root has no user bus) — fixed to
  `runuser -u randozart -- env XDG_RUNTIME_DIR=/run/user/<uid>
  systemctl --user`; the script now FAILS loudly unless the engine is in
  the system cgroup with adj -500
- polkit rule lets randozart manage exactly the two vitriol units;
  the sidecar's restarts target the system bus via
  `VITRIOL_SYSTEMCTL=systemctl`

## Doctrine updates (AGENTS.md)

- `--ctx-checkpoints` is a load-bearing, fingerprint-carried knob:
  4 = reservoir-trimmed, 0 forbidden.
- cache-idle-slots off / cache-ram 256 are part of the blessed
  operating point.
- Restart-via-unit is now SYSTEM scope: `systemctl restart
  vitriol-server.service` (--user is the retired legacy).