# VITRIOL Flash-Next Optimization Plan — 2026-09-06

## Executive Summary

Flash-Next (Qwen3.8-Flash-Next, 176.9B params, 74GB Q2_K) is the quality
king but currently the slowest model in our fleet. Warm decode at 9.1 t/s
beats CPU-only (7.69 t/s) by 18%, but prefill is crippled by synchronous
DMA. This plan outlines the path to make Flash-Next competitive.

## Architecture Analysis

### Flash-Next (qwen4exp)

| Parameter | Value |
|-----------|-------|
| Total params | 176.9B (512 experts x 48 layers) |
| Active per token | 10 experts (1.95% sparsity) |
| Expert size (Q2_K_XL) | ~1.54 MB per expert |
| Per-layer expert total | ~693 MB (512 experts) |
| Working set per forward | ~13 GB (active experts + attention + SSM) |
| Model on disk | 74 GB (3 shards) |
| Context length | 262,144 tokens |

### Memory Budget (Panther Lake, 62 GB LPDDR5X)

| Resource | Available | Used by Flash-Next |
|----------|-----------|-------------------|
| RAM | 62 GB | 74 GB mmap (needs swap) |
| GPU VRAM | Shared LPDDR5X | VITRIOL LRU 2-4 GB |
| zram swap | 62 GB | Partial relief, OOM risk |

### Key Insight: Extreme Sparsity

512 experts per layer, 10 active = 98% of expert weights are NEVER needed
per token. In a typical session, many experts are never activated at all.
The LRU cache only needs to hold the hot experts, not the full model.

## Current State

### What Works (First Light, 2026-09-06)

- Server starts, loads 74GB Flash-Next, health check OK
- Expert weights allocated in host-pinned memory (sycl::malloc_host)
- LRU cache (2GB device-local) holds hot experts
- Warm decode tg=9.1 t/s beats CPU-only 7.69 t/s by 18%
- No crashes, no OOM

### What's Broken

1. **Prefill**: pp=1.5 t/s (cold), 8.6 t/s (warm) — synchronous DMA blocks
2. **Predictive prefetch**: Implemented but not tested with async DMA
3. **LRU size**: 2 GB may be too small for 13 GB working set
4. **Context**: c=512 too small for real use

## Optimization Phases

### Phase 1: Async DMA + Predictive Prefetch (IMPLEMENTED + TESTED)

**Results:**
- Cold pp: 1.5 -> 12.2 t/s (+8.1x)
- Cold tg: 5.4 -> 9.1 t/s (+1.7x)
- Warm pp: 8.6 -> 14.4 t/s (+1.7x)

### Phase 2: LRU + Context Tuning (COMPLETED)

**Optimal config:**
- LRU: 4 GB (8 GB doesn't help over 4 GB)
- ubatch: 2048 (key factor for longer prompts)
- Context: 2048 (c=4096 doesn't help over c=2048)

**Final benchmark:**
- Cold pp: 12.2 t/s (+8.1x from first light)
- Cold tg: 9.1 t/s (+1.7x)
- Warm pp: 14.4 t/s (+1.7x)
- Warm tg: 9.2 t/s (~same)
- Long prompt pp: 14.4 t/s (+18x from first light)

### Phase 3: Three-State Eviction (IMPLEMENTED + TESTED)

Ported from CUDA VITRIOL. Skips in-flight DMA slots during eviction.
Biggest win on longer prompts (0.8 -> 11.1 t/s before tuning).

The CUDA VITRIOL (1158 lines) has features the SYCL port (368 lines) lacks:

#### 3a. Three-State Eviction (CRITICAL)

CUDA VITRIOL skips in-flight slots during eviction:
```cpp
// PROVENANCE: kimi-k3-in-c (Apache-2.0; re-derived)
for (auto it = std::prev(g_lru_order.end()); it != g_lru_order.begin(); --it) {
    if (cuEventQuery(g_lru_slot_events[dev][s]) == CUDA_SUCCESS) {
        evict = *it;  // Only evict COMPLETED DMA slots
        break;
    }
}
```

SYCL equivalent: use `sycl::event::get_info<info::event::command_execution_status>()`

#### 3b. Expert Pinning

Pin first N layers' experts in VRAM (always needed, never evicted):
```bash
VITRIOL_PIN_FIRST_N_LAYERS=5  # Pin layers 0-4
```

Uses ~220 MB per pinned layer. First 5 layers = ~1.1 GB.

#### 3c. Output Cache

Reuse previous token's expert outputs (approximate, ~2% quality loss):
- Most MoE routers produce similar expert selections for consecutive tokens
- Cache the last token's expert outputs, skip recomputation

### Phase 4: Upstream Sync

**19 commits behind upstream/master.** Key ones:

| Commit | Value |
|--------|-------|
| `cd8cdf397` | GGML_SYCL_MEMTRACE — per-site memory tracking |
| `4d9176092` | Kronecker product FWHT support |

**WARNING:** Upstream ggml sync (`64a155d24`) actively strips VITRIOL files.
Rebase must preserve:
- vitriol-sycl-buffer.cpp/hpp
- vitriol-sycl-integration.cpp
- dsv4_hc_*.comp shaders
- ggml-sycl.cpp hooks

### Phase 5: Vulkan VITRIOL Port (Longer Term)

Vulkan is 1.9x faster for MoE decode (31.9 vs 24.1 t/s on GLM).
Port approach:
1. Reuse host-side logic (LRU, predictor) from SYCL
2. Rewrite DMA for Vulkan memory model (vkCmdCopyBuffer + timeline semaphores)
3. Add VITRIOL buft type for Vulkan (host-visible, device-local via BAR)
4. Estimated: ~870 lines

## Benchmark Targets

| Metric | First Light | After Phase 1-3 | Target |
|--------|-------------|-----------------|--------|
| pp512 (cold) | 1.5 | **12.2** | 15-20 |
| pp512 (warm) | 8.6 | **14.4** | 20-25 |
| tg64 (warm) | 9.1 | **9.2** | 14-16 |

## Risk Assessment

1. **SYCL queue concurrency**: In-order queues serialize DMA + compute.
   Mitigation: predictor prefetch fires DMA for NEXT layer while CURRENT
   layer computes (cross-layer overlap, not within-layer).

2. **Memory pressure**: 74GB model + 4GB LRU + 2048 context KV.
   On 62GB RAM, this means ~16GB swap. Mitigation: reduce LRU to 2GB
   if OOM occurs.

3. **Upstream rebase complexity**: VITRIOL files are actively stripped.
   Mitigation: maintain a rebase script that re-applies VITRIOL patches.

## Files to Modify

- `vitriol-sycl-integration.cpp` — three-state eviction, expert pinning
- `vitriol-sycl-buffer.cpp` — pin_first_n_layers config
- `vitriol-sycl-buffer.hpp` — new declarations
- `ggml-sycl.cpp` — predictor_update calls, pinning hooks

## Status

- [x] Async DMA implemented + tested (+8.1x cold pp)
- [x] Predictive prefetch wired into hooks + tested
- [x] Three-state eviction ported from CUDA + tested (+18x long prompt pp)
- [x] LRU/ubatch/context tuned (optimal: 4GB LRU, ub=2048, c=2048)
- [x] Profile qwen80b configured for daily driver
- [ ] Port expert pinning (pin first N layers in VRAM)
- [ ] Port output cache (approximate reuse)
- [ ] Upstream sync (GGML_SYCL_MEMTRACE)
- [ ] Vulkan VITRIOL port
