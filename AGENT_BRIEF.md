# VITRIOL — Project Status & Agent Brief

**Date:** 2026-05-17 22:00 CEST
**Repo:** `/home/randozart/Desktop/Projects/VITRIOL`
**Model:** Qwen3.6-35B-A3B-UD-Q2_K_XL.gguf (12.29 GB, 256 experts, 8 active/token)
**GPU:** GTX 1070 Ti (8 GB GDDR5, Pascal, PCIe 3.0 x16)
**CPU:** i7-3770 (4C/8T, DDR3)
**Binary:** `llama.cpp/build/bin/llama-server` (custom build with VITRIOL patches)

---

## Commit History (chronological)

```
84a4079 phase3: add KV quant, prompt lookup, engine-mode CLI flags + TUI
33c5bba fix: move _store_turn to background thread
005a8dd db: increase busy_timeout to 30s
41668c7 fix: SSE streaming proxy + write mutex
4368432 fix: DB lock contention
02a047b fix: strip stream flag from forwarded requests
c5a02e7 fix: handle OpenAI-style list content
b070adf docs: add OpenCode setup guide
e188ce6 docs: add CONFIG_REFERENCE.md
a448647 docs: add end-to-end test report
fa3a847 fix: shim memory mode bugs
6edb1f6 fix: handle content lists in shim
e2d262f phase2: add predictive prefetching
cd0a472 phase1: complete with semantic search
85e8ea4 docs: update experiment log
97d5a19 context: add KV mode, frozen prompt, sparse eviction
391b72e memory: add emulated memory architecture
```

---

## All Optimizations

### Layer 0: Foundation — Always Active

| # | Name | Mechanism | Status | Prior Art |
|---|------|-----------|--------|-----------|
| 0a | **VITRIOL Buffer Type** | Custom ggml backend buffer using `mmap` + `mlock` + `cudaHostRegister` for page-locked host RAM. GPU reads expert weights over PCIe DMA during MUL_MAT_ID. | ✅ Tested (10,040 MiB VITRIOL buffer) | llama.cpp PR #11397 (slaren), PR #6387 |
| 0b | **LRU VRAM Cache** | 512 MB VRAM pool caches hot experts. Async `cuMemcpyHtoDAsync` on dedicated CUDA stream, synced via `cuStreamWaitEvent`. Composite key `(tensor_base, expert_idx)`. | ✅ Tested | llama.cpp PR #11571 (fairydreaming) |

### Layer 1: Context Efficiency — Toggleable via CLI

| # | Name | Flag | Status | Gain | Prior Art |
|---|------|------|--------|------|-----------|
| 1a | **KV Cache Offload** | `--kv-mode offload` | ✅ Tested (5.80 tok/s) | Frees ~470 MiB VRAM, enables 20K+ context, 2 graph splits | FlexGen (2023), InfiniGen |
| 1b | **Sparse KV Caching** | `--kv-mode sparse` | ✅ Built | 4-8x effective context compression via attention eviction | SnapKV (Li et al. 2024), H2O (Zhang et al. 2023), StreamingLLM (Xiao et al. 2023) |
| 1c | **Frozen Prompt Caching** | `--frozen-prompt on` | ✅ Tested | Prefill ~93% faster (~16 min → ~1 min at 20K) | vLLM prefix caching (Kwon et al. 2023) |
| 1d | **KV Cache Quantization** | `--kv-quant q4_0` | ⚙️ Plumbed (untested) | 4x context (~96K tokens at Q4_0) via `--cache-type-k` | KIVI (Liu et al. 2024), vLLM PagedAttention |

### Layer 2: Engine Throughput — Toggleable via CLI

| # | Name | Flag | Status | Gain | Prior Art |
|---|------|------|--------|------|-----------|
| 2a | **Predictive Prefetching** | `VITRIOL_PREDICTIVE_PREFETCH=1` | ✅ Built | +10-20% tok/s estimated — uses previous layer's expert IDs to fire async DMA before next layer's IDs copy | MoE routing prediction (Fate, PROBE), KTransformers |
| 2b | **Prompt Lookup Decoding** | `--lookup N` | ⚙️ Plumbed (untested) | ~1.5-2x tok/s on coding tasks — N-gram speculation from existing context | Prompt Lookup Decoding (Umang 2024), LLM Accelerator (2024) |
| 2c | **Engine Mode** | `--engine-mode native` | ⚙️ Plumbed (untested) | Zero VITRIOL overhead for high-VRAM users | Standard llama.cpp |

### Layer 3: Persistent Memory — Toggleable via CLI

| # | Name | Flag | Status | Gain | Prior Art |
|---|------|------|--------|------|-----------|
| 3a | **Memory Mode** | `--memory-mode on` | ✅ Tested (5.03 tok/s) | Cross-session persistent memory via SQLite with Hebbian weight updates | MemGPT (2023), GraphRAG (Microsoft 2024) |
| 3b | **Semantic Search** | `--semantic-mode on` | ✅ Built | Cosine similarity via sentence-transformers, replaces Jaccard keyword overlap | — |
| 3c | **Cascading Multi-Hop Retrieval** | Always on in memory mode | ✅ Built | Spreading activation: direct search → edge traversal → score → rank | GraphRAG (Edge et al. 2024), spreading activation theory |
| 3d | **Hebbian Weight Updates** | Always on in memory mode | ✅ Built | Post-response edge weight adjustments based on co-occurrence | Hebbian theory (1949) |
| 3e | **Memory Consolidation** | Background thread | ✅ Built | Summarizes raw episodes into dense knowledge nodes during idle | Hippocampal consolidation theory |
| 3f | **Token-Budgeted Compaction** | Always on in memory mode | ✅ Built | Strict token budget for injected memory context | — |

### Layer 4: Shim Reliability — Always Active

| # | Name | Mechanism | Status |
|---|------|-----------|--------|
| 4a | **SSE Streaming Proxy** | Flask `stream_with_context` proxies SSE chunks from llama-server to client | ✅ Tested |
| 4b | **Write Mutex** | `threading.Lock()` serializes all SQLite writes | ✅ Tested |
| 4c | **Background Store** | `threading.Thread(daemon=True)` for post-stream DB writes | ✅ Tested |
| 4d | **Content List Handling** | Supports OpenAI multimodal format (`[{type:text, text:...}]`) | ✅ Tested |
| 4e | **Busy Timeout** | `PRAGMA busy_timeout=30000` on every connection | ✅ Tested |
| 4f | **Retry Loop** | 3 attempts with 1s backoff on DB writes | ✅ Tested |

---

## Measured Performance

| Mode | Text Gen | Prompt Eval | VRAM Used | Context |
|------|----------|-------------|-----------|---------|
| Standard (RAM Shot) | 6.21 tok/s | ~24-50 tok/s | ~1.3 GiB + 10 GiB host | 3-4K VRAM |
| + KV Offload | 5.80 tok/s | ~21 tok/s | ~1.3 GiB + 10 GiB host + 480 MiB host KV | 20K+ |
| + Memory Mode | 5.03 tok/s | ~21 tok/s | ~1.3 GiB + 10 GiB host | 20K+ |
| All features | ~4.5-5.0 tok/s | ~21 tok/s | ~1.3 GiB + 10 GiB host + 480 MiB host KV | 20K+ |

**Speculative (estimated, not yet tested):**
- + KV Quant Q4_0: ~5.8 tok/s, ~96K context
- + Prompt Lookup: ~8-12 tok/s on code tasks
- + GTX 960 Speculative: ~12-15 tok/s
- + Top-1 Expert Self-Speculation: ~10-14 tok/s

---

## Planned Optimizations (Not Implemented)

| # | Name | Sprint | Effort | Mechanism | Prior Art |
|---|------|--------|--------|-----------|-----------|
| P1 | **GTX 960 Speculative Decoding** | Sprint 2 | 2-3 sessions | Draft model on GTX 960, oracle on GTX 1070 Ti | Leviathan et al. 2022 (Google), Chen et al. 2023 (DeepMind) |
| P2 | **Top-1 Expert Self-Speculation** | Sprint 3 | 3-4 sessions | Draft: load 1/8 experts. Verify: load all 8 in parallel pass | Self-Speculative Decoding (2023), Mixture of Speculative Experts (2024) |
| P3 | **Block-Quantized PCIe Transfer** | Sprint 3.5 | 2-3 sessions | Compress expert weights to 2-bit for PCIe, decompress on GPU | MoQE (Kim et al. 2023), HOBBIT |
| P4 | **Disk Offload Fallback** | Sprint 3.5 | 1 session | File-backed mmap instead of mlock + cudaHostRegister | LLM in a Flash (Apple 2023) |
| P5 | **Fiddler CPU Activation Offload** | Sprint 4 | 4-5 sessions | Move 2 MB activations to CPU instead of 1.5 GB weights | Fiddler (Kamahori et al. 2024) |
| P6 | **GGUF TQ1_0 Type Registration** | Sprint 4 | 3-5 sessions | 1.58-bit ternary GGUF support | BitNet b1.58 (Ma et al. 2024) |
| P7 | **AVX-512 / SIMD CPU Kernels** | Sprint 4 | 2-3 sessions | Fast integer addition/subtraction for ternary experts | T-MAC (Microsoft 2024) |
| P8 | **Thread Affinity + Priority** | Sprint 4 | 1 session | Core pinning + nice for anti-freeze | — |
| P9 | **Rust Cognitive Orchestrator** | Phase 4 | 8-10 sessions | tree-sitter AST graph, cascading retrieval, state-bound .vitriol dir | Aider (Gauthier 2023), petgraph, rusqlite |
| P10 | **Graph Split Optimization** | Deferred | 1 session | Reduce 17→2 graph splits | ggml-backend.cpp scheduler |

---

## Key Files

| File | Purpose |
|------|---------|
| `scripts/vitriol` | Main CLI: config TUI, run, serve, stop. 1248 lines. |
| `libvitriol/vitriol_shim.py` | Flask proxy: memory mode, SSE streaming, context rectification. |
| `libvitriol/memory/` | 7 modules: db, scorer, retrieval, compact, hebbian, consolidate, __init__. |
| `llama.cpp/ggml/src/ggml-cuda/vitriol-cuda-integration.{cpp,h}` | LRU cache, config, predictive prefetching. |
| `llama.cpp/ggml/src/ggml-cuda/vitriol-buffer.{cpp,h}` | RAM Shot buffer type (mmap + mlock + cudaHostRegister). |
| `llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu` | VITRIOL hooks: supports_buft, LRU ensure, predictor prefetch/update. |
| `llama.cpp/src/llama-kv-cache.cpp` | KV offload buffer type + sparse eviction. |
| `llama.cpp/src/llama-kv-cells.h` | Per-cell attention score tracking. |
| `docs/OPTIMIZATION_PLAN.md` | 4-layer roadmap with 7 citations. |
| `docs/CONFIG_REFERENCE.md` | Every flag, trade-off, use case. |
| `docs/OPENCODE_SETUP.md` | OpenCode provider guide. |
| `docs/TEST_REPORT_2026-05-17.md` | Measured benchmarks. |
| `docs/AIRLLM_OPTIMIZATIONS.md` | AirLLM-inspired optimizations analysis. |
| `docs/PHASE3_OPTIMIZATIONS.md` | Phase 3 prior art citations. |
| `docs/EMULATED_MEMORY_ARCHITECTURE.md` | Memory design doc. |
| `EXPERIMENT_LOG.md` | 14 experiments documented. |
| `SESSION_LOG_2026-05-17.md` | Full session progress. |
| `~/.config/opencode/opencode.jsonc` | OpenCode provider config with X-Project-Id / X-Session-Id headers. |
| `~/.vitriol/config` | VITRIOL runtime config (INI format). |

---

## Critical Gotchas for Agents

1. **The engine is an in-tree directory, not a submodule** (since 2026-10-23 the llama.cpp fork lives at `engine/`, upstream frozen - see `docs/ENGINE_PROVENANCE.md`). C++ changes are normal monorepo commits under `engine/`. Rebuild with: `cmake --build engine/build -j$(nproc) --target llama-server` (binary ships as `vitriol-server`).

2. **`libvitriol/types.py` was renamed to `vitriol_types.py`** to avoid shadowing Python stdlib `types` module. Don't recreate `types.py`.

3. **`vitriol setup` must run once** (`sudo setcap cap_ip_lock=+ep`) for `mlock` to work. Without it, page faults stall the GPU silently.

4. **The shim's Flask dev server is single-threaded for streaming.** With `--parallel 2` in vitriol config, llama-server handles concurrent requests, but the shim itself may struggle with >1 concurrent SSE stream. For production, use waitress/gunicorn.

5. **Port swap:** memory mode ON → llama-server on PORT-1 (8278), shim on PORT (8279). memory mode OFF → llama-server on PORT directly. OpenCode config never changes.

6. **OpenCode sends `stream: true` by default.** The shim proxies SSE chunks. Without the streaming proxy fix (commit 41668c7), the shim returns buffered JSON and OpenCode hangs.

7. **The Qwen3.6 model uses ~180 tokens of reasoning** before answering. Set `max_tokens` generously or it produces empty content.

---

## Prior Art Bibliography

### Inference Engine & Offloading
- **llama.cpp** — Core inference engine. GGUF format, CUDA backend. (ggml-org)
- **LLM in a Flash** — Alizadeh, Mirzadeh et al. (Apple, 2023). Windowed streaming from flash/host memory.
- **Fiddler** — Kamahori, Gu, Zhu, Kasikci (2024). CPU-GPU orchestration for MoE activation offloading.
- **KTransformers** — kvcache-ai. YAML-based layer placement, double-buffer prefetch.
- **PowerInfer** — SJTU-IPADS. Neuron-level offloading with predictor.
- **T-MAC** — Microsoft (2024). LUT-based CPU inference for low-bit models.

### Quantization
- **BitNet b1.58** — Ma, Wang et al. (Microsoft Research, 2024). Ternary weights {-1, 0, 1} match FP16 perplexity.
- **MoQE** — Kim, Fahim, Awadalla (Microsoft, 2023). MoE experts are robust to 2-bit quantization.
- **Unsloth** — Daniel & Michael. Dynamic quantization formats (UD-Q2_K_XL).
- **3LTERN** — ELX987. W1.58A8 CUDA kernel for Pascal.

### KV Cache
- **SnapKV** — Li et al. (2024). Safe eviction of filler tokens, 8.2x compression.
- **H2O** — Zhang, Sheng et al. (2023). Heavy-Hitter Oracle token dropping.
- **StreamingLLM** — Xiao et al. (2023). Attention sinks preserve coherence.
- **KIVI** — Liu et al. (2024). 2-bit KV cache quantization.
- **vLLM PagedAttention** — Kwon et al. (2023). Block-level KV management.

### Speculative Decoding
- **Speculative Sampling** — Leviathan et al. (Google, 2022). Verification is parallelizable.
- **Speculative Sampling** — Chen et al. (DeepMind, 2023). Rejection sampling math.
- **Medusa** — Cai et al. (2024). Multiple decoding heads on a single model.
- **EAGLE** — Li et al. (2024). Feature-vector speculation, SOTA self-speculation.
- **Self-Speculative Decoding** — (2023). Layer skipping for draft generation.
- **Mixture of Speculative Experts** — (2024). Top-1 expert draft for MoE.
- **Prompt Lookup Decoding** — Umang (2024). N-gram speculation from context.

### Memory & Retrieval
- **MemGPT** — (2023). LLMs as operating systems with hierarchical memory.
- **GraphRAG** — Edge, Trinh et al. (Microsoft, 2024). Multi-hop retrieval via knowledge graphs.
- **Aider** — Gauthier (2023). tree-sitter AST repo mapping for code context.
- **LLM in a Flash** — (Apple, 2023). DRAM/VRAM streaming foundation.

---

*Generated: 2026-05-17 22:00 CEST*
