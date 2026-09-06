# VITRIOL SYCL Dispatch Chain - Root Cause and First Light

2026-09-06 ~18:00-20:30 CEST

## Objective

Diagnose why VITRIOL SYCL LRU hooks never fired (5 stacked gates found), fix
them, and get first working expert streaming on the Arc B390 iGPU.

## Root Cause Chain (5 gates, all fixed)

With `-ngl 0`, MUL_MAT_ID must route to SYCL with expert weights in
VITRIOL pinned-host buffers. Five independent gates blocked the chain:

1. **VITRIOL bufts never in any buft list.** `make_cpu_buft_list`
   (src/llama-model.cpp) only queried the CPU backend's extra bufts.
   VITRIOL bufts live on the SYCL reg. Fix: also query GPU/IGPU backends'
   `ggml_backend_dev_get_extra_bufts` and prepend them (before the host
   buffer type) to the CPU buft list.

2. **`vitriol_sycl_get_extra_bufts` matched dev by pointer and failed.**
   The loader passes registry device instances whose pointer identity does
   not match `buft->device`. Fix: return device 0's array unconditionally
   (single-iGPU rig; multi-GPU SYCL needs index matching later).

3. **IGPU type filter.** Intel Arc B390 reports as
   `GGML_BACKEND_DEVICE_TYPE_IGPU`, not GPU. First fix attempt filtered
   `!= GPU` and silently skipped the iGPU. Fix: accept both GPU and IGPU.

4. **mmap redirect + SYCL_Host shadowing.** Even before the fix,
   `select_weight_buft` picked `SYCL_Host` (host buft section precedes
   extra bufts), and the loader's "avoid using a host buffer when using
   mmap" redirect (llama-model-loader.cpp:1268) converted it to a plain
   CPU buffer - expert weights never left CPU. Fix: VITRIOL bufts are
   placed FIRST, survive the redirect (they are not the device's host
   buft), and SYCL claims them (see 5) so the scheduler routes the op
   WITHOUT copying weights.

5. **supports_op gate.** VITRIOL bufts must only serve MUL_MAT_ID; any
   other op on a vitriol buffer would read host pointers as device USM.
   Fix: gate at the top of `do_ggml_backend_sycl_device_supports_op` -
   reject vitriol-buffer src0 for all ops except MUL_MAT_ID. Dense
   weights fall through to CPU; only expert tensors select VITRIOL.

## Bonus bugs found on the way

- **LRU init self-deadlock** (vitriol-sycl-integration.cpp):
  `lru_init_pool` took `g_lru_init_mtx`, then called `lru_ensure_queue`
  which locks the same non-recursive mutex. Every LRU init would have
  hung forever. This also retro-explains why LRU stats NEVER appeared in
  earlier "first light" tests - the hooks were never reached, and had
  they been, the pool init would have deadlocked.
- **Fused path bypass**: `ggml_sycl_mul_mat_id_mmvq_fused` (decode,
  ne12==1) reads `src0->data` directly - a host pointer when streaming.
  Bypassed when VITRIOL is enabled so the hooked loop (LRU swap at
  src0_row.data) runs instead.
- **Level-Zero wedge**: kill -9 of a live L0 process leaves the next
  process with 0 SYCL devices (`ggml_sycl_init: device_count == 0`,
  logged as "SYCL: failed to initialize"). sycl-ls clears it.
  Operational rule for this host: SIGTERM servers, never -9 a live one;
  after any -9, run `sycl-ls` before relaunching. (Supersedes the
  blanket "always killall -9" rule for the iGPU rig.)

## How the routing works now

Model load (ngl 0):
  cpu_buft_list = [VITRIOL_SYCL(sycl_dev), SYCL_Host(sycl_dev),
                   CPU extras (KLEIDIAI/REPACK), CPU]
  expert tensors (ffn_{gate,up,down}_exps) -> VITRIOL_SYCL
    (weight_buft_supported: SYCL supports_op accepts MUL_MAT_ID on a
    vitriol dummy buffer; everything else falls through)
  dense weights -> CPU / CPU_REPACK (VITRIOL gate rejects them)

Graph schedule:
  MUL_MAT_ID node: src0 buffer usage=WEIGHTS, in a vitriol buffer
  -> ggml_backend_sched_backend_from_buffer: SYCL supports_buft(vitriol)
     (ggml-sycl.cpp:6645-6655) AND supports_op -> SYCL wins (1.wgt0)
  -> weights STAY in pinned host memory, no split-input copies
  -> ggml_sycl_mul_mat_id hooks: LRU ensure -> DMA expert to device pool
     -> src0_row.data swapped to LRU device copy -> compute

Decode: fused path bypassed, hooked loop runs per expert.
Prefill (ne12 != 1): hooked loop at the ne12!=1 branch (already wired).

## Verification (GLM-4.7-Flash Q4_K, ngl 0, c 2048, ub 2048, t 8)

- Load: 16 GB pinned host buffer ("allocated 15993 MiB host-pinned
  buffer"), ~10 s warm.
- LRU pool: 4096 MB, 2427 slots x 1769472 bytes.
- Hooks fire: `ggml_sycl_mul_mat_id` with `src0 buf=VITRIOL_SYCL`.
- LRU stats: hits/misses/evictions counting (53% hit rate warm on GLM).
- Output coherent, HTTP 200, decode 15.3-15.9 t/s.
- vs CPU-only baseline 27.4 t/s: VITRIOL is slower ON A MODEL THAT FITS
  IN RAM - exactly the residency rule (AGENTS.md: streaming a fitting
  model is a pessimization). GLM was used because it fits and exercises
  the full path. Flash-Next (74 GB > 62 GB RAM) is the real target.

## Flash-Next blocker (next work item)

VITRIOL bufts COPY expert weights into pinned host memory at load.
Flash-Next is 74 GB on a 62 GB box: full pinned copies OOM. Options:

- Port CUDA VITRIOL's lazy page locking (`vitriol_get_buffer_type_lazy`,
  max_locked_mb budget; mmap-backed fallback, DMA from pageable memory
  is legal on iGPU since there is no PCIe - it is the same LPDDR).
- OR wrap mmap regions without copying (buffer_from_host_ptr style).

The LRU DMA already reads from arbitrary host pointers, so unpinned
experts take the slower pageable-DMA path - acceptable on UMA.

## Logs

- /tmp/vitriol-glm17.log (first successful hook fire, VITRIOL_SYCL)
- /tmp/vitriol-glm18.log (post-deadlock-fix, LRU stats, 15.3 t/s)
- /tmp/vitriol-final.log (clean build verification, 15.9 t/s)

## Commit

Inner llama.cpp: `5fb6a67a9` "VITRIOL-SYCL: fix expert buffer discovery,
routing, and LRU init deadlock" (4 files, +48 -12).
