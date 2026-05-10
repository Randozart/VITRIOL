# VITRIOL v3: SSD-to-GPU Layer Streaming

**Date:** 2026-05-09  
**Goal:** Use SSD as external VRAM via llama.cpp + VITRIOL DMA integration  
**Hardware:** GTX 1070 Ti (8GB) + i7-3770 + NVMe SSD on /mnt/data

---

## Architecture

```
OpenCode → VITRIOL Inference Server (modified llama.cpp) → VITRIOL DMA Kernel Module → GPU VRAM
                                                              ↓
                                                         NVMe SSD (full model stored here)
```

---

## Phase 1: Setup and Baseline (Week 1)

1. Clone llama.cpp to `/mnt/data/ai/`
2. Build with CUDA, no AVX2
3. Verify baseline inference with Qwen 3.5 9B
4. Test VITRIOL kernel module in stub mode

---

## Phase 2: Page Manager (Week 1-2)

- Create `src/vitriol-page-manager.{h,cpp}` (~400 LOC)
  - LRU cache for GPU layers
  - Layer states: UNLOADED → LOADING → LOADED → EVICTED
  - Async prefetching
  - GPU memory budget management
- Modify `src/llama-model-loader.cpp` (~200 LOC)
  - Add on-demand loading mode
  - Register layer offsets, skip loading
  - Preload first N layers
- API extensions in `include/llama.h` (~50 LOC)
  - `on_demand_loading`, `gpu_memory_budget`, `preload_layers`
  - `llama_model_is_layer_loaded()`, `load_layer()`, `evict_layer()`

---

## Phase 3: VITRIOL DMA Integration (Week 2)

- Create `src/vitriol-dma.{h,cpp}` (~100 LOC)
  - Open `/dev/vitriol` character device
  - ioctl for DMA transfers
  - Poll for completion
- Integrate with page manager (~100 LOC)
  - `load_layer()` triggers VITRIOL DMA
  - Sliding window for layers > 256MB
- Update kernel module for ioctl interface

---

## Phase 4: Inference Loop (Week 3)

- Modify `src/llama-context.cpp` (~100 LOC)
  - Check layer loaded before computing
  - Blocking wait if loading
  - Prefetch next layer during compute
- Add prefetching (~50 LOC)

---

## Phase 5: Optimization and Testing (Week 3-4)

- Targets: <100ms layer load, >80% prefetch hit rate, 30+ tok/s, <7GB VRAM
- Unit tests, integration tests, stress tests, benchmarks

---

## Effort Estimate

| Component | New LOC | Modified LOC |
|-----------|---------|--------------|
| Page manager | 400 | 0 |
| VITRIOL DMA FFI | 100 | 0 |
| llama.cpp mods | 0 | 500 |
| API extensions | 50 | 0 |
| Kernel updates | 0 | 100 |
| Tests/examples | 200 | 0 |
| **TOTAL** | **~750** | **~600** |

**Timeline:** 3-4 weeks  
**Files affected:** ~14
