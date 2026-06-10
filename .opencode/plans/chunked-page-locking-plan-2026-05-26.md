# Chunked Per-Expert Page Locking for VITRIOL
**Date:** 2026-05-26

## Problem
The Genesis model (Qwen3.6-35B-A3B-Uncensored-Genesis-MTP-APEX-Compact.gguf, 17 GB Q5_K_M) requires ~12.3 GB of page-locked host RAM with VITRIOL's current eager allocation. The system has 15 GB total RAM with ~11 GB free — insufficient. Other models (IQ2_M, IQ1_M) also lock 8.9-9.1 GB unnecessarily.

## Root Cause
`vitriol-buffer.cpp` allocates each expert tensor (e.g., `ffn_gate_exps.weight` with 256 experts) as **one contiguous page-locked block**:
```
mmap(MAP_ANONYMOUS | MAP_POPULATE, total_size)
mlock(ptr, total_size)         // all 256 experts locked
cudaHostRegister(ptr, total_size, 0)  // all 256 experts DMA-registered
```

Only 8 of 256 experts (~3%) are used per token, but all are pinned.

## Solution: Chunked Per-Expert Page Locking

### Concept
Split expert weight tensors into per-expert sub-buffers. Page-lock only the experts needed for the current token via a LRU. Limit total locked RAM to a configurable max (default 4 GB).

### Working Set
- 8 experts/token × 229 MiB/expert (Genesis Q5_K_M) = 1.83 GB
- With 2× lookahead (VITRIOL predictor): ~3.7 GB peak
- Default `VITRIOL_MAX_LOCKED_MB=4096` covers all models comfortably

### Memory Savings
| Model | Current (all locked) | With chunked (4 GB limit) |
|---|---|---|
| IQ2_M | 8.9 GB | ~3.7 GB |
| IQ1_M | 9.1 GB | ~3.5 GB |
| Genesis | 12.3 GB (OOM) | **~3.7 GB** |

### Files & Changes

**1. `vitriol-cuda-integration.h`** (+20 lines)
- Add `vitriol_ensure_expert_locked(tensor_base, expert_idx, expert_size)` declaration
- Add `vitriol_lazy_lock_active()` inline
- Add `max_locked_mb` field to `vitriol_config_t`

**2. `vitriol-cuda-integration.cpp`** (+150 lines)
- `locked_slot` struct: `tensor_base`, `expert_idx`, `size`, `in_use`
- `locked_lru`: hashmap `(tensor_base, expert_idx) → slot`, global total_locked_bytes
- `vitriol_ensure_expert_locked()`: lock a single expert slice
  - Already locked? → return
  - At limit? → evict LRU slot (cudaHostUnregister + munlock)
  - mlock(addr, size) + cudaHostRegister(addr, size, 0) on just nb02 bytes
  - Add to locked_lru, increment total
- `vitriol_init()`: read `VITRIOL_MAX_LOCKED_MB` env var (default 4096)
- atexit: unlock all active locked slots

**3. `vitriol-buffer.cpp`** (+30 lines)
- Add `vitriol_get_buffer_type_lazy()`: new buffer type variant
  - mmap(MAP_ANONYMOUS) without MAP_POPULATE, without mlock, without cudaHostRegister
  - Only used when `VITRIOL_MAX_LOCKED_MB` is set (activates lazy mode)
- Existing buffer type unchanged (default for small models)

**4. `vitriol-buffer.h`** (+5 lines)
- Add `vitriol_get_buffer_type_lazy()` declaration

**5. `ggml-cuda.cu`** (+15 lines)
- Fast-path selection (line 2556): add `&& !vitriol_lazy_lock_active()` to force per-expert loop when lazy mode active
- Per-expert loop (line 2694): before LRU check, add `vitriol_ensure_expert_locked()` call

### Data Flow
```
Allocation:
  vitriol_get_buffer_type_lazy() → mmap(ANONYMOUS, size, no POPULATE)
  → memcpy tensor data (pages fault in but NOT locked)

First use of expert i02:
  per-expert loop → vitriol_ensure_expert_locked(tensor, i02, nb02)
    → mlock(addr, nb02)       (~0.5ms)
    → cudaHostRegister(addr, nb02, 0)  (~0.3ms)
    → vitriol_lru_ensure() copies to VRAM via cuMemcpyHtoDAsync

Eviction (at max_locked_mb limit):
  → cudaHostUnregister(addr, nb02)
  → munlock(addr, nb02)
  → pages stay in mmap (swapable if needed)
```

### Total: ~220 lines across 5 files
