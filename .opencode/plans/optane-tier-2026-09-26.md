# Optane XFS tier for the VITRIOL pipeline — 2026-09-26

Status: approved by user (plan mode), executing.

## Goal

One fast, concurrent, µs-latency tier for everything the VITRIOL daily-driver
pipeline touches: KV session persistence, build trees, model files. Grown
`/scratch` (xfs) over the raw flash-next store partition on the Optane
(SSDPED1D480GA, ~2.5 GB/s, µs-class latency). No OS migration, no bootloader
changes (Limine p4 untouched), /home btrfs untouched.

## Target layout

| partition | size | role |
|---|---|---|
| nvme0n1p1 | 64G | swap (unchanged) |
| nvme0n1p2 /scratch — GROWN | ~371G xfs | kvtier + builds + models (+ store-as-file slot for future streaming research) |
| nvme0n1p4 | 8.1G | Limine /boot (untouched) |
| sda6 | 1.9T btrfs | /home, sources (untouched) |

## Steps

1. Stop `vitriol-server.service` (clean DMA shutdown); verify no compute
   procs on the NVMe.
2. ROOT (user runs; passwordless sudo unavailable):
   `sudo sgdisk --delete=3 /dev/nvme0n1 && sudo growpart /dev/nvme0n1 2
   && sudo xfs_growfs /scratch`
   - deletes p3 (flash-next store — REBUILDABLE: store-on-file on this fs
     via O_DIRECT keeps the streaming research alive; source GGUFs remain
     on /scratch)
   - grows p2 to the p4 boundary; xfs online grow, p2 data untouched
3. `mkdir /scratch/kvtier /scratch/builds`; move + symlink in place:
   - `llama.cpp/build`, `llama.cpp/build-ku-cu12`, `vitriol-stream/target`
   - in-place symlinks keep LLAMA_DIR / unit ExecStart / SERVER_BINARY
     paths valid (stale-path landmine doctrine)
4. Config: `slot_save_path = /scratch/kvtier`; `parallel = 2` (two warm
   sessions in VRAM: 11.3G weights + 2x typical session KV). Restart
   service via unit.
5. Fingerprint diff (recorded; path keys not fingerprinted, provenance
   here), session-switch timing validation, greedy probe + suite.

## Milestone B (next, scoped after validation)

Optane-backed session LRU in `tools/server/server-context.cpp`:
save-on-release/timeout, restore-on-demand by session id (automatic,
manual /slots/load fallback), recurrent/SSM state included, ~50+ sessions
capacity at ~5-10G/slot. Restores in ~0.3-2s vs minutes of re-prefill.

## Risks

- p3 wipe kills the flash-next store (approved; rebuildable as file).
- growpart/xfs_growfs leave p2 data untouched (standard op).
- build-tree move: in-place symlinks + verified-binary protocol
  (grep .so for probe string before launch) per doctrine.
- Stale-binary landmine: every probe verifies the .so contains the
  expected probe string BEFORE conclusions are drawn.

## Log

- 2026-09-26: plan written; executing.
- 2026-09-26 22:3x: DONE.
  - Disk: p3 deleted, p2 grown to 920651775s (off-by-one lesson: parted
    resizepart END is inclusive; 100% overlapped p4), xfs_growfs online:
    /scratch 96G -> 375G (98304000 blocks). p4 Limine untouched.
  - Tier: /scratch/builds + /scratch/kvtier created (root-owned /scratch
    needed one sudo mkdir+chown).
  - Discovery: llama.cpp/build was ALREADY a symlink to build-ku-cu12
    (the "two trees" hardlink mystery explained - one tree, two paths).
    Moved build-ku-cu12 (997M) + vitriol-stream/target (63M) to
    /scratch/builds, symlinked in place; cmake configure re-run (VITRIOL_RUST_LIB
    env is per-configure - the old absolute path died with the move; the
    per-build env export is now a known-pitfall in the doctrine).
  - setcap note: mv across devices dropped a security.capability xattr on
    one moved binary (warning); no functional impact observed.
  - Config: slot_save_path=/scratch/kvtier, parallel=2. Service restarted
    via unit. Fingerprint DIFF (intentional only): par=1->2; ct= new key
    (chat-template-tolerant.j2 now fingerprinted). All else identical to
    blessed.
  - Validation: greedy ' Paris.' 28.6 t/s; suite 7/8 + checker artifact
    (identical to pre-move baseline); 2-slot rotation A/B/A/B: zero
    re-prefill on warm switches (par=2 keeps both sessions VRAM-resident).
  - NEXT: Milestone B design (Optane-backed session LRU,
    server-context.cpp) against real deep-fill switch numbers; user's
    session-count answer still wanted.
