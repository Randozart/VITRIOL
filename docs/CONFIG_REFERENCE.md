# VITRIOL Configuration Reference

Every flag, what it does, when to use it, and when not to.

---

## `--memory-mode on | off` (env: `VITRIOL_MEMORY_MODE`, config: `memory.mode`, default: `off`)

**What it does:** Activates the Python Flask shim. The shim intercepts every chat request, retrieves relevant context from a per-project SQLite memory database, injects it as a system message, forwards to llama-server, then stores the conversation turn (user + assistant) back into the database. Enables cross-session persistent memory.

**Port behaviour:**
- `off`: llama-server listens directly on port 8279.
- `on`: llama-server moves to port 8278, shim listens on 8279. OpenCode config never changes (still points to 8279).

**Use when:** You want the model to remember context across OpenCode sessions — yesterday's bugfix conversation, project-specific knowledge, recurring patterns. The memory DB accumulates episodes per project and uses Hebbian weight updates to strengthen frequently co-occurring relationships.

**Don't use when:** You need maximum throughput. The shim adds ~19% overhead (5.03 vs 6.21 tok/s) from Flask routing, JSON parsing, SQLite reads/writes, and context injection. For single-session coding tasks where you don't need cross-session recall, leave it off.

**Requires:** `libvitriol/memory/` — 7 Python modules (always present, only loaded when mode is on).

---

## `--kv-mode standard | offload | sparse` (env: `VITRIOL_KV_MODE`, config: `kv.mode`, default: `standard`)

### `offload`

**What it does:** Moves the KV cache from GPU VRAM to page-locked host RAM (`CUDA_Host` buffer type). The GPU reads/writes KV cache entries over PCIe DMA instead of GDDR5. Frees ~470 MiB of VRAM for a 24K-token context on a 10-layer model.

**Use when:** You need larger context windows on a VRAM-constrained GPU. With the 12 GB model already using 1.3 GiB VRAM for weights + compute buffers, the remaining ~6.7 GiB can hold ~280K tokens of KV cache in host RAM vs ~78K in VRAM. Also produces **2 graph splits** instead of 17, reducing scheduling overhead.

**Don't use when:** Your GPU has sufficient VRAM for the context you need (e.g., 24 GB cards). The PCIe round-trip for KV reads adds ~0.4 tok/s overhead (5.80 vs 6.21).

### `sparse`

**What it does:** Enables attention-score-based KV eviction inside llama.cpp's `llama-kv-cache.cpp`. Every cache cell tracks its attention score. When the cache fills, `evict_sparse()` preserves the first 4 tokens (attention sinks) + a recent window, then drops low-scoring middle tokens. Provides 4-8x effective compression.

**Use when:** You want extreme effective context length. Composes with `offload` for maximum reach. The eviction is position-based, so no attention-score CUDA kernel modifications were needed.

**Don't use when:** Your prompts require perfect recall of every token in the middle of a long context. Extremely old or low-attention tokens may be evicted.

### `standard` (default)

Pure VRAM KV cache. Maximum generation speed, minimum context window. Default for a reason — use this unless you need one of the above.

---

## `--frozen-prompt on | off` (env: `VITRIOL_FROZEN_PROMPT`, config: `frozen_prompt`, default: `off`)

**What it does:** Identifies system and tool messages at the start of the conversation as a "stable prefix." These messages are kept byte-identical across requests — never truncated, never metadata-stripped. llama.cpp's prompt cache recognizes the unchanged prefix and skips re-evaluation on subsequent requests.

**Use when:** You stream many requests to the same model in the same session. The first request pays full prefill cost; subsequent requests reuse the cached KV prefix. At 20K tokens, this drops prefill from ~16 minutes to ~1 minute. Complements `--memory-mode on` (the shim already keeps frozen messages intact).

**Don't use when:** Your system prompt changes frequently, or you only make one request per session. No benefit if there's nothing to cache.

**Requires:** Any messages designated as "frozen" must be byte-identical across requests. The shim identifies `system` and `tool` roles automatically.

---

## `--semantic-mode on | off` (env: `VITRIOL_SEMANTIC_MODE`, config: `memory.semantic_mode`, default: `off`)

**What it does:** Replaces Jaccard keyword overlap with cosine similarity (via sentence-transformers `all-MiniLM-L6-v2`) for relevance scoring in the memory retrieval path. Embeddings are computed lazily and cached in the memory DB's `embeddings` table (keyed by SHA-256 content hash).

**Use when:** You're using memory mode and want better retrieval quality. Keyword overlap fails on synonyms and paraphrased queries; cosine similarity captures semantic meaning. Especially valuable once the memory DB has hundreds of episodes.

**Don't use when:** `sentence-transformers` is not installed (it's an optional dependency). The first inference will download the ~80 MB model. Small projects with <50 episodes won't see a meaningful difference from keyword search.

**Requires:** `pip install sentence-transformers numpy`

---

## `VITRIOL_PREDICTIVE_PREFETCH=1` (env var only, no CLI flag, default: off)

**What it does:** Inside `ggml_cuda_mul_mat_id`, records which expert indices were used in the current MoE layer call. Before the next call, fires async DMA (`vitriol_lru_prefetch`) to load the same experts into the LRU VRAM cache while the `ids` tensor is being copied from GPU to host. The heuristic assumes adjacent MoE layers tend to activate similar expert sets (60-70% hit rate).

**Use when:** You want every last drop of throughput. On cache hit, the expert is already in VRAM before the matmul starts — hiding the ~100ms PCIe load behind the ~30ms `cudaMemcpyAsync` + synchronize at the start of `ggml_cuda_mul_mat_id`. Estimated +10-20% tok/s gain.

**Don't use when:** You're on battery power or have a very weak CPU. The predictor loop iterates over up to 256 experts per layer, adding ~5% CPU overhead per token. On i7-3770 this is negligible; on a laptop Celeron it may be meaningful.

**Requires:** `--kv-mode offload` or RAM Shot mode (any mode with the LRU VRAM cache active).

---

## `--verbose` / `VITRIOL_VERBOSE=1`

**What it does:** Prints detailed VITRIOL diagnostics: LRU cache stats (hits, misses, evictions), buffer allocation sizes, config dump, and per-request logging from the shim.

**Use when:** Debugging performance issues, tuning LRU cache size, or verifying that VITRIOL modes are active.

**Don't use when:** You don't need the log noise. Adds ~10 lines of output per inference request in memory mode.

---

## `vitriol config` TUI settings

Accessed via `vitriol config` at the terminal. All settings are persisted to `~/.vitriol/config` in INI-style format.

| Menu | Setting | Values | Description |
|------|---------|--------|-------------|
| GPU | Device Index | 0-7 | CUDA device to use |
| GPU | Exclude Secondary | true/false | Exclude GTX 960 (CC 5.2) |
| Model | Path | file path | GGUF model location |
| Model | Context | 512-32768 | Context window (tokens) |
| Model | Threads | 1-32 | CPU threads for prompt processing |
| Model | GPU Layers | 1-99 | Layers to offload to GPU |
| VITRIOL | Operation Mode | stream/sync/async/off | RAM Shot + LRU (stream) or alternatives |
| VITRIOL | LRU Cache | 0-4096 MB | VRAM pool for hot expert caching |
| VITRIOL | Verbose | true/false | Debug logging |
| Memory | Memory Mode | on/off | Cross-session persistent memory |
| Memory | Semantic Search | on/off | Sentence-transformers retrieval |
| Context | KV Mode | standard/offload/sparse | KV cache placement + eviction |
| Context | Frozen Prompt | on/off | Stable prefix KV caching |
| Server | Host | IP | Bind address |
| Server | Port | 1024-65535 | API port |
| Server | Parallel | 1-8 | Concurrent request slots |

---

## Recommended Configs by Use Case

### Max throughput (daily OpenCode use)

```bash
vitriol serve --detach
# 6.21 tok/s, 1.3 GiB VRAM, no memory overhead
```

### Long context coding sessions

```bash
vitriol serve --detach --kv-mode offload --frozen-prompt on
# 5.80 tok/s, 20,000+ token context, prefill cached after first request
# 2 graph splits (optimal scheduling)
```

### Persistent memory (multi-session projects)

```bash
vitriol serve --detach --memory-mode on --semantic-mode on
# 5.03 tok/s, cross-session recall, semantic retrieval
```

### Maximum everything

```bash
VITRIOL_PREDICTIVE_PREFETCH=1 vitriol serve --detach \
  --memory-mode on \
  --kv-mode offload \
  --frozen-prompt on \
  --semantic-mode on
# 4.5-5.0 tok/s, 20K+ context, persistent memory, semantic search, prefetch, cached prefill
```

---

## Cost-Benefit Quick Reference

| Flag | tok/s | VRAM saved | System RAM used | Best for |
|------|-------|------------|-----------------|----------|
| (none — standard) | 6.21 | ~8.7 GiB* | ~10 GiB* | Daily use |
| `--kv-mode offload` | 5.80 | +0.5 GiB | +0.5 GiB | Long contexts |
| `--memory-mode on` | 5.03 | — | +~10 MB | Cross-session recall |
| `--semantic-mode on` | ~5.0 | — | +80 MB | Large memory DBs |
| All of the above | ~4.5-5.0 | ~9.2 GiB | ~10.6 GiB | Max capability |

*\*RAM Shot baseline: expert weights in host RAM (10 GiB), non-expert weights in VRAM (1.3 GiB).*

For detailed test methodology and measured numbers, see `TEST_REPORT_2026-05-17.md`.

---

*Last updated: 2026-05-17*
