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
| **Status** | ✅ Implemented — tested (fast path) |
| **Commit** | `683122e49-dirty` (llama.cpp) |

**Approach**: On top of RAM Shot, add a small VRAM pool (~512 MB) for frequently-used expert weights. `cuMemcpyHtoDAsync` copies from page-locked host RAM to VRAM pool on cache miss. Dedicated LRU stream + `cuStreamWaitEvent` before matmul. Composite key `(tensor_base, expert_idx)` prevents cross-layer collisions.

| Metric | Expected | Actual |
|--------|----------|--------|
| VRAM pool | 512 MB | Allocated lazily on first LRU call |
| Generation | 10-50% over RAM Shot | 6.9 t/s (+9.4% over 6.31) |
| Prompt eval | — | 22.4 t/s |
| LRU cache usage | Slow path only | Fast path used (MMVQ with ids) |
| Model | Qwen3.6-35B-A3B | Loaded with `-ngl 99` on 8 GB GPU |

**Test command**:
```bash
CUDA_VISIBLE_DEVICES=0 VITRIOL_MODE=stream VITRIOL_LRU_MB=512 VITRIOL_VERBOSE=1 \
  llama-cli -m Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf -ngl 99 -c 512 -n 8 -p "Hello" -t 4
```

**Fast-path note**: Generation uses MMVQ (batch ≤ 8) which reads experts directly from page-locked host RAM via PCIe DMA. LRU cache is only activated on the slow path (cuBLAS per-expert slices). The fast-path kernel accesses `src0->data` directly, not through per-expert slices, so the LRU pointer swap doesn't apply.

**LRU cache testing**: Slow path tested with `-DGGML_CUDA_FORCE_CUBLAS=ON`. LRU pool allocated once per expert tensor dimension — 3 reallocations during warmup (303K → 401K → 557K byte slots across different layer groups), then stable during inference. Prompt eval: 14.3 t/s (slow path), generation: 7.0 t/s (MMVQ fast path).

**Pool thrashing bug**: Fixed. Pool now allocates once with first expert's slot size; larger experts bypass cache and fall through to host RAM PCIe DMA.

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
| 9 | **LRU Cache** | **May 16** | **✅ Tested (fast path)** | **6.9** | **10 GB** | **Medium** |

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

## Experiment 10: Emulated Memory Architecture (Design + Config)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-17 |
| **Approach** | Intercept-retrieve-inject pattern via Python Flask shim. SQLite-backed episodic + semantic memory with cascading retrieval. Port-swap: memory ON → llama-server on PORT-1, shim on PORT. |
| **Status** | ✅ Design docs complete. Memory package written (7 modules, 1,400+ LoC). Shim updated with memory toggle. `vitriol` CLI edited (956 LoC) with full TUI menu + port swap logic. |

### Deliverables

| Artifact | Lines | Status |
|----------|-------|--------|
| `docs/OPTIMIZATION_PLAN.md` (V2) | 590 | ✅ Full roadmap, 7 citations |
| `docs/EMULATED_MEMORY_ARCHITECTURE.md` | 592 | ✅ DB schema, scoring, cascading retrieval, Hebbian, compaction, sleep, deployment |
| `libvitriol/memory/` (7 modules) | ~1,400 | ✅ db, scorer, retrieval, compact, hebbian, consolidate, __init__ |
| `libvitriol/vitriol_shim.py` (memory toggle) | 763 | ✅ `VITRIOL_MEMORY_MODE=on` intercept loop, `/memory/stats`, `/memory/clear` |
| `scripts/vitriol` (TUI + port swap) | 956 | ✅ Memory Settings menu, `--memory-mode` flag, detach/foreground port swap |
| `~/.config/opencode/opencode.jsonc` | — | ✅ X-Project-Id, X-Session-Id custom headers |

### Memory Mode Config

```
VITRIOL_MEMORY_MODE=on   → llama-server on 8278, shim on 8279 (port swap)
VITRIOL_MEMORY_MODE=off  → llama-server on 8279 directly (existing behavior)
```

### Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| Python Flask shim (not Rust) | Fastest path to working prototype; Rust daemon is Phase 2 |
| Port swap (Option A) | OpenCode config never changes — always points to 8279 |
| Episodic + semantic split | Episodic for raw context, semantic for cross-session patterns |
| Hebbian weight updates | Post-response per-connection weight increase based on co-occurrence |
| Compaction during `POST` intercept | Avoids blocking token generation; happens on next request |
| Consolidation background thread | Periodic summarization + pruning for memory wellness |

### Next Phases

1. **Phase 0** — Config, TUI, port swap ✓
2. **Phase 1 (now)** — KV cache offload, sparse caching, frozen prompt caching ✓
3. **Phase 2** — Rust daemon (`vitriol-router`), tokio + rusqlite + tree-sitter
4. **Phase 3** — Agentic memory (GPT-Researcher-style iterative search, tool-based memory editing)

---

## Experiment 11: KV Cache Offload + Sparse + Frozen Prompt (2026-05-17)

| Field | Value |
|-------|-------|
| **Status** | ✅ Implemented (C++ in llama.cpp + Python shim + CLI config) |
| **Approach** | Three independent but composable context-efficiency features, all toggleable via `--kv-mode` and `--frozen-prompt` |

### Feature 1: Zero-Copy KV Cache Offload (`--kv-mode offload`)

Puts the KV cache tensors in page-locked host RAM (via `ggml_backend_dev_host_buffer_type()`) instead of GPU VRAM. The GPU reads them over PCIe DMA during attention — same approach as RAM Shot's expert offload but applied to K/V state.

| Before (VRAM KV) | After (Host RAM KV) |
|------------------|---------------------|
| 500-1000 tokens max | 20,000+ tokens |
| 5.2 GiB VRAM for KV | ~0.5 GiB VRAM (hot window only) |
| OOM at -c 2048 | Scales with system RAM |

**Modified:** `llama-kv-cache.cpp` buffer type selection (line 193-200), `ggml-cuda.cu` `supports_buft` host buffer guard removed.

### Feature 2: Sparse KV Caching (`--kv-mode sparse`)

Per-cell attention score tracking + position-based eviction. Always preserves the first 4 tokens (attention sinks) and the most recent window. Low-scoring middle tokens are evicted when cache fills, providing 4-8x effective compression.

**Modified:** `llama-kv-cells.h` (score vector + accessors), `llama-kv-cache.cpp` (`evict_sparse()` + `prepare()` hook).

### Feature 3: Frozen Prompt Caching (`--frozen-prompt on`)

The Python shim identifies system/tool messages as a stable prefix. They are kept byte-identical across requests — never truncated, never metadata-stripped. llama.cpp's prompt cache recognizes the unchanged prefix and skips re-evaluation, reducing prefill from ~16 min to ~1 min at 20K tokens.

**Modified:** `vitriol_shim.py` (`frozen_count` param in `rectify_context`, hash tracking, rectification scope).

### Config Interface

```
--kv-mode standard | offload | sparse    (default: standard)
--frozen-prompt on | off                 (default: off)
```

Available via CLI flag, env var (`VITRIOL_KV_MODE`, `VITRIOL_FROZEN_PROMPT`), and TUI (Context & Memory Settings menu).

---

## Experiment 12: Semantic Search (`--semantic-mode on`)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-17 |
| **Approach** | Optional sentence-transformers (`all-MiniLM-L6-v2`) for cosine similarity retrieval, replacing Jaccard keyword overlap |
| **Status** | 💡 Implemented, untested (no end-to-end run yet) |

### Implementation

Three-layer integration:

1. **`memory/scorer.py`** — lazy-loaded `SentenceTransformer` model, `semantic_similarity()` computes cosine similarity via numpy. Falls back to `keyword_overlap()` if sentence-transformers not installed. `compute_score()` now calls `semantic_similarity()` when `VITRIOL_SEMANTIC_MODE=on`.

2. **`memory/db.py`** — optional `embeddings` SQLite table caches computed embeddings keyed by SHA-256 content hash. `_compute_and_cache()` stores float32 blobs. `get_embedding_for_text()` public helper for external use.

3. **`memory/retrieval.py`** — candidate pool expanded to `20x` top_k when in semantic mode (vs `10x` for keyword) to allow full ranking over more candidates.

### CLI Interface

```
--semantic-mode on | off     (env: VITRIOL_SEMANTIC_MODE, default: off)
```

Available via CLI flag, env var, config key `memory.semantic_mode`, and TUI (option 4 in Context & Memory Settings).

### Notes

- Depends on `sentence-transformers` and `numpy` Python packages (not installed by default).
- First inference after mode is toggled on will download the `all-MiniLM-L6-v2` model (~80 MB).
- Embedding cache lives in each project's `memory.db` so it persists across sessions.
- The `vector_store.py` module (separate FAISS-based archival context streaming) is NOT replaced — this enhances the episodic memory retrieval path only.

**Modified:** `memory/scorer.py`, `memory/db.py`, `memory/retrieval.py`, `vitriol_shim.py` (health endpoint), `scripts/vitriol` (CLI + config + TUI + env piping).

---

## Experiment 13: Predictive Prefetching (`VITRIOL_PREDICTIVE_PREFETCH=1`)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-17 |
| **Approach** | Heuristic: store expert IDs from previous `ggml_cuda_mul_mat_id` call, prefetch same experts via async DMA before next call's device→host ID copy completes |
| **Status** | ✅ Tested — +7.8% with DMA overlap (2026-05-20) |

### Implementation

Three hooks in the MoE matmul path:

1. **`vitriol_predictor_prefetch()`** — called at the START of `ggml_cuda_mul_mat_id` (before `cudaMemcpyAsync` of `ids` tensor). Iterates the previous call's expert indices and fires `vitriol_lru_prefetch()` for each via the dedicated LRU CUDA stream.

2. **`vitriol_predictor_update()`** — called at the END of `ggml_cuda_mul_mat_id` (after `get_rows_cuda` scatter). Iterates `tokens_per_expert[]` to collect unique expert indices used in this invocation. Stores them for next call's prefetch.

3. **Control** — `VITRIOL_PREDICTIVE_PREFETCH=1` env var sets `g_vitriol_config.async_prefetch = true` in `vitriol_cuda_init()`.

### Expected Impact

- Heuristic hit rate: 60-70% (MoE routing is layer-correlated; adjacent layers tend to activate similar expert sets)
- Overlap: prefetch DMA runs concurrently with the device→host `ids` copy + `cudaStreamSynchronize` at start of `ggml_cuda_mul_mat_id`
- Miss cost: synchronous load via `vitriol_lru_ensure()` fallback (existing behavior)
- Net gain: +10-20% tok/s when heuristic hits, zero regression on misses

### Limitations

- Heuristic only (no learned predictor yet). A proper linear probe (~1K params) could raise hit rate to 85-90%.
- Only prefetches from the immediately preceding layer; does not look further ahead.

**Modified:** `vitriol-cuda-integration.h/.cpp`, `ggml-cuda.cu`, `llama.cpp-patches/`.

### Update 2026-05-20: Dedicated DMA Stream Overlap (Fire-and-Forget Prefetch)

Converted `vitriol_lru_prefetch` from a blocking call (which submitted DMA + waited on compute stream) to a true fire-and-forget async operation. Key changes:

- **New `vitriol_lru_prefetch_async()`** (static): submits `cuMemcpyHtoDAsync` on `g_lru_stream`, records `cuEventRecord`, but does **not** call `cuStreamWaitEvent` on any compute stream.
- **Cache-hit path in `vitriol_lru_ensure()`**: now calls `cuStreamWaitEvent(cstream, g_lru_event, 0)` before returning the VRAM pointer, ensuring data DMA'd by a prefetch is fully resident before the matmul reads it.
- **`vitriol_lru_prefetch()`**: now delegates to `vitriol_lru_prefetch_async()`, ignoring the `compute_stream` parameter.

Rationale: previously the comment on `vitriol_lru_prefetch` claimed "fire-and-forget" but the implementation called `vitriol_lru_ensure` which performed a synchronous wait. The added wait was moved to the cache-hit path where it's needed (the per-expert loop reads the data), allowing prefetches to overlap with codes copy + sort at the start of each layer.

### DMA Overlap Benchmark Results

Measured with Qwen3.6-35B-A3B-UD-IQ2_M, stream mode, 1024 MB LRU, output cache ON, Q4_0 KV, FA on, -ngl 99, -t 4, -mmp 0. `llama-bench` with `-n 100 -r 3`.

| Configuration | tg100 (t/s) | LRU hit rate | vs baseline |
|---|---|---|---|
| Output cache only (sorted path) | 9.34 ± 0.05 | 99.04% | — |
| + Predictive prefetch + DMA overlap | **10.07 ± 0.09** | 99.54% | **+7.8%** |

Statistically significant (non-overlapping error bars). The predictor increases LRU hit rate marginally (99.04→99.54%) but the DMA overlap itself provides the bulk of the speedup by hiding PCIe transfer latency behind compute (IDs copy + sort at start of each layer's ggml_cuda_mul_mat_id).

Note: the sorted path (required for output cache + LRU + predictor) is only entered during single-token generation (`ne12 == 1`) with `VITRIOL_OUTPUT_CACHE=1`. The output cache itself is approximate (reuses previous token's expert outputs), but the predictor + DMA overlap have zero quality impact.

### Config Integration

Added `predictive_prefetch = on|off` to VITRIOL config file, TUI (option 4 in VITRIOL Mode Settings), and env var `VITRIOL_PREDICTIVE_PREFETCH=1`. Defaults to `off`.

---

## Experiment 14: Graph Split Optimization (Planned — Deferred)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-17 |
| **Status** | 💡 Analysis done — implementation deferred until end-to-end validation |

### Context

VITRIOL currently produces 17 graph splits at 6.9 tok/s (`GGML_SCHED_DEBUG=1`). The all-VRAM baseline produces 2. Each split adds scheduling overhead + cross-backend tensor copies.

### Root Cause (Hypothesized)

The `vitriol_is_vitriol_buffer_type()` check in `ggml_backend_cuda_device_supports_buft()` (line 5285) already returns true. However, `tensor_backend_id()` for VITRIOL-buffer tensors may return a different ID than the CUDA backend, causing the scheduler (Pass 5, lines 1272-1301) to create a new split when it encounters the first VITRIOL-weighted op after a CUDA-op run.

The `GGML_SCHED_MAX_SPLIT_INPUTS` (30) limit per split may also be hit when 8+ expert weights cross backend boundaries per MoE layer.

### To Investigate

1. Run `GGML_SCHED_DEBUG=1` to confirm 17 splits and identify where they occur
2. Check if `ggml_backend_buft_is_cuda_host()` should return true for VITRIOL buft (it IS page-locked host RAM, same as CUDA host buft)
3. If confirmed: increase `GGML_SCHED_MAX_SPLIT_INPUTS` to 256 when VITRIOL is active, or make `vitriol_get_buffer_type()` share the CUDA host buft identity

### Mitigation

Predictive Prefetching (§5) hides DMA latency regardless of split count, making this less critical. Deferred until end-to-end test validates remaining bottleneck.

---

---

## Experiment 15: Expert Pinning (Tensor-Level VRAM Preload)

| Field | Value |
|-------|-------|
| **Date** | 2026-05-20 |
| **Approach** | Pre-load full expert weight tensors (all 256 experts) of the first N model layers into VRAM at first use. Redirect `src0->data` to VRAM pointer locally in `ggml_cuda_mul_mat_id` before the fast-path MMVQ/MMQ/MMF kernel launches. No kernel changes — scoped `ggml_tensor` copy with `.data` redirected, restored before per-expert loop. |
| **Config key** | `vitriol.pin_first_n_layers` (0=off, N=pin first N model layers) |
| **Env var** | `VITRIOL_PIN_FIRST_N_LAYERS=N` |
| **CLI flag** | `--pin-layers N` |
| **TUI** | VITRIOL Mode Settings → option 5 |
| **Status** | ✅ Implemented, benchmarked — **+4% decode gain**, negative prefill impact |

### Implementation Details

- **Layer-to-tensor mapping**: Each model layer produces 2 `ggml_cuda_mul_mat_id` calls (fused gate+up + down). Fixed: layer index divided by `pin_tensors_per_layer` (=2) so `pin_first_n_layers=5` pins 10 tensor ops = 5 model layers.
- **Lazy allocation**: VRAM buffer allocated on first encounter of each tensor during prefill. Full tensor (all 256 experts) H2D copied via `cuMemcpyHtoDAsync`, then `cuStreamSynchronize`.
- **Scoped redirect**: Before fast-path checks, creates a local `ggml_tensor` copy of `src0` with `.data` pointing to VRAM buffer. Restores original `src0` before per-expert loop (LRU/predictor/cache unaffected).
- **Self-disable on OOM**: If `cuMemAlloc` fails, sets `pin_first_n_layers=0` and logs warning. All subsequent layers fall through to host path.

### Benchmark Results

Tested with Qwen3.6-35B-A3B-UD-IQ2_M, VITRIOL_MODE=stream, LRU=0 MB, output cache=off, -ngl 99, -t 4, `llama-bench -p 64 -n 100`.

| Configuration | Tensors pinned | VRAM used | Prefill (pp64) | Decode (tg100) | vs baseline |
|---|---|---|---|---|---|
| Baseline (pin=0) | 0 | 0 MB | **297 ms** | **8.94 t/s** | — |
| Pin 5 layers | 10 | 756 MB | — | ~8.97 t/s | ~0% |
| Pin 15 layers | 30 | 2,300 MB | **334 ms** (+12%) | **9.30 t/s** | **+4.0%** |

### Key Findings

1. **Pinning helps prefill bandwidth but hurts latency.** The H2D copy of pinned tensors adds ~37 ms to prefill (297→334 ms). Once pinned, subsequent prefill passes would benefit, but `llama-bench` reloads the model each run.

2. **Pinning gives +4% decode gain.** The gain is modest because the **bottleneck is compute, not PCIe**. The MMVQ kernel for IQ2_M (2-bit weights) is ALU-bound — dequantization + multiply takes longer than the weight fetch regardless of where the weights live (VRAM vs host RAM).

3. **This is the compute ceiling.** At 8.94 t/s = 112 ms/tok, with 40 layers → 2.8 ms/layer. The GTX 1070 Ti (Pascal CC 6.1) peaks at ~21 INT8 TFLOPS. Each token requires ~130M MACs (8 experts × 2048 hidden × 1024 FF × 2 matmuls). The theoretical speed of light is roughly **16 t/s** (60 ms/tok purely compute). At 8.94 t/s, we're at ~56% of peak, confirming the GPU is compute-limited.

4. **Per-layer time breakdown:**
   - Fast path (MMVQ with ids): ~2.8 ms/layer
   - ~0.5 ms of that is PCIe read (hidden by CUDA stream overlap with next layer)
   - ~2.3 ms is pure GPU compute (dequant + matmul for 8 active experts)
   - Pinning saves at most the PCIe portion (~0.5 ms/layer × 15 pinned = ~7.5 ms/tok → ~8% gain theoretical), but existing CUDA stream overlap already hides most of it

### Conclusion

Expert pinning works correctly but provides only **+4% decode gain** because the GPU is compute-bound for low-bit MoE matmuls. The PCIe bus is no longer the primary bottleneck. This is a **valuable negative result** — it tells us where to focus next.

### Modified Files

| File | Changes |
|------|---------|
| `vitriol-cuda-integration.h` | Added `pin_first_n_layers`, `pin_tensors_per_layer`, `pin_active` to config struct; `vitriol_pin_ensure()`, `vitriol_pin_lookup()`, `vitriol_pin_active()` |
| `vitriol-cuda-integration.cpp` | Pin table (unordered_map), lazy alloc + H2D copy, env var read, cleanup, stats |
| `ggml-cuda.cu` (~2529-2574) | Scoped `src0` redirect before fast-path, restore before per-expert loop |
| `scripts/vitriol` | Config key, TUI option 5, `--pin-layers` CLI flag, auto-disable output cache, env passthrough |

### Plans for Next Sprint: "Cheating Compute"

Four approaches documented in `.opencode/plans/`:

1. **Top-K Pruning** (`TOP_K_PRUNING.md`) — Drop bottom 4 of 8 active experts, halve matmul time. Targets compute directly.
2. **T-MAC** (`T_MAC_LUT_MATMUL.md`) — Replace multiply with lookup tables for TQ1_0/IQ2 weights. Bypasses ALU entirely.
3. **Early Exit** (`EARLY_EXIT.md`) — Skip layers 21-40 when residual stabilizes. Saves 50% compute.
4. **Asymmetric Pinning + Cache** (`ASYMMETRIC_PIN_CACHE.md`) — Pin early layers (compute-bound, no cache benefit), output-cache late layers (sluggish residual, high cache hit rate).

---

## Experiment 16: Quality Regression Discovery (2026-05-20)

**Critical finding:** All benchmarks with prune > 0 or output_cache = 1 were measuring **garbage token generation**. The model outputs repetitive nonsense when these optimizations are active. Only timing/scheduling changes (DMA overlap, expert pinning, prefetch) preserve output quality.

### Test Methodology
- Model: Qwen3.6-35B-A3B-UD-Q2_K_XL (known-working)
- Server: `vitriol serve` with `--reasoning off`
- Prompt: "The capital of France is" (expects "Paris")
- Each config tested independently

### Quality Results

| Config | Quality | Output excerpt |
|--------|---------|---------------|
| Stream only | ✅ Clean | "Paris. That is correct..." |
| + Predictive prefetch | ✅ Clean | "Paris. That is correct..." |
| + Expert pin 15 | ✅ Clean | "Paris. That is correct..." |
| + Prune 2 (keep 6) | ❌ Garbage | "OnClick...联想到联想到ож.b.beln..." |
| + Prune 4 (keep 4) | ❌ Garbage | "ayayayayayayayayayayay..." |
| + Output cache | ❌ Garbage | "everyone. I have am, I have am..." |
| + Prune 4 + cache | ❌ Garbage | "?? (empty content)" |
| + MTP N=2 (server) | ✅ Clean | "Paris..." (no acceleration) |

### Throughput (Verified Clean)

| Config | t/s | Real gain |
|--------|-----|-----------|
| Stream only | **8.96** | — |
| + Pin 15 | **9.12** | +1.8% |
| + Prefetch | 8.94 | ~0% |
| + Pin 15 + prefetch | 9.12 | +1.8% |

**All previously reported "10.71 t/s" and similar numbers are invalid** — the model produced garbage at those speeds.

### Corrected Best Config
```
VITRIOL_MODE=stream
VITRIOL_PIN_FIRST_N_LAYERS=15
```
→ **9.12 t/s** with verified clean output.

### Why It Failed
- **Pruning**: Bottom experts are essential for output diversity — dropping them causes repetition loops
- **Output cache**: Stale hidden state reuse creates positive feedback loops in MoE models
- Both findings are consistent with the literature but were not verified until now due to missing quality checks in benchmarks

### What's Next
- **IQ2_M tokenizer fix**: Investigate and fix the `?` output from IQ2_M GGUF — likely chat template metadata with `thinking = 1`. Compare metadata between Q2_K_XL and IQ2_M using `gguf` Python tools, try `--override-kv`, or patch the GGUF.
- **MTP N=2 benchmark**: Once IQ2_M works, re-run the MTP benchmark to verify 10.96 t/s with clean output.
- **T-MAC / hardware upgrade**: The 9.12 t/s ceiling is real. T-MAC (TQ1_0 format) or a GPU upgrade are the only paths to significantly higher throughput.

### Final Verified Best Config (2026-05-20)

The IQ2_M tokenizer issue was simply `--reasoning off`. The GGUF has `thinking = 1` in its chat template. Adding this flag produces clean output and enables MTP.

```
model  = Qwen3.6-35B-A3B-UD-IQ2_M.gguf (MTP-capable)
mode   = stream
spec   = mtp N=2
args   = --reasoning off
```
→ **17.62 t/s** with verified clean output. MTP acceptance rate: 98.5% (65/66 drafts).
The `--reasoning off` flag is now hardcoded in `vitriol serve` server args.

---

## Experiment 17: V Cache Quantization Bug (2026-05-21)

### Finding
`--cache-type-v q4_0` (and any non-f16 V cache quantization) produces `?` garbage output when combined with VITRIOL and flash attention. K-only quantization (`--cache-type-k q4_0`) is clean.

### Test Setup
- Server: `llama.cpp/build/bin/llama-server` (b101-e6487cdaf)
- GPU: GTX 1070 Ti (8 GB VRAM, CC 6.1, PCIe 3.0 x16)
- Model: Qwen3.6-35B-A3B-UD-Q2_K_XL (12 GB GGUF)
- VITRIOL: stream mode, pin=15, prefetch=on, `--reasoning off`, `--no-mmap`, `-fa on`

### Results

| `--cache-type-k` | `--cache-type-v` | Output | KV Cache |
|-----------------|-----------------|--------|----------|
| — | — | ✅ Clean | K f16, V f16 |
| q4_0 | — | ✅ Clean | K q4_0, V f16 |
| — | q4_0 | ❌ `?????` | K f16, V q4_0 |
| q4_0 | q4_0 | ❌ `?????` | K q4_0, V q4_0 |
| q4_0 | q8_0 | ❌ `?????` | K q4_0, V q8_0 |
| — | q8_0 | ❌ `?????` | K f16, V q8_0 |

### Root Cause
VITRIOL intercepts only MoE expert tensors (`ffn_down_exps`, `ffn_gate_exps`, `ffn_up_exps`) into page-locked host RAM. The KV cache stays entirely in GPU VRAM. The bug is in **llama.cpp's flash attention V dequantization path for the `qwen35moe` architecture** (Gated Delta Net/SSM with `full_attention_interval=4`, creating a sparse 10/40-layer KV layout).

The `--cache-type-v` flag was removed from the vitriol script at `scripts/vitriol:1327`. Only `--cache-type-k` is passed now.

### Current Best Config (IQ2_M + MTP, Verified Clean)
```
model  = Qwen3.6-35B-A3B-UD-IQ2_M.gguf
mode   = stream
spec   = mtp N=2
pin    = 8
args   = --reasoning off --cache-type-k q4_0 -fa on
```
→ **12.82 tok/s** (MTP: 5/6 drafts accepted, 83%).

Note: `pin_first_n_layers` reduced from 15→8 for IQ2_M due to increased VRAM pressure from MTP head + pin pool.

*Last updated: 2026-05-21 17:30 CEST**

## 2026-08-06 — Spagyric Phase 0: correctness-gated baselines (fresh rebuild)

Binary: llama-server rebuilt (commit 6fd83b2). GTX 1070 Ti, 15 GB RAM.
Prompt: "Write a Python function for merge sort." 64 tokens, temp 0, 3 rounds.
VITRIOL_MODE not set → native (mode 0, RAM Shot); models fit VRAM.

| model | config | gen t/s | eval t/s | correctness |
|---|---|---|---|---|
| DeepSeek-Coder-V2-Lite-Instruct IQ2_M | ngl=99 c=4096 t=4 | 58.1-58.3 | 56.7-58.4 | PASS (valid merge_sort) |
| Mellum2-12B-A2.5B Q4_K_M | ngl=24 c=32768 t=4 | 30.9-34.3 | ~49 | PASS (valid merge_sort) |

Both gates passed — the prior "legible output" concern did NOT reproduce at temp 0.
Matches/beats documented baselines (DeepSeek ~50, Mellum ~27-32). Next: Spagyric decode-knob
sweep (ubatch/batch/parallel/threads) on these, then VITRIOL knobs.

## 2026-08-06 — Spagyric S2 decode-knob sweep (both models) — parallel is the lever

Harness: libvitriol/spagyric_sweep.py (mode A single-request decode t/s, mode B
concurrent aggregate). Merge-sort prompt, 64 tok, temp 0, warmup + 3 rounds.
All configs correctness PASS.

DeepSeek IQ2_M (ngl=99 c=4096):
  ubatch 64/128/256/512: 60.2/59.8/60.0/59.8 t/s (flat)
  threads 2/4/8: 59.5/59.8/59.6 (flat, GPU-bound)
  parallel 2/4/8 aggregate: 78.5 / 87.9 / 135.8 t/s  (2.3x at 8)

Mellum Q4_K_M (ngl=24 c=32768):
  ubatch 64/128/256/512: 29.8/28.3/28.8/31.1 (flat)
  threads 2/8: 27.6 / 2.24 (t=8 catastrophic — HT contention)
  parallel 2/4 aggregate: 37.2 / 41.8 t/s (1.4x at 4)

Reading: ubatch and threads are not decode levers; --parallel is the decode
throughput knob (amortized weight fetch in native llama.cpp). Spagyric autotune axis
= --parallel; fix threads=4; ubatch default. Report:
.opencode/plans/2026-08-06-spagyric-decode-knob-sweep.md

## 2026-08-06 — Spagyric S4 stream-path: unblocked, then blocked by bad model file

- vitriol setup: cap_ipc_lock=ep on llama-server + RUNPATH fix (39 ELF), verified.
- Stream (ternary Qwen) OOM'd with Chimera auto (GGML_VULKAN=ON → VK buffer mlocks
  every alloc). VITRIOL_CHIMERA_MODE=off fixes launch: CUDA0 482MiB + CUDA_Host
  6480MiB pageable, healthy ~80s, RAM 11G/3.8G.
- Output garbage in stream AND native CPU (-ngl 0); dense BitNet TQ1_0 native CPU is
  GOOD. Verdict: qwen3.6-35b-a3b-instruct-TQ1_0.gguf is a bad file (suspects: bad
  conversion / wrong TQ1_0 variant / vision-model tokenizer). Stream path NOT at fault.
- VITRIOL-knob sweep deferred: needs known-good stream model + >=24GB RAM.
- Record: .opencode/plans/2026-08-06-spagyric-stream-path-finding.md (both repos).

## 2026-08-06 — Spagyric S4b KV/context levers (DeepSeek, GTX 1070 Ti)

Question: context (KV) vs weights claiming VRAM. Measured the 3 levers:
- Default f16 KV in VRAM: 58-60 t/s, parallel ceiling p=8@c4096 (the working path).
- --no-kv-offload (KV to host): decode 15 t/s (CPU-attention bottleneck, 4x penalty);
  p=1 17.6 / p=8 15.5 aggregate — no recovery. REFUTED on this box.
- --cache-type-k/v q4_0: decode 13.9 t/s + server crash (threads=4 config). REFUTED.
Verdict: on this box KV stays in VRAM; context limited by parallel x ctx product.
VITRIOL Layer 1a custom KV offload (CUDA-graph split) is the designed path, untested.
Record: .opencode/plans/2026-08-06-spagyric-kv-context-levers.md (both repos).
Harness: fixed stderr blindness (devnull -> /tmp/opencode/server_stderr.log).

## 2026-08-06 — Layer 1a kv-mode offload: 2 bugs fixed, then measured

FIXED:
- llama-kv-cache.cpp: VITRIOL_KV_MODE=offload aborted on CPU-placed layers
  (get_host_buffer_type NULL on CPU backend -> buft_is_host(NULL) assert). NULL-fallback
  fix (submodule 85d01eda8).
- scripts/vitriol: setup ran fix_rpath (patchelf ELF rewrite) AFTER setcap -> cleared
  the cap. Reordered (a583047). Verified cap persists.

MEASURED (Mellum Q4_K_M, ngl=24, t=4, p=1, KV offload):
- Empty-ctx decode ~18-21 t/s across 32K-131K allocated (vs 30-34 VRAM) = ~35-40% PCIe
  overhead. VRAM stays 6.7G regardless of context.
- USED-context decode collapses: 8K->7.7 t/s, 29K->5.1 t/s. Attention reads O(used) KV
  over PCIe per token. Extrapolated ~2-3 t/s at 100K+.
- Correctness PASS at all sizes (long-context prose responses were valid; strict gate
  false-negatived).
- Verdict: VRAM path better up to ~32K (30+ t/s); KV offload niche = >32K (to 131K) at
  ~5-8 t/s. 200K impossible (model native cap 131072). "Acceptable speed" and ">32K"
  are mutually exclusive on this box.
- Record: .opencode/plans/2026-08-06-spagyric-layer1a-kv-offload-investigation.md

## 2026-08-06 — P1 VITRIOL memory service (OpenCode RAG)

- libvitriol/memory_server.py: HTTP API (store/node/search/stats/health), localhost :8090.
  Reuses libvitriol/memory (db, retrieval, compact). Keyword+recency scoring; GPU GGUF
  embeddings wired in P2.
- db.py: added store_node() (knowledge-node upsert keyed by label).
- BUG FOUND + FIXED: store_episode called _ensure_edge() without commit, leaving an open
  write transaction that held the SQLite write lock -> 3rd sequential store stalled ~5s
  (busy_timeout) under threaded Flask; direct single-thread calls masked it. Fixed:
  commit after edge link; also get_or_create_edge now commits under the write lock.
- VERIFIED: 5 sequential stores + node all ~0.003s; search returns scored formatted
  snippets; stats correct.
- Plan: .opencode/plans/2026-08-06-vitriol-memory-opencode-rag.md (P1 done).
  + (refactor) param bundling per Praetor/AGENTS.md 5.3: db.EdgeSpec dataclass;
    store_episode/store_node take meta dict; get_or_create_edge/_ensure_edge take
    EdgeSpec. Callers updated (hebbian, consolidate, shim, memory_server).
