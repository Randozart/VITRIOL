# VITRIOL Cooperative Direct DMA

> **Date**: 2026-05-13
> **Status**: Phase 1 (of 4) — Planning
> **See also**: `VITRIOL_PROBE_SPEC.md`, `BIND_HANDOFF.md`, `ALKA_EXECUTOR_DESIGN.md`

## Architecture Decision

**Alka pipes data. CUDA computes.**

The GPU is idle 82-85% of inference time waiting for expert weights from system RAM (120ms per token). VITRIOL's core value is eliminating that wait by streaming directly from NVMe to VRAM.

Compute dispatch overhead (~0.05ms via CUDA) is negligible compared to that 120ms loading bottleneck. There is no benefit to replacing CUDA's compute submission with Alka firmware pokes — the complexity of per-generation GPU firmware protocols (Maxwell PUSH_BUFFER vs Pascal GPFIFO vs Ampere) outweighs the tiny latency gain.

```
┌──────────┐  FLOW (DMA)  ┌────────────────────┐  CUDA  ┌─────────┐
│  NVMe     ──────────────▶  GPU VRAM            ───────▶  Output  │
│  SSD      │  (nvidia P2P) │  (expert weights)   │        │  Tokens  │
└──────────┘               └────────────────────┘        └─────────┘
                              ▲
                              │ VITRIOL owns data movement
                              │ CUDA owns tensor compute
```

## The Pipeline

### Data Plane (VITRIOL/Alka — always on)
```
GGUF file → kernel_read() → staging buffer → nvidia_p2p pages → VRAM
```

1. `VITRIOL_IOC_SET_SOURCE` — userspace opens GGUF, passes fd to kernel
2. `FLOW drop` — `kernel_read()` from GGUF at `src_addr` into staging buffer
3. `nvidia_p2p_get_pages()` — get VRAM page physical addresses for target offset
4. DMA from staging buffer → VRAM physical pages (via `dma_map_resource` or `memcpy_toio` to P2P-mapped pages)
5. `nvidia_p2p_put_pages()` — release page references

### Compute Plane (CUDA — unmodified)
```
llama.cpp loads model → CUDA kernels → inference → output tokens
```

llama.cpp runs normally. The only change: when it needs expert weights, VITRIOL has already placed them in VRAM via the data plane. CUDA operates on the same VRAM addresses, unaware that data arrived over PCIe directly instead of through `cudaMemcpy`.

## Implementation Levels

| Level | Approach | When | Status |
|-------|----------|------|--------|
| **3** | Cooperative `nvidia_p2p_get_pages()` | nvidia driver present | Phase 1 (this sprint) |
| **2** | Kernel BIND IOCTL (`pci_stop_and_remove_bus_device`) | nvidia not needed (e.g. GTX 960 for testing) | Fallback |
| **1** | Userspace sysfs `vitriol-bind` | Quick testing, pre-Alka | Already exists in executor `--bind` |

We implement Level 3 as the primary path. Levels 1-2 are fallbacks when nvidia driver isn't in use.

## Phase Breakdown

### Phase 1: Symbol Resolution ✅ (Current)
- Use kprobe workaround to get `kallsyms_lookup_name`
- Resolve `nvidia_p2p_get_pages` and `nvidia_p2p_put_pages` at module init
- Graceful fallback if symbols not found (nvidia not loaded, or restricted kernel)

### Phase 2: P2P FLOW Handler
- New FLOW path that uses nvidia P2P pages instead of BAR1
- When `nvidia_p2p_get_pages()` succeeds, DMA data directly to VRAM
- When it fails (device busy, symbol not found), fall back to BAR1 or staging buffer

### Phase 3: Executor Integration
- Add `--cooperative` flag to executor (or auto-detect when `--source` is set and no `--bind`)
- Pass flag to kernel via `SET_VIAL` or a new IOCTL field
- Verified via `memcpy_fromio` readback

### Phase 4: Dynamic Expert Loading
- Only DMA the 8 active experts per token (not all 256)
- Parse GGUF expert index at load time
- Prefetch next layer's experts while computing current layer (double-buffering)

## VRAM Caching Strategy

```
┌─────────────────────────────────────────────┐
│  VRAM (8GB GTX 1070 Ti / 2GB GTX 960)      │
│                                             │
│  ┌─────────────┐  ┌──────────────────────┐  │
│  │ Reserved     │  │ Inference working set│  │
│  │ (256MB BAR1) │  │ (current layer       │  │
│  │              │  │  active experts,     │  │
│  │              │  │  KV cache, scratch)  │  │
│  └─────────────┘  └──────────────────────┘  │
│                                             │
│  ┌─────────────────────────────────────────┐│
│  │  Expert Cache (round-robin)             ││
│  │  Most recently used experts stay hot     ││
│  │  LRU eviction when full                  ││
│  └─────────────────────────────────────────┘│
└─────────────────────────────────────────────┘
```

For 35B MoE with 256 experts per layer, 8 active per token:
- Each expert ~40-80MB
- VRAM cache holds ~20-40 experts (2-3GB)
- DMA overhead: ~3-5ms per expert instead of 120ms full-model load
- LLM token frequency: ~70-140ms per token → prefetching can hide latency

## Files Changed

| File | Change |
|------|--------|
| `vitriol-daemon/vitriol.c` | Phase 1: kallsyms/kprobe + nvidia P2P resolution; Phase 2: P2P FLOW |
| `vitriol-daemon/vitriol_alka_kernel.h` | Possibly new cooperative DMA flag |
| `alka-executor/executor.c` | Phase 3: `--cooperative` flag |
| `alka-executor/vitriol_alka_user.h` | Mirror kernel header changes |

## Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| `kallsyms_lookup_name` not available | Phase 1 blocks | Use kprobe workaround (proven on kernels 5.7-6.x) |
| nvidia P2P symbols not exported | Level 3 impossible | Fall back to Levels 1-2 (bind-based DMA) |
| P2P DMA not supported on PCIe 3.0 | DMA to VRAM fails | Fall back to `memcpy_toio` via BAR1 (bind-based) |
| nvidia P2P API changes | Future breakage | Symbol versioning via `kallsyms_lookup_name` adapts automatically |
| GTX 960 VRAM fragmentation | Large allocations fail | Use multiple smaller P2P pages |
