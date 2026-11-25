# Is the Kintsugi engine start thrashing? — measurement note

2026-11-24. Owner hypothesis: the engine start is thrashing (both Kintsugi and
Inkling failing). Investigated read-only against the live process, their
launcher source, and the engine log. **Verdict: it does not thrash — it is
swapping zero bytes and doing two silent five-minute CPU phases. The real
defect is the KV tier collapsing to its floor because the launcher's VRAM
accounting was polluted by a leftover engine.**

## Evidence

### 1. It is not swapping

```
/proc/1409254/status:  VmSwap: 0 kB     VmRSS: 8,120,404 kB   VmHWM: 12,206,088 kB
/proc/vmstat pswpin/pswpout over 10 s:  609950 -> 610182 (paged IN)  = +232 pages ≈ 1 MB
                                       1940887 -> 1940887 (paged OUT) = 0
free: Mem 40,018 MB total / 5,839 free / 22,920 buff-cache; Swap 105 GB, 3.3 GB used (zram)
```

A process that is thrashing has non-zero `VmSwap` and rising `pswpout`. This
one has never had a page swapped out. The 3.3 GB of swap in use belongs to
other things (browser, IDE), not the engine.

### 2. The load is disk-trivial and IO-flat

```
/home is /dev/sda6 btrfs, 1.9 T, 90 % used;  /sys/block/sda/queue/rotational = 0  (SATA SSD)
engine /proc/io read_bytes = 1,519,054,848 and static across a 5 s sample after load
```

10.5 GB of weights are mmap'd and mostly served from page cache (22.9 GB
buff/cache). Disk is not the bottleneck.

### 3. What the 10.6 minutes actually are

Engine log timestamps (`MM.SS.mmm.uuu`):

| t | event |
|---|---|
| 0.00 | `load_model: loading model` |
| 0.07 | `graph_reserve: failed to allocate compute buffers` → 34,938,029,568 B; retry without pipeline parallelism |
| 0.07 → 5.20 | **silent 5 m 13 s** — weight upload + tiered-KV initialisation |
| 5.20 | loading the DFlash2 drafter (1.1 GB, 3 s — cache-warm) |
| 5.23 → 10.37 | **silent 5 m 14 s** — draft-context construction (MTP window 16,384 + DFlash `n_ubatch` 64) |
| 10.37 | `model loaded`, listening on 127.0.0.1:18290 |

Two near-equal silent halves. The second half is drafting and is removable
(`KINTSUGI_DRAFT=off`); the first scales with context and KV cells.

### 4. The actual defect: KV tier collapsed to its floor

The launcher computes KV cells from **card 0 only**:

```bash
IFS=',' read -r TOTAL USED DISP < <(nvidia-smi --query-gpu=memory.total,memory.used,display_active --format=csv,noheader,nounits | head -1)
```
(`~/kintsugi/Kintsugi-27B/start-server.sh:147`)

and what it printed for this box:

```
note   only 3328 K/V cells fit in VRAM: decode reads most of the K/V cache from system RAM
kv     q4_0, tiered: cells 0..3328 in VRAM, 3328..262144 in system RAM
       (free 6344 MiB, reserved 361, ceiling 11919, fixed 9632, growth 350, margin 600 (display on this card))
```

**3,328 of 262,144 cells = 1.3 % in VRAM.** Free was 6,344 MiB because a
previous engine still held ~5.9 GB on dev0 — `stop.sh` had not killed it (the
earlier "restart failed, model load error" cycle left `llama-server` alive;
`nvidia-smi --query-compute-apps` showed `llama-server 5112 MiB / 5410 MiB`
before the restart that finally succeeded). `cells_for()` clamps at a 3,328
floor, so the launcher happily started a 262k engine that reads 98.7 % of its
K/V over PCIe on every step.

So the observed "it never finishes loading / it is thrashing" is really:
load #2 was launched on top of load #1's VRAM, then ran with a degenerate KV
tier.

### 5. Our box violates one of their assumptions

Their whole budget assumes **one 12 GB card holds the model**. Here
`-ngl 99` splits layers across both 3060s (log shows
`CUDA0_tier_kv…` and `CUDA1_tier_kv…` with `no staging alias … host tail is
read over PCIe in place`), while `CELLS` was derived from dev0's free space
alone. The tier is per-device, so the cell count is set for the *smaller*
half of a 24 GB problem.

## Recommendations (for the trial, not yet executed)

1. **Start-clean invariant**: before every start, require
   `nvidia-smi --query-compute-apps=pid,used_memory --format=csv,noheader` to
   be empty apart from desktop processes; `stop.sh` is not sufficient after a
   failed load.
2. **Set cells explicitly** on this box: combined free VRAM is ~14 GB after
   weights, so `KINTSUGI_CELLS=65536`–`98304` should fit the working set in
   VRAM instead of 3,328. Verify against their warning line
   (`WARN KINTSUGI_CELLS=… is more than this card can hold`).
3. **Cut the load time** for trials: `KINTSUGI_DRAFT=off` (removes the second
   5-minute phase), `KINTSUGI_CTX=65536`, `KINTSUGI_UB=256` (smaller pp
   reservation than the 34 GB attempt at `-ub 1024`).
4. **Pin the card** if the split misbehaves: `CUDA_VISIBLE_DEVICES=0` with the
   model on one card, matching their tuned path (262k window then needs the
   KV tier in RAM, which is what their 3060 report measures).
5. Do not read `-ub 1024` as a quality knob: their own README says 512 costs
   ~146 MiB and 8–13 % prompt-read speed.

## Inkling is a different failure

Inkling's `cudaMalloc failed` was a **GPU compute-buffer** reservation
(7.7–8.5 GB), reproduced with the pre-patch graph and with
`VITRIOL_FA_BIAS=0`, and it is ub-independent and FA-independent
(see `inkling2-per-head-fa-mask-2026-11-24.md`). Nothing to do with swap or the
RAM Shot store, which is disabled (`mode=off`, `store=0`).

## Open questions this note does not answer

- Which phase inside the first silent 5 minutes dominates (weight upload vs
  per-layer tier initialisation). Needs `GGML_SCHED_DEBUG`-level tracing or
  `perf` on the loader.
- Whether cells above ~65k are safe with the display resident; their warning
  line is the only oracle.
