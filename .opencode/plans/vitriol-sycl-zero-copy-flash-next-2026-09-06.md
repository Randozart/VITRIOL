# VITRIOL SYCL Zero-Copy Streaming - Flash-Next First Run

2026-09-06 ~20:30-23:00 CEST

Continues `vitriol-sycl-dispatch-fix-2026-09-06.md` (commit `5fb6a67a9`).

## What was built

Zero-copy mmap wrap mode for VITRIOL SYCL buffers (commit `9eba8229e`).
Expert weights are no longer copied into pinned host memory: the model
loader wraps the GGUF files' file-backed mmap pages as VITRIOL buffers
(new procs `vitriol_buft_supports_host_ptr` / `vitriol_buffer_from_host_ptr`
queried by src/llama-model.cpp), and the LRU hooks DMA from those pages
into the device pool on demand. RAM cost = page cache only.

## Bugs fixed this session (chain from wrap mode)

1. **supports_op gate only checked src[0]** - for MUL the weight is
   src[1], so DENSE weights (attn_norm, etc.) also selected VITRIOL
   buffers. Pinned mode survived by accident (malloc_host = USM,
   device-readable); wrapped mode crashed with DEVICE_LOST because plain
   mmap pages are not USM. Fix: scan ALL sources. This was the crash
   behind wrap1-wrap8 (every "clean" retry after was wedged-device noise).

2. **vitriol_sycl_is_vitriol_buffer_type name-sniffed foreign contexts**
   (cast buft->context and compared a std::string) - UB on foreign buft
   types. Fix: pointer identity against the per-device singletons.

3. **L0 copy engines cannot page-fault.** First DMA from never-faulted
   file-backed pages = GPU page fault = DEVICE_LOST. Fix:
   `vitriol_sycl_touch_pages`: MADV_WILLNEED (kernel readahead) + 1-byte
   touch per page before every DMA (both lru_ensure and prefetch paths).

4. **LRU slot size fixed at first expert size.** Flash-Next down experts
   (0.92 MB) are larger than gate/up (0.47 MB): lru_ensure returned
   nullptr, the fallback handed the raw mmap pointer to a device kernel
   (DEVICE_LOST after exactly 16 good DMAs). Fix: `lru_resize` re-creates
   the pool with the larger slot size, draining in-flight DMAs first.

5. **Wedge discipline (operational, this host):** a kill -9 (or an abort)
   of a live L0 process leaves the next process with 0 SYCL devices or
   hanging copies. `sycl-ls` clears it; needs ~20-30 s. Several "crashes"
   this session were wedge echoes of the previous crash, not new bugs.
   Rule: SIGTERM servers; after any -9/crash, run sycl-ls and wait.

## Flash-Next results (qwen4exp, 74 GB, 512 experts, 10 active, Q2_K_XL)

All: -ngl 0, t 8, build 143 `9eba8229e`, VITRIOL_LRU_MB=4096 unless noted.
Load: ~13 s, zero-copy (19.4 GB + 27.5 GB wrapped ranges across shards),
RAM ~10 GB used + 49 GB page cache at idle.

| config | pp512 | tg64 |
|---|---|---|
| VITRIOL wrap, LRU 4 GB, sync DMA | 76.6 +/- 7.6 | **3.36 +/- 0.01** |
| VITRIOL wrap, LRU 8 GB | 60.8 +/- 9.5 | 2.94 +/- 0.40 |
| VITRIOL wrap, async DMA, LRU 4 GB | - | 2.95 |
| CPU-only (prior session baseline) | 82.1 | 7.69 |

LRU at 4 GB: 42.6% hit rate over ~125k accesses (53300 hits / 71714
misses / 67016 evictions). Misses are disk-bound (NVMe, ~0.92 MB random
slices). 8 GB LRU is WORSE: on UMA the device pool steals page cache from
the model. Async DMA regresses (lru_sync waits only one slot; predictor
prefetch thrashes).

Correctness: coherent paragraph generation and correct arithmetic
(17x23 reasoning intact) through the full streaming path.

GLM-4.7-Flash (17 GB, fits in RAM) in wrap mode: pp 16.6, tg 10.8 - vs
pinned-VITRIOL 15.9 and CPU-only 27.4. Residency rule holds: streaming a
RAM-fitting model is a pessimization; wrap < pinned < CPU there.

## Verdict and economics

- Prefill: VITRIOL wrap reaches 93% of CPU-only on a 74 GB model that
  previously ran CPU-only at best - and the GPU now does the MMID GEMMs.
- Decode: CPU-only still 2.3x faster. The CPU reads the same disk
  directly through mmap fault-around + readahead at ~6.9 GB/s effective;
  the VITRIOL path pays per-slice touch + DMA + sync on top of the same
  disk reads. VITRIOL decode wins require a HIGH LRU hit rate, i.e. a
  hot expert set that fits the LRU budget - not the case at 10/512
  sparsity over 47 GB of expert bytes.
- Architectural conclusion for UMA: VITRIOL streaming's LRU exists for
  discrete GPUs (kernels cannot read host memory across PCIe). On iGPU
  UMA the DMA is a copy within the same physical RAM; its value is
  bounding device-pool memory, not bandwidth. For oversize models the
  disk is the shared bottleneck and the CPU's direct-read path is
  currently the better decode engine.

## Next options (not done)

1. Async DMA done right: wait ALL in-flight slots in lru_sync (currently
   only g_lru_last_slot), gate the predictor prefetch to only fire when
   the LRU has slack; re-measure decode overlap.
2. pread-based fault-in (fd plumbed through the wrap buffer) - one
   syscall per slice instead of per-page faults.
3. Mixed ngl: attention/dense layers resident on device, experts
   streamed - modest expected gain (see report in dispatch-fix doc).
4. Accept CPU-only decode for oversize models on UMA and use VITRIOL
   streaming for prefill-heavy workloads (summarization, long-context
   ingest) where pp512 is near parity and the GPU does the GEMMs.

## Logs

- /tmp/vitriol-flash1.log - first Flash-Next load + inference (coherent)
- /tmp/vitriol-flashbench1.log - pp512 75.82, tg64 3.36 (LRU 4 GB)
- /tmp/vitriol-flashbench2.log - LRU 8 GB regression (2.94)
- /tmp/vitriol-flashbench3.log - madvise touch (76.55 / 3.32, noise)
- /tmp/vitriol-wrap9-11.log - wrap-mode bring-up on GLM
- /tmp/vitriol-wrap6.log - scheduler dump proving MUL_MAT_ID on SYCL0
  with claimed (uncopied) expert weights

## Commits

- Inner llama.cpp `9eba8229e` - zero-copy wrap + gate/identity/resize fixes
- (prior) `5fb6a67a9` - dispatch chain + LRU init deadlock
