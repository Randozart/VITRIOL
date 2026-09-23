# Optane Store Repartition + File Organisation — 2026-09-23

## Objective

Split the XFS partition on the Optane 900P (nvme0n1) to carve a **raw,
no-filesystem store zone** for architecture-dependent expert/KV streaming,
while keeping swap and the XFS scratch mount. XFS cannot shrink → the XFS
partition is wiped and recreated smaller; a new raw partition takes the freed
span.

## Hardware ground truth (probed 2026-09-23)

- Device: `nvme0n1` = INTEL SSDPED1D480GA (Optane 900P, 480GB). 512B sectors.
- `max_sectors_kb 128`, scheduler mq, write-through, `fstrim.timer` active.
- No IOMMU in cmdline → VFIO (Level 3 bypass) is NOT in scope. This plan is
  Level 2: raw partition + O_DIRECT + io_uring. No DAX possible (900P is NVMe,
  not pmem).
- `liburing` + headers installed (`/usr/include/liburing.h`).
- Fetch of `Qwen3.8-Flash-Next-UD-Q3_K_XL` completed: all 3 shards
  `ALL_SHARDS_OK`, 84 GiB total in `/scratch/qwen38fn/`.

## Current partition table (512B sectors)

| part | start | size (sectors) | end | content |
|---|---|---|---|---|
| p1 | 2048 | 134217728 | 134219776 | 64G swap (0B used; zram holds live swap) |
| p2 | 134219776 | 786432000 | 920651776 | 375G XFS `/scratch` |
| p4 | 920651776 | 17051278 | 937703054 | 8.1G unformatted — future boot, untouched |

p2 ends exactly where p4 starts (no gap). ~34-sector slack at disk end.

## Target layout

| part | start | size (sectors) | end | content |
|---|---|---|---|---|
| p1 | 2048 | 134217728 | 134219776 | 64G swap — never listed in sfdisk input |
| p2 | 134219776 | 201326592 | 335546368 | 96G XFS `/scratch` (new UUID → fstab update) |
| p3 NEW | 335546368 | 585105408 | 920651776 | 279G raw store zone — no mkfs |
| p4 | 920651776 | 17051278 | end | future boot — untouched |

1 MiB aligned (new starts ÷ 2048 exact). Old XFS primary + secondary supers all
land ≤48 GiB into the old partition → inside the new p2; `wipefs -a` on p3 head
added anyway.

## Rationale for raw zone vs filesystem

Current cold tier = offset-0 `mmap(MAP_PRIVATE|MAP_POPULATE)` of the GGUF
(`vitriol-buffer.cpp:164-208`). Problems:
1. MAP_PRIVATE + `set_tensor` memcpy COWs every page → file backing lost after load.
2. Page cache is a hidden 4th tier that fights VITRIOL's own RAM tier
   (mlock/`cudaHostRegister` pool) — OOM kingpin history.
3. Layout is ggml-buffer, not GGUF offsets → naive.

O_DIRECT into pinned host buffers removes kernel page cache from the tiering
equation: Optane → pinned RAM (VITRIOL LRU) → VRAM. One explicit copy, kernel
no longer a tier.

## Store format (v1)

```
zone 0: super 4K   magic, ver, slot_size, zone table, CRC  ← only disk read at open
zone 1: EXPERT     fixed 2MiB-aligned slots; dense bitmap+gen in RAM
         offset(tensor_base, expert_idx) = zone_base + slot * stride   ← O(1)
         matches existing LRU key {dev, tensor_base, expert_idx}
zone 2: KV ring    (P3) append-only, fixed 2MiB cell-blocks, overwrite-oldest
```

Architecture selection: MoE → experts; dense long-ctx → KV; both → both.
Profile keys: `store.path=/dev/nvme0n1p3`, `store.zones=experts|kv|both`.

## Phases

- **P0** (no root): stage `/scratch/qwen38fn` → `/home/randozart/Downloads/staging-qwen38fn`, verify.
- **P1** (sudo script): `scripts/vitriol-optane-repart.sh` — guarded repart, mkfs, fstab, stage-back.
- **P2**: `vitriol-store.{h,cpp}` (open/mkfs/read_slot, pread + io_uring O_DIRECT), `scripts/vitriol-store-mkfs` (GGUF→slots), wire-in at `vitriol-buffer.cpp:164-208`, `vitriol-cuda-integration.cpp:862/957`.
- **P3**: KV ring dual-write at `llama-kv-cache.cpp:2338` / slot-save.
- **P4**: benchmark vs mmap path (unit 3×64 + filled ladder). If kernel overhead < ~20% of fetch time → stop at Level 2; VFIO deferred to separate plan.

## Risks

| risk | mitigation |
|---|---|
| sfdisk slides p1 | explicit starts; p1/p4 absent from input; pre/post assert |
| umount /scratch fails | lsof gate in script |
| fstab UUID miss | script prints+applies, findmnt assert |
| stage loss | verify before wipe; rsync back before stage rm |
| engine holding /scratch | lsof gate |

## Status

- 2026-09-23 10:0x P0 staging in progress.