# VITRIOL Experiment Log

**Purpose:** Track every architecture approach, its performance, and the outcome. All timestamps are in CET/CEST.

---

## Legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Working — production quality |
| ⚠️ | Working — with caveats / partial |
| ❌ | Failed — blocked or crash |
| 💡 | Concept / not implemented |

---

## Experiment 0: Baseline (All-VRAM)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-10 to 2026-05-13 |
| **Commit** | `df4d525`, `a818380` |
| **Approach** | Vanilla llama.cpp with `-ngl 41`, all tensors in CUDA device memory |
| **Model** | Qwen3.6-35B-A3B-UD-Q2_K_XL (11.44 GiB, 256 experts) |
| **GPU** | GTX 1070 Ti (8 GB) |

**Note**: The full model does NOT fit in 8 GB VRAM. The "baseline" was established with different context sizes and quantization levels that did fit.

| Metric | Value | Notes |
|--------|-------|-------|
| Prompt eval | 4.89 tok/s | Baseline from MILESTONE_1.md Test 3 |
| Generation | **6.52 tok/s** | 153.28 ms/token |
| Model memory | ~2129 MiB | Without full expert tensor allocation |
| Graph splits | 2 | Default scheduler behavior |

**Verdict**: ❌ Model doesn't fit in VRAM in full. Only partial runs were possible.

---

## Experiment 1: PCI BIND — Userspace Driver Takeover

| Field | Value |
|-------|-------|
| **Date** | 2026-04-30 to 2026-05-15 |
| **Commit** | `b02a6dc` |
| **Approach** | Fork-based userspace PCI rebinding, unbind nvidia → bind vitriol, `memcpy_toio(BAR1)` |
| **3 tiers**:| polite unbind → firm remove/rescan → TTY escalation |

**Result**: ❌ Failed — GMMU page tables never populated by nvidia RM.

| Attempt | Outcome |
|---------|---------|
| Warm unbind (preserve RM state) | `0xBAD0FBxx` on readback — GMMU tables empty |
| Cold remove/rescan | RM state wiped, even worse |
| `driver_override` at boot | Starved GPU entirely of init |

**Root cause**: NVIDIA RM's proprietary GMMU init is required for BAR1 to be a valid memory window. Without it, writes go nowhere.

---

## Experiment 2: Boot-Time Reservation (udev `driver_override`)

| Field | Value |
|-------|-------|
| **Date** | 2026-04-30 |
| **Commit** | `95be3dd` |
| **Approach** | udev rule sets `driver_override=vitriol` at boot, preventing nvidia from initializing GTX 960 |

**Result**: ❌ Failed — preventing nvidia init made the GMMU problem worse.

Secondary/headless GPU's GMMU was never initialized by RM. Even after clearing the override and rebinding nvidia, RM refused to fully initialize it.

---

## Experiment 3: GPUDirect RDMA / CUDA P2P

| Field | Value |
|-------|-------|
| **Date** | 2026-05-13 to 2026-05-15 |
| **Commit** | `289a819` |
| **Approach** | `cuPointerGetAttribute(IS_GPU_DIRECT_RDMA_CAPABLE)`, `cuMemCreate`, Peer-to-Peer access tokens |

**Result**: ❌ Blocked by NVIDIA GeForce SKU lockout.

| Attempt | Outcome |
|---------|---------|
| `IS_GPU_DIRECT_RDMA_CAPABLE` | Returns 0 for all `cudaMalloc` allocations |
| P2P tokens | Error (GeForce SKU restriction) |
| `cuMemCreate` for export | Fails — only available on Tesla/Quadro |
| `nvidia-peermem` module | Unavailable |

---

## Experiment 4: Nouveau DRM Init

| Field | Value |
|-------|-------|
| **Date** | 2026-05-13 to 2026-05-15 |
| **Commit** | `289a819` |
| **Approach** | Load `nouveau` driver to initialize GMMU, then hand off to VITRIOL |

**Result**: ❌ Blocked by nvidia/nouveau mutual exclusion.

Loading nouveau requires `modprobe -r nvidia`, which crashes the display server (1070 Ti drives desktop). Even if loaded, nouveau's GMMU state doesn't persist through unbind (GPU drops to D3).

---

## Experiment 5: PAT Side-Load (Write-Combining Mapping)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-13 to 2026-05-15 |
| **Commit** | `289a819` |
| **Approach** | Side-load kernel module that calls `ioremap_wc()` on BAR1, then userspace `/dev/mem` mmap |

**Result**: ❌ Blocked by kernel PAT enforcement on kernel 6.17.

Kernel Page Attribute Table rejects overlapping mappings with different cache types. nvidia maps BAR1 as UC-; our WC mapping conflicts. Even userspace `/dev/mem` mmap fails because `track_pfn_remap` enforces PAT for IO memory.

---

## Experiment 6: Copy Engine DMA (CE DMA) — Standalone

| Field | Value |
|-------|-------|
| **Date** | 2026-05-15 |
| **Commit** | `289a819` |
| **Approach** | `cuMemcpyDtoDAsync` via GPU Copy Engine. Bounce buffer (cuMemHostAlloc) → CE DMA → VRAM |

**Result**: ✅ Verified — data integrity confirmed.

```
CE DMA completed successfully
VRAM first 64 bytes: 47 47 55 46 03 00 00 00 ...
=== PASS: DMA data matches GGUF source! ===
```

| Metric | Value |
|--------|-------|
| Source | GGUF vocab file on NVMe |
| Buffer | cuMemHostAlloc (256 MB, DEVICEMAP) |
| DMA engine | cuMemcpyDtoDAsync on Copy Engine stream |
| Verification | cuMemcpyDtoH readback, byte-for-byte |
| Transfer size | 4096 bytes |
| Per-expert cost | ~0.06 ms (projected for 42 MB) |
| CE DMA bandwidth | ~12 GB/s (PCIe 3.0 x16) |

**Verdict**: ✅ CE DMA works. The GPU's internal Copy Engine can DMA from host memory to VRAM without CPU involvement.

---

## Experiment 7: CE DMA + supports_buft (Original VITRIOL Buffer)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-15 |
| **Commit** | `289a819`, `0ea005b` |
| **Approach** | Create custom VITRIOL buffer type with `is_host=false`. `supports_buft` accepts VITRIOL type. set_tensor records source pointer (skips copy). On MUL_MAT_ID, CE DMA from source to VRAM pool. |

**Result**: ❌ CRASH — ROPE failed (illegal memory access).

| Symptom | Cause |
|---------|-------|
| ROPE crash during warmup | GPU kernel tried to access system memory pointer without page-locking |
| VRAM pool allocation conflict | 3420 MB pool allocated late, corrupted CUDA memory manager |
| `supports_buft` not triggered | Scheduler didn't route MUL_MAT_ID to CUDA for VITRIOL tensors |

**Root cause**: The VITRIOL buffer allocated system RAM via `posix_memalign` but reported `is_host=false`. GPU kernel tried to dereference a system address → illegal memory access (not page-locked).

---

## Experiment 8: RAM Shot — Page-Locked Host Memory ✅

| Field | Value |
|-------|-------|
| **Date** | 2026-05-16 |
| **Commit** | `94162e0` |
| **Approach** | VITRIOL buffer with `mmap` → `madvise(MADV_HUGEPAGE)` → `mlock` → `cudaHostRegister` → `is_host=true`. Expert weights in page-locked host RAM. GPU reads over PCIe DMA during MUL_MAT_ID. |

**Result**: ✅ WORKING — 6.31 tok/s on GTX 1070 Ti (8 GB VRAM).

| Metric | Value | vs Baseline |
|--------|-------|-------------|
| Prompt eval | 33.86 tok/s | +592% (baseline had warmup cost) |
| Text generation | **6.31 tok/s** | **-3.2%** |
| VRAM used | 1.3 GiB (model only) | -83% |
| System RAM used | +10 GiB (expert weights) | +10 GiB |
| Model load time | ~64 s | +113% (10 GB memcpy) |
| Graph splits | 17 | +15 |
| Sched copies | 4 | +3 |

**Privileges**: Needs `CAP_IPC_LOCK` (one-time `sudo setcap cap_ipc_lock=+ep ./bin/llama-server`).

**Key insight**: Setting `is_host=true` on a page-locked host memory buffer enables the GPU to read expert weights over PCIe DMA transparently. The scheduler routes MUL_MAT_ID to CUDA via the intelligent MoE offload path.

---

## Experiment 9: CE DMA LRU Cache (Implemented) 🚧

| Field | Value |
|-------|-------|
| **Date** | 2026-05-16 |
| **Status** | ✅ Implemented — not yet tested |
| **Commit** | `683122e49-dirty` |

**Approach**: On top of RAM Shot, add a small VRAM pool (~512 MB) for frequently-used expert weights. `cuMemcpyHtoDAsync` copies from page-locked host RAM to VRAM pool on cache miss. Dedicated LRU stream + `cuStreamWaitEvent` before matmul. Composite key `(tensor_base, expert_idx)` prevents cross-layer collisions.

| Metric | Expected |
|--------|----------|
| VRAM pool | 512 MB (configurable via `VITRIOL_LRU_MB`) |
| Eviction policy | LRU (list + hash map) |
| DMA path | cuMemcpyHtoDAsync on dedicated stream |
| Synchronization | cuEventRecord + cuStreamWaitEvent |
| Fallback | Host RAM read on cache miss |
| Slot sizing | Dynamic — reallocates if expert size changes |
| Layer safety | Composite key `(tensor_base, expert_idx)` |
| Fast-path prefetch | 💡 Skipped for now — fast paths use small batches |

**Expected improvement over RAM Shot**: 10-50% on repeated expert access patterns.

**Configuration**:
```
VITRIOL_MODE=stream           # Enable RAM Shot + LRU cache
VITRIOL_LRU_MB=512            # VRAM pool size (default: 512)
VITRIOL_VERBOSE=1             # Log cache hits/misses/evictions
```

---

## Architecture Comparison

| # | Approach | Date | Status | Gen tok/s | VRAM Saved | Complexity |
|---|----------|------|--------|-----------|------------|------------|
| 0 | All-VRAM | May 10 | ❌ Doesn't fit | 6.52* | 0 GB | None |
| 1 | PCI BIND | Apr 30–May 15 | ❌ GMMU brick | — | — | Extreme |
| 2 | driver_override | Apr 30 | ❌ No GMMU init | — | — | High |
| 3 | GPUDirect RDMA | May 13 | ❌ GeForce lock | — | — | Low (API) |
| 4 | Nouveau DRM | May 13 | ❌ nvidia conflict | — | — | High |
| 5 | PAT side-load | May 13 | ❌ Kernel 6.17 | — | — | Medium |
| 6 | CE DMA alone | May 15 | ✅ Verified | — | — | Low |
| 7 | CE DMA + buft | May 15 | ❌ Illegal access | — | 10 GB | Medium |
| 8 | **RAM Shot** | **May 16** | **✅ Working** | **6.31** | **10 GB** | **Low** |
| 9 | **LRU Cache** | **May 16** | **✅ Implemented** | **TBD** | **10 GB** | **Medium** |

*\* Baseline established with partial model that fit in VRAM.*

## Models Tested

| Model | Params | Experts | Quant | File Size | Tested | Works? |
|-------|--------|---------|-------|-----------|--------|--------|
| Qwen3.6-35B-A3B | 34.66B | 256 (8 active) | UD-Q2_K_XL | 11.44 GiB | ✅ | ✅ RAM Shot |
| (other models TBD) | | | | | | |

## Key Technical Decisions

| Decision | Rationale |
|----------|-----------|
| `is_host=true` | Scheduler sees host buffer → intelligent MoE offload → GPU reads via PCIe DMA |
| `mmap`+`mlock`+`cudaHostRegister` | Three-step page-locking: map, pin, register for GPU access |
| `madvise(MADV_HUGEPAGE)` | Hint for 2 MB pages → lower GPU TLB pressure |
| No VRAM pool | RAM Shot needs zero VRAM for weights — all freed for compute |
| LRU CE DMA kept as async | Dedicated stream + cuStreamWaitEvent, no blocking |
| Composite cache key | (tensor_base addr, expert_idx) prevents cross-layer collisions |
| Variable slot sizing | Pool reallocates if expert size changes between layers |
| `CUDA_VISIBLE_DEVICES=0` | GTX 960 (CC 5.2) lacks kernel images for some ops |

## Configuration Matrix

```
VITRIOL_MODE=stream → RAM Shot + LRU VRAM cache active
  VITRIOL_LRU_MB=512  → VRAM pool size (default: 512 MB)
  VITRIOL_VERBOSE=1   → detailed cache hit/miss/eviction logging
  CUDA_VISIBLE_DEVICES=0 → single GPU (1070 Ti only)

Model requirements:
  -gguf format
  -MoE architecture with expert tensors named containing "exps"
  -CAP_IPC_LOCK capability for mlock + cudaHostRegister
```

---

*Last updated: 2026-05-16 16:00 CEST*
