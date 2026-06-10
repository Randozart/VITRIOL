# Pascal DMA LRU Fix: Per-Slot CUDA Events

**Date:** 2026-06-10 13:05  
**Author:** opencode  
**Status:** Plan → Implementation

## Problem

On Pascal GPUs (GTX 1070 Ti, CC 6.1), VITRIOL's LRU expert weight swap corrupts
data due to a slot-overwrite race:

1. Expert A's matmul launches on `compute_stream` → reading slot X (in-flight)
2. Expert B (next iteration of the loop) evicts slot X on `g_lru_stream`
3. `cuMemcpyHtoDAsync` starts WRITING new data to slot X
4. Expert A's matmul is STILL reading slot X → mixed old/new data = corruption
5. `cuStreamWaitEvent(cstream, g_lru_event, 0)` is too late — the DMA write has
   already overlapped with the in-flight compute read

On Volta+ (CC 7.0+), independent thread scheduling + guaranteed cross-engine
L2 coherence prevents this. On Pascal, the copy engine and compute SMs are
fully independent with no automatic fencing.

## Root Cause

A **single global `g_lru_event`** shared across all LRU slots. There's no way
to wait for "compute on slot X is done" before overwriting slot X, because
the global event doesn't carry per-slot provenance.

## Fix: Per-Slot CUDA Events

Replace `g_lru_event` with an array of events — one per LRU slot. Each event
tracks the last operation (DMA or compute) that touched that slot.

### Flow

```
1. Cache miss → DMA to slot S on lru_stream
2. cuEventRecord(slot_events[S], lru_stream)     ← "DMA for slot S done"
3. cuStreamWaitEvent(cstream, slot_events[S], 0)  ← compute waits for DMA
4. ggml_cuda_mul_mat(...) on cstream               ← compute reads slot S
5. vitriol_lru_mark_compute_done(ptr, cstream)     ← "compute for slot S done"
6. Cache miss (eviction of slot S) →
   cuStreamWaitEvent(lru_stream, slot_events[S], 0) ← DMA waits for compute
7. cuMemcpyHtoDAsync(dst, new_data, size, lru_stream) ← safe to overwrite
```

### Files Changed

| File | Change |
|------|--------|
| `vitriol-cuda-integration.h` | Add `vitriol_lru_mark_compute_done()` declaration |
| `vitriol-cuda-integration.cpp` | Replace `g_lru_event` with `g_lru_slot_events[]` array |
| `ggml-cuda.cu` | Call `vitriol_lru_mark_compute_done()` after each matmul |

### Detailed Changes

#### vitriol-cuda-integration.cpp

1. **Line 62**: Remove `static CUevent g_lru_event = 0;`
2. **Add after line 38**: `static CUevent *g_lru_slot_events = nullptr;`
3. **`lru_ensure_stream()` (line 723)**: Remove `cuEventCreate(&g_lru_event, ...)`
4. **`lru_init_pool()` (after line 763)**: Allocate per-slot events:
   ```cpp
   g_lru_slot_events = new CUevent[g_lru_num_slots];
   for (int i = 0; i < g_lru_num_slots; i++)
       cuEventCreate(&g_lru_slot_events[i], CU_EVENT_DISABLE_TIMING);
   ```
5. **`vitriol_lru_ensure()` — cache hit (line 800)**: Wait for slot's event:
   ```cpp
   cuStreamWaitEvent(cstream, g_lru_slot_events[it->second], 0);
   ```
6. **`vitriol_lru_ensure()` — cache miss (before line 829)**: Wait for slot:
   ```cpp
   if (/* slot was evicted, not fresh */) {
       cuStreamWaitEvent(g_lru_stream, g_lru_slot_events[slot], 0);
   }
   ```
7. **`vitriol_lru_ensure()` — after DMA (replace lines 832-835)**: Record slot event:
   ```cpp
   cuEventRecord(g_lru_slot_events[slot], g_lru_stream);
   ```
   Note: `cuStreamWaitEvent(cstream, ...)` is no longer needed here because
   the cache-miss return goes through the same path as prefetch—the compute
   stream wait happens at cache-hit time, or we can keep it for immediate
   compute. Actually, KEEP the wait on cstream for correctness on first access.
   
8. **`vitriol_lru_prefetch_async()` (line 891)**: Record slot event:
   ```cpp
   cuEventRecord(g_lru_slot_events[slot], g_lru_stream);
   ```
9. **`vitriol_cuda_cleanup_vram()`**: Free per-slot events before freeing pool

10. **New function** at end of file:
    ```cpp
    void vitriol_lru_mark_compute_done(CUdeviceptr vram_ptr, CUstream stream) {
        if (!vram_ptr || !g_lru_slot_events) return;
        int slot = (int)((vram_ptr - g_lru_pool) / g_lru_slot_size);
        if (slot >= 0 && slot < g_lru_num_slots) {
            cuEventRecord(g_lru_slot_events[slot], stream);
        }
    }
    ```

#### vitriol-cuda-integration.h

Add after the `vitriol_lru_ensure` declaration (after line 92):
```cpp
void vitriol_lru_mark_compute_done(CUdeviceptr vram_ptr, CUstream stream);
```

#### ggml-cuda.cu

After `vitriol_lru_ensure()` returns and after the matmul call (after line 2766
or wherever the matmul is dispatched), call:
```cpp
if (vram_ptr != 0) {
    vitriol_lru_mark_compute_done(vram_ptr, stream);
}
```

This is needed in TWO places:
1. The regular matmul path (after `ggml_cuda_mul_mat`)
2. The output-cache hit path (skips matmul but still "uses" the slot)

## Testing

1. Build: `cmake --build .`
2. Start server with VITRIOL enabled, MXFP4_MOE model
3. Test simple generation: "Say hello"
4. Test with tool calls in history
5. Verify no `</tool_call>` loop
6. Check VITRIOL stats: LRU hit rate > 0 (confirms cache is being used)

## Rollback

If the fix introduces regressions, set `VITRIOL_PIN_FIRST_N_LAYERS=28` to
bypass LRU entirely (use the safe pinning path) while debugging.
