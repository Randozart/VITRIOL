# VITRIOL Roadmap

> Current base: RAM Shot — page-locked host RAM for MoE expert weights (6.31 tok/s)

---

## Phase 3: CE DMA LRU Cache ✅ (Done)

**Goal**: Keep hot experts in a small VRAM pool → native VRAM speed on cache hit.
**Status**: ✅ Implemented — builds clean, not yet tested.
**Estimated gain**: +10–50% over RAM Shot (6.31 → ~7–9 tok/s)

### Architecture

```
Token → ids[] → {expert_7, expert_42}
                    │
                    ▼
         ┌──────────────────────┐
         │  LRU Cache Check     │
         │  (composite key:     │
         │   tensor_base + idx) │
         └──────┬───────────────┘
                │
        ┌───────┴───────┐
        ▼               ▼
    Cache HIT       Cache MISS
        │               │
        │        ┌──────┴────────────┐
        │        │ cuMemcpyHtoDAsync │
        │        │ on dedicated LRU  │
        │        │ stream            │
        │        │ cuEventRecord     │
        │        │ cuStreamWaitEvent │
        │        │ Update LRU order  │
        │        └──────┬────────────┘
        │               │
        └───────┬───────┘
                ▼
        ┌──────────────────┐
        │ Use VRAM pointer │
        │ for MUL_MAT_ID   │
        │ (no sync needed) │
        └──────────────────┘
```

### Implementation

| Component | Description |
|-----------|-------------|
| VRAM pool | 512 MB `cuMemAlloc` (configurable via `VITRIOL_LRU_MB`) |
| Cache key | `(tensor_base_address, expert_idx)` — prevents cross-layer collisions |
| DMA path | `cuMemcpyHtoDAsync` on dedicated `CUstream` |
| Sync | `cuEventRecord` on LRU stream → `cuStreamWaitEvent` on compute stream |
| Eviction | LRU list (std::list + std::unordered_map) |
| Slot resizing | Pool freed + reallocated if a tensor has larger experts |
| Fallback | Returns `0` → caller reads from host RAM |

### Hook (ggml-cuda.cu)

```cpp
if (vitriol_is_stream_enabled()) {
    CUdeviceptr vram_ptr = vitriol_lru_ensure(
        src0->data,              // tensor_base
        (int)i02,                // expert_idx
        (const void *)((char *) src0->data + i02*nb02),
        (size_t)nb02,
        stream                   // compute stream
    );
    if (vram_ptr != 0) {
        src0_slice.data = reinterpret_cast<char *>(vram_ptr);
    }
}
```

### Fast-path note

The fast paths (MMVQ/MMQ/MMF with `ids`) access `src0->data` directly rather than through per-expert slices. Replacing the data pointer there would require a reorganized VRAM buffer. Since fast paths handle small batches (≤8 tokens for MMVQ), PCIe DMA overhead is negligible. Skipped for now.

---

### Implementation Plan

1. **VRAM pool**: 512 MB `cuMemAlloc` in `vitriol_cuda_init()`
2. **LRU tracker**: `unordered_map<expert_key, vram_offset>`
3. **Hook in ggml_cuda_mul_mat_id**: Before expert loop, read `ids` tensor. For each active expert, check cache. On miss, CE DMA from host VITRIOL buffer → pool.
4. **Override `src0_slice.data`**: Point to VRAM pool offset instead of host pointer.
5. **Eviction**: LRU eviction when pool full.

### Files to modify

| File | Change |
|------|--------|
| `vitriol-cuda-integration.h` | Add `vitriol_lru_ensure()` declaration |
| `vitriol-cuda-integration.cpp` | LRU cache init, lookup, CE DMA load, eviction |
| `ggml-cuda.cu` | Call `vitriol_lru_ensure()` in `ggml_cuda_mul_mat_id` |

---

## Phase 4: Graph Split Optimization 🟡 Next

**Goal**: Reduce from 17 splits to ~2–5 by making VITRIOL buffer appear as CUDA-host to the scheduler.
**Status**: Investigated — requires `ggml-backend.cpp` scheduler refactoring.

### Root cause

When `is_host=true`, `ggml_backend_sched_backend_from_buffer` (ggml-backend.cpp:851) iterates backends by priority. VITRIOL's buffer type passes `supports_buft` for CUDA, but the scheduler's MoE offload path at line 1576 triggers on `is_host=true` and creates copy tensors for partial expert copies. The `need_new_split` condition at line 1282 fires on each backend mismatch, producing 17 splits.

### Proposed fix

Make VITRIOL buffer type use `ggml_backend_cuda_host_buffer_type_name` as its `get_name`, so `ggml_backend_buft_is_cuda_host()` returns true. Then modify `supports_buft` (ggml-cuda.cu:5271) to accept `cuda_host` buffers without the `integrated` GPU check. This would make the scheduler treat VITRIOL as a native CUDA buffer, eliminating the CPU→GPU backend mismatch.

### Files to modify

| File | Change |
|------|--------|
| `vitriol-buffer.cpp` | Match `get_name` to CUDA host buffer type name |
| `ggml-cuda.cu` | Remove `integrated` guard for `cuda_host` in `supports_buft` |
| (no ggml-backend.cpp changes) | Scheduler already handles `cuda_host` natively |

**Estimated gain**: -3–10% latency (eliminated copy overhead).

---

## Phase 5: io_uring + O_DIRECT (Future)

**Goal**: Remove mmap memory pressure by reading expert data directly from NVMe into pre-pinned buffers. Frees page cache.
**Estimated gain**: -10 GB system RAM (page cache), no perf change.

### Approach

- Open GGUF with `O_DIRECT`
- Use `io_uring` to read expert slices into bounce buffers
- CE DMA from bounce → VRAM cache (or keep in pin buffer for RAM Shot)

---

## Phase 6: Dual-GPU Speculative Decoding (Future)

**Goal**: GTX 960 (2 GB) as draft model, 1070 Ti as target.
**Estimated gain**: +50–100% tokens/s (speculative decoding speedup).

### Approach

- GTX 960 runs small draft model (e.g., Qwen2.5-1.5B)
- 1070 Ti runs Qwen3.6-35B-A3B target
- CE DMA streams expert data between GPUs for cross-validation

---

## Phase 7: Alka Orchestration (Future)

**Goal**: High-level stream language for expert loading patterns.

### Approach

- Define Alka recipes for expert fetch patterns
- Compile to CE DMA + FENCE operations
- Coordinate multiple GPUs

---

## Milestone Timeline

| Phase | Description | Est. Duration | Status |
|-------|-------------|---------------|--------|
| **3** | CE DMA LRU Cache | 1–2 sessions | ✅ Done |
| 4 | Graph Split Optimization | 1 session | 🟡 Next |
| 5 | io_uring + O_DIRECT | 2–3 sessions | 🟢 Low |
| 6 | Dual-GPU Spec Decode | 3–5 sessions | 🟢 Low |
| 7 | Alka Orchestration | 5+ sessions | 🟢 Low |

## Current Session: Phase 4 — Graph Split Optimization 🟡 Next

**Goal**: Reduce from 17 splits to ~2–5.**

*Last updated: 2026-05-16 16:00 CEST*
