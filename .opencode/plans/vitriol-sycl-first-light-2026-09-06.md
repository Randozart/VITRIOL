# VITRIOL SYCL Streaming: First Light Findings & Plan — 2026-09-06

## Executive Summary

VITRIOL SYCL streaming is **functional** on Arc B390 iGPU. Flash-Next
(176.9B params, 74GB Q2_K) loads and runs. Warm decode (9.1 t/s) beats
CPU-only (7.69 t/s) by 18%. Prefill is bottlenecked by synchronous DMA.

## What We Learned

### 1. Model loader integration was the blocker

The VITRIOL SYCL buffer type existed but was invisible to the model
loader. The SYCL backend's `get_proc_address` didn't expose
`ggml_backend_dev_get_extra_bufts`, so expert tensors were allocated
using the standard SYCL buft (GPU-accessible), not the VITRIOL buft
(host-pinned via `sycl::malloc_host`).

Fix: registered `vitriol_sycl_get_extra_bufts` in the SYCL reg's
`get_proc_address`. The model loader now discovers VITRIOL bufts and
allocates expert weights in host-pinned memory.

### 2. SYCL in-order queue kills async DMA

The CUDA VITRIOL uses **two separate streams**: a dedicated DMA stream
and the compute stream. The DMA fires asynchronously, and the compute
stream waits via `cuStreamWaitEvent` only when it actually needs the data.

The SYCL VITRIOL uses a **single in-order queue**:
```cpp
g_lru_queue = new sycl::queue(devices[0],
    sycl::property_list{sycl::property::queue::in_order{}});
```

This serializes everything. The DMA copy is technically async (returns
an event), but the next line immediately `.wait()`s on it, making it
effectively synchronous.

**Impact**: Cold prefill pp=1.5 t/s (vs 82.1 CPU-only). Each prompt
token's expert access copies from host→device synchronously.

### 3. Warm decode is faster than CPU

When experts are in the LRU cache (device-local memory), SYCL kernel
execution is faster than CPU memory access:

| Metric | VITRIOL warm | CPU-only | Delta |
|--------|-------------|----------|-------|
| tg (32 tok) | 9.1 t/s | 7.69 t/s | +18% |
| pp (1 tok) | 8.6 t/s | 82.1 t/s | -90% |

The tg gain proves the LRU cache approach works. The pp regression is
purely from synchronous DMA.

### 4. SYCL LRU has a simpler eviction than CUDA

The CUDA VITRIOL has a **three-state eviction** (uses `cuEventQuery` to
skip in-flight slots). The SYCL version always `.wait()`s before
evicting, which is correct but slower. With async DMA, we'll need the
three-state approach.

### 5. Flash-Next fits in 2GB device-local LRU

The LRU pool (2048 MB, `sycl::malloc_device`) holds enough hot experts
for active decode. The model is 74GB total, but only ~2GB of expert
weights are active per decode step (2-4 experts x ~500MB each).

## Current Architecture

```
┌─────────────────────────────────────────────────┐
│ SYCL Backend (in-order queue)                    │
│                                                  │
│  ┌──────────┐    sync DMA    ┌──────────────┐   │
│  │ Host-Pin │ ──────────────→│ LRU Pool     │   │
│  │ (malloc_ │    .wait()     │ (malloc_     │   │
│  │  host)   │                │  device)     │   │
│  └──────────┘                │ 2GB          │   │
│       ↑                      └──────┬───────┘   │
│       │ model loader                 │           │
│  ┌────┴─────┐              ┌────────┴────────┐  │
│  │ GGUF file│              │ SYCL Kernels    │  │
│  │ (mmap)   │              │ (MUL_MAT_ID)    │  │
│  └──────────┘              └─────────────────┘  │
└─────────────────────────────────────────────────┘
```

## Plan: Next Phase Optimizations

### Phase A: Async DMA (Priority: CRITICAL) - IMPLEMENTED 2026-09-06

**Goal**: Make host->device DMA non-blocking. Pipeline copies.

**What was done**:
1. Added `VITRIOL_ASYNC_DMA=1` env var to control async mode
2. Modified `vitriol_sycl_lru_ensure()` to skip `.wait()` when async mode is on
3. Added `vitriol_sycl_lru_sync()` function that waits on the last-fired DMA event
4. Modified hooks in `ggml-sycl.cpp` to call `vitriol_sycl_lru_sync()` before `ggml_sycl_mul_mat`
5. Added predictor calls to both hook paths (ne12==1 and ne12!=1)
6. Enabled predictive prefetch by default (was off by default)

**How it works**:
- Without async: lru_ensure fires DMA, `.wait()` (blocks ~0.5ms), returns. Then compute.
- With async: lru_ensure fires DMA (returns immediately). The hook calls `vitriol_sycl_lru_sync()` which waits on the last-fired event. Then compute.
- Predictor fires DMA for NEXT layer's experts (async, no wait), which overlap with CURRENT layer's compute.

**Files changed**:
- `vitriol-sycl-integration.cpp`: +15 lines (async flag, last_slot tracking, sync function)
- `vitriol-sycl-buffer.cpp`: +8 lines (async_dma config, enabled-by-default prefetch)
- `vitriol-sycl-buffer.hpp`: +2 lines (declarations)
- `ggml-sycl.cpp`: +8 lines (predictor calls in both hooks)

**Expected improvement**: The predictor prefetch is the real win. By firing DMA for next-layer experts while current-layer computes, we hide DMA latency behind compute. Cold prefill pp should improve from ~1.5 to ~5-10 t/s.

**Build status**: Compiles successfully (100% built target llama-server)

### Phase B: Predictive Prefetch (Priority: HIGH)

**Goal**: Prefetch experts from the NEXT layer before the current
layer finishes.

**Already implemented** in `vitriol-sycl-integration.cpp`:
- Cross-layer prediction: uses experts from previous layer
- Temporal prediction: uses experts from same layer of previous token
- But currently disabled (`VITRIOL_PREDICTIVE_PREFETCH=0` by default)

**Changes needed**:
- Enable by default in `vitriol_sycl_init()`
- Fix the predictor to work with async DMA (currently assumes sync)
- Add the "three-state eviction" to avoid evicting in-flight prefetches

**Expected improvement**: Cold prefill pp from ~10 → ~20-30 t/s
(hide DMA latency behind compute)

### Phase C: Vulkan Port (Priority: MEDIUM)

**Goal**: Port VITRIOL streaming to Vulkan backend (1.9x faster decode).

**Key differences from SYCL port**:
- Vulkan uses `vkCmdCopyBuffer` for DMA (command buffer-based)
- Vulkan has native timeline semaphores for synchronization
- Vulkan can do true concurrent DMA + compute via queue families
- DSV4_HC shaders already ported (committed `f7b93e4a1`)

**Approach**:
1. Reuse host-side logic (LRU, predictor) from SYCL
2. Rewrite DMA layer for Vulkan (`vkCmdCopyBuffer` + timeline semaphores)
3. Add VITRIOL buft type for Vulkan (host-visible, device-local via BAR)
4. Register in Vulkan backend's `get_proc_address`

**Estimated effort**: ~870 lines (mostly Vulkan boilerplate)
**Expected improvement**: tg from ~9.1 → ~15-20 t/s (1.9x Vulkan advantage)

### Phase D: Larger Context (Priority: MEDIUM)

**Goal**: Run with c=4096+ for real use.

**Changes needed**:
- Increase `c=4096` and adjust `ub` accordingly
- Test with larger prompts (the current c=512 limits prompt to ~200 tokens)
- Monitor LRU eviction rate at higher context

### Phase E: VITRIOL_VERBOSE Logging (Priority: LOW)

**Goal**: Log per-operation LRU stats (hits, misses, evictions).

**Changes needed**:
- Add `fprintf(stderr, ...)` in `vitriol_sycl_lru_ensure()` for misses
- Add periodic stats dump (every N misses)
- Fix the existing `vitriol_sycl_print_stats()` to work with server shutdown

## Benchmark Reference (Current)

| Model | Config | pp | tg | Notes |
|-------|--------|----|----|-------|
| Flash-Next Q2_K | -ngl 0 -c 512 VITRIOL warm | 8.6 | 9.1 | LRU hit path |
| Flash-Next Q2_K | -ngl 0 -c 512 CPU-only | 82.1 | 7.69 | Baseline |
| Flash-Next Q2_K | -ngl 20 partial GPU | 27.2 | 7.11 | GPU-CPU overhead |

## Commit Reference

- Inner: `8ce0396eb` — VITRIOL SYCL streaming integration
- Outer: `251adb3` — EXPERIMENT_LOG update

## Risk Assessment

1. **SYCL queue concurrency**: In-order queues may serialize DMA + compute.
   Mitigation: try out-of-order queue, or accept pipeline serialization
   and overlap at the expert granularity (prefetch N+1 while computing N).

2. **Memory pressure**: 74GB model + 2GB LRU + 512 context KV = ~77GB.
   On 62GB RAM, this means heavy swap. Mitigation: reduce LRU to 1GB,
   or use c=2048.

3. **Vulkan port complexity**: 870 lines of Vulkan boilerplate. Mitigation:
   Reuse SYCL host-side logic, only rewrite the DMA layer.

## Recommendation

**Immediate next step**: Phase A (async DMA) — this is the single biggest
performance lever. The current synchronous path makes prefill unusable.
With async DMA, we can pipeline copies and overlap with compute.

**Secondary**: Phase B (predictive prefetch) — already implemented, just
needs enabling and testing with async DMA.

**Tertiary**: Phase C (Vulkan port) — worth it for the 1.9x decode
advantage, but lower priority than making the SYCL path fast first.
