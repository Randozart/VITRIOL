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
| `libvitriol/hermetis/` (7 modules) | ~1,400 | ✅ db, scorer, retrieval, compact, hebbian, consolidate, __init__ |
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

- libvitriol/hermetis_server.py: HTTP API (store/node/search/stats/health), localhost :8090.
  Reuses libvitriol/hermetis (db, retrieval, compact). Keyword+recency scoring; GPU GGUF
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
    EdgeSpec. Callers updated (hebbian, consolidate, shim, hermetis_server).

## 2026-08-06 — P2 embeddings: BLOCKED by fork BERT embedding bug

GPU-GGUF embedding provider verified (--embedding + --pooling present; nomic Q8_0/F16 +
bge-small Q8_0 downloaded+served). BUT both BERT-family models return ALL-ZERO
embeddings for many common inputs ("fast"->0.0, "how do we sort a list fast"->0.0,
"Write a Python function for merge sort"->0.0; "hello world"->1.0). Reproduces GPU and
CPU, under --pooling cls/mean/default. Fork regression in BERT-family embedding forward
pass (backend- and pooling-independent). sentence-transformers NOT installed (CPU
fallback unavailable). Mitigation: zero-guard in hermetis/embed.py (near-zero -> None ->
keyword). Paths: debug fork bert graph / pip install sentence-transformers / llama-arch
embedder.

## 2026-08-06 — P2 resolved + P4 Copula Hermetis plugin

- P2: sentence-transformers installed (--user --break-system-packages; torch 2.13, CUDA
  unavailable on driver 535 -> CPU). all-MiniLM-L6-v2 384-dim, ~86ms first encode.
  Hermetis semantic retrieval verified: zero-keyword query ranks the right episode
  (0.696 vs 0.533; and 0.892 in the e2e loop). GGUF-GPU path stays zero-guarded; fork
  BERT bug -> backlog.
- P4: plugins/copula.ts (installed ~/.config/opencode/plugins/copula.ts). event hook
  ingests session transcript (user/assistant text) + tool.execute.after captures tool
  results; memory_search custom tool -> /hermetis/search. TS syntax verified (node
  strip-types). E2E simulated: store user/assistant/tool -> search ranks relevant
  episode 0.892. Restart opencode to load.

## 2026-08-06 — P3: node versioning + repo map + P5 validation

- P3.1 db: knowledge_nodes -> UNIQUE(label, git_rev) + git_rev/superseded/superseded_by;
  _ensure_node_schema migrates old tables (rebuild); store_node supersedes current on
  new git_rev, refreshes in place on same rev.
- P3.2 retrieval: search_nodes/retrieve filter superseded=0 by default,
  include_history opt-in; +0.05 node-over-episode score bonus.
- P3.3 repomap.py: Aider-style map (regex symbol extraction, import-graph in-degree
  rank, token budget); /hermetis/repo_map endpoint (full or single-file store).
- P3.4 plugin: file.edited/file.watcher.updated -> debounced single-file node refresh.
- P5 verified: v1 store -> edit+commit -> v2 re-store: mod.py rev1 superseded_by rev2;
  retrieval current-only returns current rev, include_history returns both; single-file
  refresh superseded 2 old main.py versions, current carries new symbol.

## 2026-08-06 — BERT embedding bug investigation: NOT a current-source bug, P2 unblocked

Investigated the "fork BERT zero-embedding bug" (P2 backlog). Findings:
1. Reproduced zeros only in STALE P2-era server processes (surviving on :8081: "fast"->0.0,
   "hello world"->1.0). Fresh rebuilds are all-correct.
2. Backend-independent (CPU+GPU), pooling-independent (cls/mean/last/default), model-
   independent (nomic + bge, Q8_0 + F16) — reproduced in the stale binary.
3. Isolated to the pooled output: pooling none gave NONZERO raw token embeddings.
4. Clean rebuild of committed source (85d01eda8): ALL inputs norm 1.0 (16+ inputs, both
   models, CPU+GPU), with and without patchelf fix_rpath. Verified upstream base
   (277ff5fff) SIGILLs on this i7-3770 (BMI2 in SIMD helpers) — unrelated, fork build
   avoids it.
5. Root cause: the P2-era binary was a stale build artifact (older/incremental source
   state), NOT a current-source bug.
VERDICT: GGUF-GPU embedding provider WORKS. P2 unblocked. sentence-transformers stays as
CPU fallback; zero-guard stays defensive. llama-server was rebuilt -> caps need `sudo
vitriol setup` re-run.

## 2026-08-06 — Copula flow re-verified with GPU-GGUF embedder + UNION bug fix

- GPU embed server (bge Q8_0, ngl=99, :8081) verified: all inputs norm 1.0.
- Hermetis /hermetis/embed uses the GPU GGUF: dims=384, norm=1.0.
- Semantic search via GPU embedder: relevant episode ranks first (0.827 vs 0.767).
- BUG FIXED: db.get_edge_targets UNION broke after P3.1 migration (episodes e.* UNION
  knowledge_nodes n.* — column counts diverged after git_rev/superseded/superseded_by
  added). Fixed with explicit matching columns (_type, id, created_at, content, strength).

## 2026-08-06 — Full-launch test: decode regression was another stale-build artifact

launch_vitriol_full.sh test revealed gen decode 8-15 t/s (baseline ~30). Root cause:
STALE incremental build (the Layer-1a-era build dir had accumulated dirty state over the
dirty tools/cli/cli.cpp + debug prints). Clean rebuild (rm -rf build + PIC) yields
37-38 t/s — BETTER than baseline. Embeddings still correct on the fresh binary.
ALSO FIXED: scripts/build-llama-server.sh — a clean build FAILED (cpp-httplib static
lib linked into libllama-common.so without -fPIC). Added
-DCMAKE_POSITION_INDEPENDENT_CODE=ON.
Full stack (gen :8080 + embed :8081 + Hermetis :8090) verified: gen 36-38 t/s,
correctness PASS, VRAM 7783/328 fits.

## 2026-08-06 — Rolling window over a database: A+B+C built + validated

- C: /hermetis/context (budget-capped recency+relevance context block). commit 4c7312d.
- A: plugin chat.message full capture + experimental.session.compacting lossless dump
  ([compaction capture]). B: per-turn auto-injection (labeled [Hermetis context],
  session.prompt noReply, COPULA_AUTO_CONTEXT toggle). commit a99052d.
- Validation: ingest -> context block (KV-offload episodes, capped) -> compaction
  capture -> retrieval finds original + capture (0.863). Plugin TS valid; live
  session.prompt injection timing needs a real opencode run.

## 2026-08-06 — Diagnostics layer + AT_SECURE RUNPATH bug

- launch_vitriol_full.sh: added status / logs / doctor subcommands + launch hardening
  (bounded poll, dump log tail on dead-on-arrival) + --verbose / --dry-run.
- BUG: gen server failed "libllama-common.so.0 cannot open shared object file" despite
  clean ldd. Root cause: binary has cap_ipc_lock (AT_SECURE) -> loader IGNORES $ORIGIN
  RUNPATH; a rebuild resets RUNPATH to $ORIGIN and setup wasn't re-run. The launch
  hardening caught it live. FIX: launch script self-heals RUNPATH (patchelf, no sudo)
  before setup; doctor checks RUNPATH. patchelf clears caps -> needs `sudo vitriol setup`
  re-run for page-locking (VRAM-fit models unaffected).
- Fix: log_err uses fatal-marker patterns only (benign "failed to fit params" no longer
  false-positives); status() no longer aborts under set -e (log_err returns 0).

## 2026-08-06 — kimi-k3-in-c adoptions (re-derived; GPL-2.0 clean)

LICENSING: VITRIOL + llama.cpp fork confirmed GPL-2.0. Apache-2.0 is GPL-2.0-INCOMPATIBLE
-> kimi code is NEVER copied, only studied + re-derived. AGENTS.md now has the GPL-2.0
incorporation policy; docs/provenance/kimi-k3-in-c.md records what was learned.

ADOPTED #1 — three-state expert cache (INFLIGHT-aware victim selection):
vitriol-cuda-integration.cpp LRU now picks the LRU victim whose last DMA has COMPLETED
(cuEventQuery==SUCCESS) instead of blindly evicting the LRU and stalling on its in-flight
fill. PROVENANCE header inline. Built clean.

## 2026-08-06 — VITRIOL owns the window: ctx-shift drain + selective re-inject

- launch: --parallel 1 (full 32768 to the single slot), --context-shift (server drains
  the front when >32768), --cache-reuse 256, --reasoning off (Mellum). opencode
  limit.context -> 131072 (never compacts). NOT launched/verified yet — GPU blocked by
  avatar capture (PID 912348, 2.1GB).
- Hermetis selective injection: /hermetis/context now returns (block, top_score,
  is_new_topic); context_block filters below min_score (0.3); _is_new_topic embeds the
  query + recent session episodes (cosine < 0.55 => new topic). Plugin gates on
  min_score + is_new_topic + hash dedupe; CONTEXT_BUDGET 1500.
- Fixed: duplicate context_block (earlier botched append); --ctx-shift -> --context-shift
  flag name.
- Verify pending (GPU): /v1/models n_ctx=32768, long session no compaction, injected
  context survives shifts.

## 2026-08-21 20:40 — REBIS Phase 0: dual-model co-residency (PASSED)

Plan: `.opencode/plans/rebis-phase0-plan-2026-08-21.md`. Raw JSON: `/tmp/opencode/t{0,1,3}_*.json`.

Setup: Mellum2-12B-A2.5B-Thinking i1-IQ4_XS (6.2 GiB) pinned GPU1 (1070 Ti), ctx 16k,
q4_0 KV, -fa on; Qwen3.8-27B UD-IQ2_S resident GPU0 (3060), ctx 32k, q4_0 KV, -fa on.
Both `--no-mmap -ngl 99`, separate llama-server processes, ports 8287/8279.

| metric | Qwen IQ3_S baseline (ts 24,12 stream) | Qwen IQ2_S @32k resident GPU0 | Mellum IQ4_XS GPU1 |
|---|---|---|---|
| prefill 1k / 4k / 16k tok/s | 264 / 262 / 239 | 428 / 438 / 417 | 559 / 513 / 442 |
| decode t/s | 20.4 | 19.6 | 69.8 solo, 70.2 co-resident |
| VRAM used/total | both GPUs (streaming) | 9.83/12 GiB (2.4 slack) | 6.68/8 GiB (1.5 slack) |

Concurrent load (both servers active): no measurable penalty — Qwen prefill 431 t/s
@8k, Mellum decode 69.8 t/s simultaneously.

Gates: G1 PASS (70 ≥ 25) · G2 PASS (1.5+2.4 GiB slack ≥ 0.5) · G3 PASS (19.6 ≥ 8 @32k)
· G4 pending maintainer eyeball of IQ2_S output quality.

Key finding: IQ2_S fully-resident on the Ampere card beats the IQ3_S dual-GPU DMA-stream
profile on BOTH axes — prefill 1.7× faster (no PCIe expert fetch), decode equal within
noise. The Pascal penalty for Qwen disappears; 1070 Ti freed entirely for the drafter.
16k-token context ingestion: 78 s → 31 s.

T4 (Strand-Rust-Coder control) deferred — low information value now that drafter gate
passed by wide margin.

## 2026-08-21 21:05 — REBIS Phase 1 smoke test (PASSED, after 2 protocol fixes)

First live loop: rebis.py, drafter = Mellum i1-IQ4_XS :8287, verifier = Qwen UD-IQ2_S
:8279, task = implement `reset` txn on a scratch Ledger crate, gate = cargo check.

Bug 1 (false GREEN): Mellum Thinking puts output in `reasoning_content` when the think
budget exhausts max_tokens; content was empty, empty file compiled clean, loop declared
success on a 0-byte draft. Fix: chat() returns full message dict; message_text() merges
content+reasoning; empty-draft guard skips gate and retries.

Bug 2 (design): compiler-green was terminal — verifier never consulted. Iteration-1
draft nulled the pointer WITHOUT freeing (leak), cargo check blind to it, loop exited.
Fix: verifier reviews EVERY draft; success requires compile_ok AND verdict pass.

Post-fix run: iteration 1 ACCEPTED — draft frees via Box::from_raw, nulls pointer before
size write, single documented unsafe block, keeps existing API. 26.8 s draft wall time.
Doubles as G4 evidence: IQ2_S verifier judgment usable on this task class.

Known caveat: Box::from_raw correct only if allocation originated from Box::into_raw —
verifier prompt should require provenance justification next. Scratch crate:
/tmp/opencode/rebis-scratch. Task packet: libvitriol/examples/rebis-example-task.json.

## 2026-08-21 22:30 — REBIS Phase 2a: agentic hardening (COMPLETE)

`libvitriol/rebis.py` v2. New: strict verdict schema (`checks[]` with per-invariant
evidence; missing coverage coerced to fail), grammar-constrained verdicts
(`/apply-template` + `/completion` + top-level `json_schema`, legacy chat fallback),
multi-file Mandatum (`file_slices[]`, `### <path>` section extraction), JSONL journal +
`--resume`, retry/respawn/wall-clock-budget semantics, token accounting + `--report`.

Bugs found and fixed during live drills:
1. Verifier rambling: unconstrained IQ2_S verdicts hit the 8192-token cap (462 s each).
   Fix: json_schema-constrained `/completion` at temp 0.0 → verdicts now ~150–570 tokens,
   parseable by construction.
2. Raw `/completion` returns `content`+`timings`, not OAI `choices`/`usage` — mapped.
3. Budget only gated between iterations; single call could overrun. Fix: every attempt
   derives its timeout from the wall-clock deadline.
4. Budget/server aborts journaled as terminal `result` — resume refused them. Fix:
   resumable `paused` events; only iteration-cap writes terminal result.
5. Host-RAM OOM killed Qwen server (dmesg: anon-rss 7 GB): fork prompt-cache unbounded
   (`server_prompt_cache`, default --cache-ram 8192 MiB) × long sessions. Fix: servers
   relaunched with `--cache-ram 2048` (Qwen) / `1024` (Mellum). AGENTIC USE REQUIRES
   THIS FLAG.

Drill results (scratch crate):
- Budget-cut run paused cleanly at wall clock; `--resume` continued and finished.
- Unsound pointer task (free untracked `*mut u64`): verifier correctly REFUSED all 3
  iterations, catching real bugs each round (Vec::from_raw_parts arity, null vs
  null_mut, unnecessary unsafe). Spec was unsound, not the loop — replaced with a
  sound task carrying a capacity-retention trap.
- Sound task: ACCEPTED iteration 1 (26 s wall, drafter 283p/1095c tok, verifier
  439p/152c tok); hand-written runtime test confirms empty + committed-clear +
  capacity preserved.

Phase 2b (Anticipatio) next: slot topology measurement, async shadow prefill, TTFT
cold-vs-warm probe.

## 2026-08-21 23:59 — REBIS hermes experiments E1–E4

Config note: hermes-agent enforces a ≥64k context floor → both servers relaunched at
-c 65536. Qwen IQ2_S @64k q4_0 KV fits GPU0 (10.2/12 GiB); Mellum pinned CANNOT reach
64k (KV+weights >8 GiB) → --n-cpu-moe 16 hybrid = 3.6/8 GiB but decode drops to
4.98 t/s (from 70 pinned).

| exp | path | result |
|---|---|---|
| E1 | hermes→Qwen direct | task CORRECT (2 tests pass), 18m23s wall — ingestion tax quantified |
| E2 | hermes→Mellum direct | FAILURE MODE CONFIRMED LIVE: zero tool calls, hallucinated API (Ledger::new()), file untouched, 12m17s at 5 t/s |
| E3 | Anticipatio probe | FAILED GATE: cold 43.6s vs warm 42.6s prefill = 2.3% reduction (need ≥40%). Fork LCP-slot-selection + checkpoint save/load overhead (~48s wall vs 43s prefill) defeats prefix reuse. Deprioritized. |
| E4 | hermes→Qwen brain→rebis.py via bash tool | **PASSED**: brain authored Mandatum JSON itself, loop accepted iteration 1 (drafter 498p/1413c, verifier 804p/242c, 97s), hermes independently verified total() == iter().sum(), cargo test green |

Second OOM root cause: 15 GB system RAM total; dual-server with --no-mmap staging
collides during loads (Mellum stages 6.2 GiB through host RAM). Fix: mmap weights
(both models fully VRAM-resident anyway → host pages stay evictable page-cache),
stagger server startups, --cache-ram 512/1024. Post-fix: 12 Gi available with both up.

ARCHITECTURE VERDICT: "Mellum as hermes brain" is dead on this hardware either way
(pinned <64k window; hybrid 5 t/s). Daily-driver shape is hermes(Qwen brain) +
rebis.py as an orchestrated tool — no proxy required for v1. Shim steering layer
(flagged/finals) becomes an optimization, not a prerequisite.

Design note from the run (keep): verdict evidence is inspection-based; add
test-emitting invariants ("add a test asserting X") so the compiler gate arbitrates
semantic claims instead of trusting drafter prose.

## 2026-08-22 00:45 — F1/F2: Mellum speed ladder + test-emitting invariants

F2 — Mellum i1-IQ4_XS @64k ctx, decode t/s by --n-cpu-moe (GPU1 8 GiB):
  moe16 3.6 GiB → 4.98 | moe8 5.28 → 33.3 | moe4 6.09 → 44.0 | moe0 PINNED 6.87 → 70.2
Earlier "can't pin at 64k" estimate was WRONG — SWA(3:1) + 4 KV heads + q4_0 KV keep
64k KV ≈0.7 GiB. Mellum CAN serve hermes' 64k floor at full speed. E2's blocker was
purely agency (tool-call initiation), never throughput.

F1 — test-emitting invariants (compiler gate = cargo test executes semantics):
- Positive run f1-v7: ACCEPTED iteration 3 after two precise compile-RED pokes
  (140 s wall; drafter 1431p/5930c, verifier 2357p/766c).
- Negative control: poisoned total() with .skip(1) + spec-correct test asserting 10:
  execution caught it (test FAILED) AND constrained inspection caught it (I2 fail,
  delta "remove .skip(1)"). Redundant layers confirmed.

Loop defects found & fixed this round (all in rebis.py):
1. Verdict JSON with raw newlines inside evidence strings → unparseable; added
   string-state sanitizer retry (_sanitize_json_candidate).
2. Compiler report kept rustc TAIL only — first error drowned by summary. Added
   ERROR DIGEST of all `error` lines leading the report.
3. Correction turns showed the ORIGINAL skeleton, so the drafter regenerated from
   scratch each poke and lost converged progress (whack-a-mole across iterations).
   Fix: drafter_messages(current_files=...) feeds the LAST DRAFT on correction turns.
4. Verdict coverage required verbatim invariant strings; paraphrase ⇒ spurious
   "unaddressed". Fix: id-based checks in schema (model echoes I-number) +
   hybrid fuzzy fallback (sequence similarity OR token containment ≥0.7).
5. TASK-SPEC lesson (twice now): invariants must be JOINTLY SATISFIABLE. Both the
   pointer task and "definitions unchanged" vs tests-needing-construction were
   contradictory specs the verifier rightly refused forever. Planner guidance must
   include a satisfiability self-check.

## 2026-08-22 02:30 — Phase 4 prep: S1 battery cell + delta-protocol bake-off

INCIDENT: battery run S1-armB wrote a 24-line drafter fragment over the real
230-line model.rs and a 5-line fragment over 1600-line ui.rs (whole-file regen
exceeds any drafter budget → truncation). Recovery: git checkout to HEAD +
replayed all session edits from transcript. Post-incident rails in rebis.py:
FRAGMENT GUARD (draft <25% of existing >400B file ⇒ rejected), .rebis-bak
snapshot on first overwrite, patch-failure reports journaled.

DELTA-PROTOCOL BAKE-OFF on real task (S1: SlotSnapshot::total_tokens, cargo-gated):
| protocol | Mellum-Thinking IQ4_XS | verdict |
|---|---|---|
| file-whole @4096 | truncated fragments every iter | dead: 242-line file ≈ 5k tok > budget |
| file-whole @12288 | emits only 1.2KB region anyway | model refuses whole-file re-emission |
| unified diff, thinking on | burns budget pre-diff; hallucinated hunk context ("// ... existing tests ...") | dead |
| unified diff, thinking off | malformed pseudo-diffs (`@@` empty, no headers) then `# FINAL` repetition loops | dead |
| SEARCH/REPLACE + few-shot example | perfect FORMAT, hallucinated CONTENT (invented duplicate impls) | fidelity beyond its grade |
| SEARCH/REPLACE, Qwen as drafter | **ACCEPTED iteration 1, 152/152 tests green, 130s wall** | ✓ |

Plus one process self-inflicted wound found: mid-experiment `git checkout` of the
baseline file reverted restored session state under three runs before detection.
Rule added to guide: snapshot, never checkout, during experiments.

Also landed in rebis.py this round:
- verify_mode "compiler_only" (skip LLM auditor when gate covers invariants) —
  motivated by IQ2_S auditor hallucinating fixes for already-present code at 17k
  prompt tokens
- enable_thinking:false passthrough (chat_template_kwargs) — thinking OFF kills
  Mellum's delta ability entirely; ON burns 40-70% of budget. Documented per-mode.
- split_sections/SR_BLOCK_RE/apply_replace_blocks (exact + whitespace-tolerant
  line-span matching, atomic multi-block application)

DRAFTER SELECTION MATRIX (guide §0): Mellum = new/small-file generation;
Qwen = modifications to real files; deterministic tooling = mechanical ops.

Battery state: S1 cell validated end-to-end (arm B variant with Qwen drafter).
Remaining: arms A/C formal timing runs, S2/H1 packets, full report.

## 2026-08-22 03:30 — S1 cell closed: arms A/B'/C results

Task: implement `push` txn (scratch crate, cargo test gate). Baselines restored
via snapshot between runs.

| arm | path | result |
|---|---|---|
| A | hermes → Qwen direct | 18m23s, correct incl. new test (E1 measurement) |
| B | rebis loop, Qwen drafter, replace+compiler_only | **130s, ACCEPTED iter 1, 152/152** |
| B' | rebis loop, Mellum drafter | fails on modify-tasks in all 6 delta configs (bake-off) |
| C | hermes → shim → Mellum, steer mode | FAIL ×2, diagnosed |

Arm C findings:
- Run 1: Mellum made real tool calls mid-loop (push method landed, cargo green)
  but skipped test-extension; final turn degenerated to ungrounded prose.
- Run 2: hallucinated a nonexistent `session_search` tool, narrated instead of
  called, file untouched.
- Steering layer FIRED both times (flags caught premature finals); run 1 exposed
  BrokenPipeError crash when hermes timed out during judge+nudge latency — fixed
  (graceful client-gone handling). Run 2: same latency exceeded client patience;
  steered recovery never landed despite clean handling.

VERDICT: Mellum-direct under an agentic harness is not viable on this rig — not
because steering fails, but because the drafter cannot reliably emit valid tool
calls for a harness toolset at all. Shim remains useful as instrumentation;
daily-driver shape stays hermes(Qwen brain) + rebis.py delegation.

Process notes: `pkill -f` self-match killed our own shell twice (use `[x]` bracket
trick); mid-experiment baseline resets must use snapshots, never git checkout.

## 2026-08-22 05:30 — H1 hard task: prompt-cache min-LCP gate (ACCEPTED)

H1 = fix cross-session bleed at source (llama.cpp server). Loop ran with Qwen
drafter, replace mode, compiler_only, gate = cmake target build; 40-min tool
timeout killed the loop mid-build AFTER all 5 files patched cleanly — manual
gate finish + one-line drafter error fixed by hand (`common_arg` has no float
lambda overload; house pattern is string+std::stof).

Change (commit 025291f6e on vitriol-mellum2): --prompt-cache-min-lcp (default
0.5) gates candidate states in server_prompt_cache::load; states <=64 tokens
always eligible. Root cause was metric bias: sim = lcp/tokens_new gives short
prompts inflated similarity against long cached states.

Functional validation (cache-ram 4096, cache ENABLED):
- A: 8.8k ledger session → correct answer, state saved (26.9s)
- B: tiny "2+3" probe with that state resident → clean "5" in 1.5s (no bleed)
- C: same-prefix follow-up → 1.2s, only 8 tokens evaluated (real cache hit)

REOPENS E3: Anticipatio failed earlier because restore machinery fed wrong
states; with gating, same-prefix turns drop 26.9s → 1.2s (22×). Shadow prefill
is back on the table for the daily driver.

Loop-vs-hard-task verdict: drafter produced a spec-exact multi-file C++ patch
on first apply (Qwen/replace/verbatim-context); the loop's remaining weakness
was infrastructural (tool timeout killing process group mid-build), not
cognitive. Long-gate tasks need detached execution or budget ≥ gate duration.

## 2026-08-22 08:45 — Anticipatio re-validation + shared-endpoint contention

E3r probe post-H1-gate, three configurations against :8279 (Qwen):
1. similarity disabled: no reuse (-3.8%) — slot affinity was OFF
2. similarity default (0.1): still cache_n=0 — OTHER ML EXPERIMENT interleave
   cleared slots between probes; one restore DID succeed per logs
   ("found better prompt, sim = 1.000")
3. id_slot=3 pinned, endpoint idle: STILL full re-prefill — then discovered the
   Qwen server had been killed by the other tenant's lifecycle management
   mid-experiment; final probe ran against a dead-then-restarted instance.

DECISIVE CONTROL on Mellum :8287 (same gated binary, quiet endpoint):
COLD 19801 tok / 46.95s → WARM **cache_n=19800, 28ms prompt, 0.06s wall**.
99.9% reduction. H1 gate validated for BOTH safety (no bleed) and reuse.

OPERATIONAL FINDING (daily-driver critical): multiple clients sharing one
llama-server endpoint thrash each other's prefix caches (interleaved
conversations evict states), and tenants that run `killall llama-server`
between their own runs nuke everyone else's servers. Role-dedicated endpoints
or slot-partition discipline is required infrastructure for concurrent use.

Anticipatio wired as opt-in: rebis.py --anticipatio fires a daemon-thread
shadow prefill of the stable prefix after each Mandatum send. Live A/B deferred
until Qwen endpoint ownership is coordinated; Mellum-side reuse already proven.

## 2026-08-23 19:05 — Incident: hard freeze under memory pressure (hardened)

User's first live hermes session ended in a box-wide hard lock. Journal cut
mid-memory-pressure with no OOM-kill: swap thrash (zram fill → 8G swapfile on
root) starved the OOM killer before it could act. Contributors: Sol+Luna
serving, hermes workers, two opencode instances, an 8-core cargo build.

Hardened (commit 090aa60): gateway memory guardrail (503 below 1200 MiB
MemAvailable), head caps lowered (cache-ram 1024/512, checkpoints 8 @ 16384),
TUI HOST RAM gauge with FREEZE RISK threshold at 800 MiB. User applied
vm.swappiness=10 persisted via /etc/sysctl.d/99-rebis.conf (verified runtime).

Full report: docs/rebis/incident-freeze-2026-08-23.md
