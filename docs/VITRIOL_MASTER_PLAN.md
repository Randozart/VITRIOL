# VITRIOL Master Plan

**Created:** 2026-05-17
**Last Updated:** 2026-05-18 09:00 CEST

**Hardware Profile:**
- GPU: NVIDIA GTX 1070 Ti (8 GB GDDR5, Pascal, PCIe 3.0 x16, compute 6.1)
- CPU: Intel i7-3770 (4C/8T, DDR3, AVX2)
- System RAM: 32 GB DDR3
- Storage: NVMe SSD

**Target Model:** Qwen3.6-35B-A3B (256 experts, 8 active/token, 34.66B params)
- Currently running: UD-Q2_K_XL GGUF (11.44 GiB, ~2.83 bpw)
- Planned: TQ1_0 GGUF (7.4 GiB, 1.69 bpw ternary) — already downloaded

---

## 1. Architecture Overview

### Buffer Architecture

VITRIOL defines three buffer types within llama.cpp's ggml backend:

| Buffer Type | Location | Mechanism | Use |
|-------------|----------|-----------|-----|
| **VITRIOL** (RAM Shot) | Page-locked host RAM | `mmap` + `mlock` + `cudaHostRegister` | Expert weights (10 GB) |
| **CUDA_Host** (KV offload) | Page-locked host RAM | `cudaHostAlloc` | KV cache when `--kv-mode offload` |
| **CUDA0** (device) | GPU VRAM | `cudaMalloc` | Base model weights, attention, compute buffers |

### Shim Architecture

```
OpenCode ──POST /v1/chat/completions──► vitriol_shim.py (port 8279)
                                              │
                                        1. Parse X-Project-Id, X-Session-Id
                                        2. Retrieve context from memory DB
                                        3. Inject as system message
                                        4. Forward to llama-server (SSE stream proxied)
                                        5. Store conversation turn (background thread)
                                              │
                                              ▼
                                        llama-server (port 8278 when memory mode on,
                                                      port 8279 when memory mode off)
```

### Key Files

| File | Purpose |
|------|---------|
| `scripts/vitriol` | Main CLI: config TUI, run, serve, stop. 1248 lines. |
| `libvitriol/vitriol_shim.py` | Flask proxy: memory mode, SSE streaming, context rectification. ~850 lines. |
| `libvitriol/hermetis/` | 7 modules: db, scorer, retrieval, compact, hebbian, consolidate, __init__. ~1,400 LoC. |
| `llama.cpp/ggml/src/ggml-cuda/vitriol-cuda-integration.{cpp,h}` | LRU cache, config, predictive prefetching. |
| `llama.cpp/ggml/src/ggml-cuda/vitriol-buffer.{cpp,h}` | RAM Shot buffer type (mmap + mlock + cudaHostRegister). |
| `llama.cpp/ggml/src/ggml-cuda/ggml-cuda.cu` | VITRIOL hooks: supports_buft, LRU ensure, predictor prefetch/update. |
| `llama.cpp/src/llama-kv-cache.cpp` | KV offload buffer type + sparse eviction (`evict_sparse()`). |
| `llama.cpp/src/llama-kv-cells.h` | Per-cell attention score tracking. |

### Port Swap Logic

- `--memory-mode off`: llama-server listens on `$PORT` (default 8279)
- `--memory-mode on`: llama-server listens on `$PORT - 1` (8278), shim on `$PORT` (8279)
- OpenCode config never changes — always points to port 8279

---

## 2. Full Optimization Catalog

### Layer 0: Foundation (Always Active)

| # | Name | Flag / Env | Status | Measured/Expected Gain | Prior Art |
|---|------|-----------|--------|----------------------|-----------|
| 0.1 | **RAM Shot** (expert weights in page-locked host RAM) | `mode=stream` (default) | ✅ Tested | Enables 35B on 8 GB GPU (10 GB → 1.3 GB VRAM) | llama.cpp PR #11397 (slaren), PR #6387 |
| 0.2 | **LRU VRAM Cache** (512 MB pool for hot experts) | `--lru-mb 512` (default) | ✅ Tested | ~+0.6 tok/s over RAM Shot alone on hit | llama.cpp PR #11571 (fairydreaming) |
| 0.3 | **VITRIOL Buffer Type** | Built into buffer type registration | ✅ Tested | Reports `is_host=true` → scheduler routes MUL_MAT_ID to CUDA | ggml-backend scheduler |

### Layer 1: Context Efficiency (Toggleable via CLI)

| # | Name | Flag / Env | Status | Measured/Expected Gain | Prior Art |
|---|------|-----------|--------|----------------------|-----------|
| 1.1 | **KV Cache Offload** | `--kv-mode offload` / `VITRIOL_KV_MODE=offload` | ✅ Tested | 5.80 tok/s, 20K+ context, 2 graph splits, ~470 MiB host KV | FlexGen (2023), InfiniGen |
| 1.2 | **Sparse KV Caching** | `--kv-mode sparse` / `VITRIOL_KV_MODE=sparse` | ✅ Built | 4-8x effective context compression via attention eviction | SnapKV (Li et al. 2024), H2O (Zhang et al. 2023), StreamingLLM (Xiao et al. 2023) |
| 1.3 | **Frozen Prompt Caching** | `--frozen-prompt on` / `VITRIOL_FROZEN_PROMPT=on` | ✅ Tested | Prefill ~93% faster (~16 min → ~1 min at 20K) | vLLM prefix caching (Kwon et al. 2023) |
| 1.4 | **KV Cache Quantization** | `--kv-quant TYPE` (f16\|q8_0\|q4_0) / `VITRIOL_KV_QUANT` | ⚙️ Plumbed (untested) | 4x context (~96K tokens at Q4_0) via `--cache-type-k` | KIVI (Liu et al. 2024), vLLM PagedAttention |
| 1.5 | **TurboQuant KV Cache** (planned) | `--cache-type-k turbo4 --cache-type-v turbo3` | 📋 Planned | 128K+ context via 3-4 bit KV compression | DeepMind TurboQuant, turbo-tan/llama.cpp-tq3 fork |

### Layer 2: Engine Throughput (Toggleable via CLI)

| # | Name | Flag / Env | Status | Measured/Expected Gain | Prior Art |
|---|------|-----------|--------|----------------------|-----------|
| 2.1 | **Predictive Prefetching** (heuristic) | `VITRIOL_PREDICTIVE_PREFETCH=1` | ✅ Built | +10-20% tok/s — uses previous layer's expert IDs for async DMA | MoE routing prediction (Fate, PROBE), KTransformers |
| 2.2 | **Prompt Lookup Decoding** | `--lookup N` / `VITRIOL_LOOKUP` | ⚙️ Plumbed (untested) | ~1.5-2x tok/s on coding tasks via N-gram speculation | Prompt Lookup Decoding (Umang 2024), LLM Accelerator (2024) |
| 2.3 | **Engine Mode** (native bypass) | `--engine-mode MODE` (vitriol-dma\|native) / `VITRIOL_ENGINE_MODE` | ⚙️ Plumbed (untested) | Zero VITRIOL overhead for high-VRAM users | Standard llama.cpp |
| 2.4 | **Router Lookahead** (planned) | `--router-lookahead` | 📋 Planned | Near 100% PCIe latency hiding — run router on CPU during GPU attention | Self-Speculative Decoding (2023) |
| 2.5 | **Block-Quantized PCIe Transfer** (planned) | `--transfer-compress` | 📋 Planned | ~+25% tok/s — compress experts to 2-bit for PCIe, decompress on GPU | MoQE (Kim et al. 2023), AirLLM |
| 2.6 | **Spatial Expert Packing** (planned) | GGUF rewriter tool | 📋 Planned | +20-30% PCIe utilization — group co-activated experts sequentially | — |
| 2.7 | **Top-1 Expert Self-Speculation** (planned, deferred) | `--self-speculate` | 📋 Planned | ~2x tok/s — draft with 1/8 experts, verify with all 8 | Mixture of Speculative Experts (2024) |

### Layer 3: Persistent Memory (Toggleable via CLI)

| # | Name | Flag / Env | Status | Measured/Expected Gain | Prior Art |
|---|------|-----------|--------|----------------------|-----------|
| 3.1 | **Memory Mode** (SQLite persistent memory) | `--memory-mode on` / `VITRIOL_MEMORY_MODE=on` | ✅ Tested | 5.03 tok/s, cross-session recall, 6 stored episodes per turn | MemGPT (2023), GraphRAG (Microsoft 2024) |
| 3.2 | **Semantic Search** (sentence-transformers) | `--semantic-mode on` / `VITRIOL_SEMANTIC_MODE=on` | ✅ Built | Cosine similarity replaces keyword Jaccard overlap | — |
| 3.3 | **Cascading Multi-Hop Retrieval** | Always on in memory mode | ✅ Built | Spreading activation: direct search → edge traversal → score → rank | GraphRAG (Edge et al. 2024) |
| 3.4 | **Hebbian Weight Updates** | Always on in memory mode | ✅ Built | Post-response edge weight adjustments based on co-occurrence | Hebbian theory (1949) |
| 3.5 | **Memory Consolidation** (background) | Background thread (daemon) | ✅ Built | Summarizes raw episodes into dense knowledge nodes | Hippocampal consolidation theory |
| 3.6 | **Token-Budgeted Compaction** | Always on in memory mode | ✅ Built | Strict token budget for injected memory context | — |

### Layer 4: Shim Reliability (Always Active)

| # | Name | Status | Description |
|---|------|--------|-------------|
| 4.1 | **SSE Streaming Proxy** | ✅ Tested | Flask `stream_with_context` proxies SSE chunks to client |
| 4.2 | **Write Mutex** | ✅ Tested | `threading.Lock()` serializes all SQLite writes |
| 4.3 | **Background Store** | ✅ Tested | `threading.Thread(daemon=True)` for post-stream DB writes |
| 4.4 | **Content List Handling** | ✅ Tested | Supports OpenAI multimodal format `[{type:text, text:...}]` |
| 4.5 | **Busy Timeout** | ✅ Tested | `PRAGMA busy_timeout=30000` on every connection |
| 4.6 | **Retry Loop** | ✅ Tested | 3 attempts with 1s backoff on DB writes |

### Layer 5: Planned / Future

| # | Name | Sprint | Effort | Expected Gain | Prerequisite |
|---|------|--------|--------|---------------|-------------|
| 5.1 | **TQ1_0 Ternary Model Integration** | B | 1-2 sessions | 8-20 tok/s | Model downloaded (7.4 GB) |
| 5.2 | **Fiddler CPU Activation Offload** | F | 4-5 sessions | +100-200% tok/s on ternary models | TQ1_0 model working + CPU path |
| 5.3 | **Non-Temporal Memory Streaming** | F | 1 session | Smoother OS during CPU compute | Fiddler CPU path |
| 5.4 | **Thread Affinity + Priority Scheduling** | F | 1 session | Anti-freeze for CPU compute | Fiddler CPU path |
| 5.5 | **Disk Offload Fallback** | — | 1 session | Enables low-RAM machines | — |
| 5.6 | **Rust Cognitive Orchestrator** | — | 8-10 sessions | tree-sitter AST graph, state-bound `.vitriol/` dir | All above stable |
| 5.7 | **GGUF TQ1_0/TQ2_0 Type Registration** | — | Already done | Ternary GGUF format in VITRIOL's fork | — |
| 5.8 | **Graph Split Optimization** | — | 1 session | Reduce 17→2 splits (already at 2 with KV offload) | ggml-backend scheduler |
| 5.9 | **GTX 960 Speculative Decoding** | ABANDONED | — | — | Confirmed fails on MoE (verification needs 64 experts at once → PCIe thrash) |

---

## 3. Measured Performance

| Mode | Text Gen | Prompt Eval | VRAM Used | System RAM Used | Context | Date Tested |
|------|----------|-------------|-----------|-----------------|---------|-------------|
| Standard (RAM Shot, Q2_K_XL) | **6.21 tok/s** | ~24-50 tok/s | 1.3 GiB | 10 GiB (VITRIOL buffer) | 3-4K | 2026-05-17 |
| + KV Offload | **5.80 tok/s** | ~21 tok/s | 1.3 GiB | 10 GiB VITRIOL + 480 MiB host KV | 20K+ | 2026-05-17 |
| + Memory Mode | **5.03 tok/s** | ~21 tok/s | 1.3 GiB | 10 GiB VITRIOL + 480 MiB host KV + ~10 MB SQLite | 20K+ | 2026-05-17 |
| All features (Q2_K_XL) | **~4.5-5.0 tok/s** | ~21 tok/s | 1.3 GiB | 10.5 GiB total | 20K+ | 2026-05-17 |

### Estimated (Not Yet Tested)

| Mode | Expected tok/s | VRAM | Notes |
|------|---------------|------|-------|
| TQ1_0 + native (full GPU) | 15-25 | 7.4 GB | Depends on CUDA TQ1_0 kernel support |
| TQ1_0 + RAM Shot | 8-12 | 1.3 GB | Experts in host RAM, 1.69 bpw → faster PCIe |
| TQ1_0 + Fiddler CPU | 12-20 | 1.3 GB | Activations to CPU, integer addition for ternary |
| TQ1_0 + all optimizations | 15-25 | 1.3 GB | Stacked: RAM Shot + Fiddler + Lookahead + Batch |

### Graph Splits

| Mode | Splits | Sched Copies |
|------|--------|-------------|
| Standard (RAM Shot) | 17 | 4 |
| + KV Offload | **2** | 1 |
| + KV Offload + native (TQ1_0) | 2 (expected) | 1 |

---

## 4. Sprint Plan

### Sprint A: OpenCode Stability (1 session)

| # | Task | Files | Effort |
|---|------|-------|--------|
| A1 | Fix request context crash: extract `project_id`, `session_id` from headers before spawning background thread | `vitriol_shim.py` | 10 min |
| A2 | Set `parallel = 1` in config to stop LRU thrash between slots | `~/.vitriol/config` | 30 sec |
| A3 | Add tool call format normalization: insert missing `type: "function"` before forwarding | `vitriol_shim.py` | 10 min |

### Sprint B: TQ1_0 Ternary Model Integration (1-2 sessions)

| # | Task | Expected Gain |
|---|------|---------------|
| B1 | Dry-run test: does the binary load TQ1_0 GGUF? | — |
| B2 | Full inference with RAM Shot mode (`--engine-mode vitriol-dma`) | 8-12 tok/s |
| B3 | Full inference with native mode (`--engine-mode native`) | 15-25 tok/s (if CUDA kernels exist) |
| B4 | If CUDA kernels missing: Fiddler CPU path (activations to CPU) | 12-20 tok/s |

### Sprint C: TurboQuant KV + Batch Tuning (1 session)

| # | Task | Flag |
|---|------|------|
| C1 | Increase batch sizes for faster prompt processing | `-b 4096 -ub 4096` |
| C2 | Test KV quant (already plumbed) | `--kv-quant q4_0` |
| C3 | If available: TurboQuant KV cache | `--cache-type-k turbo4 --cache-type-v turbo3` |

### Sprint D: Router Lookahead (1-2 sessions)

| # | Task | Files |
|---|------|-------|
| D1 | Run router FFN on CPU during GPU attention | New: `vitriol-router-lookahead.cpp` |
| D2 | Hook into existing `vitriol_predictor_prefetch()` | `vitriol-cuda-integration.cpp` |
| D3 | Add `--router-lookahead` CLI flag | `scripts/vitriol` |

### Sprint E: Spatial Expert Packing (3-4 sessions)

| # | Task |
|---|------|
| E1 | Build profiling script for expert co-activation patterns |
| E2 | Build GGUF rewriter to physically reorder tensors |
| E3 | Patch model loader to handle reordered layout |

### Sprint F: Fiddler CPU + System Stability (1-2 sessions)

| # | Task | Files |
|---|------|-------|
| F1 | Add `_mm256_stream_si256` non-temporal loads in ggml CPU kernel | `ggml-cpu/ops.cpp` |
| F2 | Add thread affinity + priority flags | New `--sys-priority` flag |
| F3 | Full Fiddler path: attention on GPU, ternary experts on CPU | `vitriol-cuda-integration.cpp` |

---

## 5. Bug Fix Log

| Date | Bug | Root Cause | Fix | Commit |
|------|-----|-----------|-----|--------|
| 2026-05-17 | Shim crash on startup | `libvitriol/types.py` shadows stdlib `types` module, breaking Python's own imports | Renamed to `vitriol_types.py` | `fa3a847` |
| 2026-05-17 | ImportError: os not defined | `compact.py`, `consolidate.py` use `os.environ` without `import os` | Added `import os` | `fa3a847` |
| 2026-05-17 | SQLite `database is locked` | Thread-local connections without `busy_timeout` under concurrent requests | Added `PRAGMA busy_timeout=5000` → 10000 → 30000 | `4368432`, `005a8dd` |
| 2026-05-17 | Thermal poll ValueError | `nvidia-smi` returns multi-line output on multi-GPU systems | Added `--id=0` and `.split('\n')[0]` | `fa3a847` |
| 2026-05-17 | Shim import path | Relative import `from . import memory` fails when run as script | Added `sys.path.insert(0, parent_dir)` + absolute import | `fa3a847` |
| 2026-05-17 | Duplicate Flask routes | `/context/archive` and `/context/retrieve` defined twice | Removed duplicate definitions | `6edb1f6` |
| 2026-05-17 | Content list concatenation (3 locations) | OpenCode sends content as `[{type:text, text:...}]` (list of parts) not plain string | Added `isinstance(content, list)` checks in `rectify_context`, `current_query` extraction, `format_episode` | `c5a02e7`, `6edb1f6` |
| 2026-05-17 | SSE streaming not proxied | Shim stripped `stream: true` from forwarded requests, returned buffered JSON — OpenCode hung waiting for SSE | Replaced with `stream_with_context` proxy | `41668c7` |
| 2026-05-17 | Request context crash in store | `_store_turn` accessed Flask request proxy after thread exited request context | Fixed in Sprint A (pending) | — |
| 2026-05-17 | Tool call format missing `type` | OpenCode omits `type: "function"` in tool_calls, llama-server rejects | Fixed in Sprint A (pending) | — |
| 2026-05-17 | LRU cache thrash with parallel slots | 2 slots ping-pong experts in 512 MB LRU pool → constant cache misses → 1.5 tok/s | Fixed in Sprint A (pending) | — |
| 2026-05-17 | Qwen3.6 model only outputs reasoning | Model has `thinking = 1` in chat template, all tokens consumed by `reasoning_content` | User must set `max_tokens` large enough; not a VITRIOL bug | — |

---

## 6. Key Findings from Prior Art

### AirLLM (lyogavin, 2023)
- **Approach:** Layer-by-layer swapping for dense models. Load one layer into VRAM, process, evict, load next.
- **Key insight:** Block-wise 4-bit/8bit quantization for PCIe transfer speedup (2x bandwidth).
- **For VITRIOL:** Not directly applicable (MoE sparsity is more efficient), but block-quantized PCIe transfer is worth stealing.
- **GitHub:** https://github.com/lyogavin/airllm

### Fiddler (Kamahori et al., 2024)
- **Approach:** Move activations to CPU instead of moving weights to GPU for MoE expert computation.
- **Key insight:** Activations are ~2 MB vs weights are ~1.5 GB. CPU does the expert math, GPU does attention.
- **For VITRIOL:** The `--engine-mode fiddler-cpu` path. Especially powerful with ternary models since CPU integer addition is fast.
- **Paper:** https://arxiv.org/abs/2402.14103

### SnapKV (Li et al., 2024) / H2O (Zhang et al., 2023)
- **Approach:** Monitor attention scores, evict low-scoring filler tokens from KV cache.
- **Key insight:** 5% of tokens get 90% of attention. Safe to drop the rest.
- **For VITRIOL:** Already implemented as `--kv-mode sparse` with attention-sink preservation.
- **Papers:** https://arxiv.org/abs/2404.14469 / https://arxiv.org/abs/2306.14048

### StreamingLLM (Xiao et al., 2023)
- **Approach:** Preserve first 4 tokens ("attention sinks") during KV eviction to maintain stability.
- **For VITRIOL:** Core insight behind `--kv-mode sparse` implementation.
- **Paper:** https://arxiv.org/abs/2309.17453

### Speculative Decoding — Standard (Leviathan 2022, Chen 2023)
- **Approach:** Small draft model guesses tokens, large model verifies in parallel.
- **For VITRIOL:** GTX 960 as draft engine was planned, but testing shows it fails on MoE models (see finding below).
- **Papers:** https://arxiv.org/abs/2211.17192 / https://arxiv.org/abs/2302.01318

### Speculative Decoding — Self-Speculation (Medusa/EAGLE 2024)
- **Approach:** Add small heads to single model to predict +1, +2, +3 tokens ahead. No second model needed.
- **For VITRIOL:** Layer skipping is most relevant — skip PCIe transfer for 80% of MoE layers during draft.
- **Papers:** Medusa (https://arxiv.org/abs/2401.10774), EAGLE (https://arxiv.org/abs/2401.15077)

### Speculative Decoding FAILS on MoE (Real-world test, 2026)
- **Finding:** YouTuber tested speculative decoding on MoE (Qwen) via PCIe offload. 65% acceptance rate but speed DROPPED from 17 to 11 t/s.
- **Root cause:** Verifying 8 speculated tokens requires 64 different experts at once → PCIe bus chokes on random reads.
- **For VITRIOL:** Abandon GTX 960 speculative decoding plan. The verification pass on MoE models causes memory thrash.

### Doctor-Shotgun Blog (2026)
- **Approach:** `-ot "exps=CPU"` regex to assign routed experts to CPU, `-b 4096 -ub 4096` for batch sizing.
- **Key insight:** Standard llama.cpp approach exactly matches VITRIOL's buffer type approach. VITRIOL does it at a lower level (buffer type instead of regex).
- **For VITRIOL:** Validate architecture direction. Add `-b 4096 -ub 4096` tuning. Mentioned ik_llama.cpp flags for reference.
- **Blog:** https://huggingface.co/blog/Doctor-Shotgun/llamacpp-moe-offload-guide

### YouTube GTX 1060 17 tps (2026)
- **Approach:** `--no-mmap --mlock` for zero-copy host RAM access + custom TurboQuant fork.
- **Key insight:** Independent discovery of same RAM locking technique VITRIOL uses. Validates the entire approach.
- **For VITRIOL:** Full architectural validation. TurboQuant KV (`turbo4/turbo3`) is the key to 128K context on 8 GB.
- **Source:** YouTube video (see transcript in session logs)

### mad-lab-ai TQ1_0 Ternary GGUF (2026)
- **Model:** Qwen3.6-35B-A3B quantized to 1.69 bpw ternary (TQ1_0) via TurboQuant methodology.
- **Size:** 7.4 GB total — small enough to consider full GPU fit.
- **Availability:** Downloaded and ready at `/home/randozart/Desktop/Projects/qwen3.6-35b-a3b-instruct-TQ1_0.gguf`.
- **For VITRIOL:** Primary target model for Sprint B. Could double throughput.
- **Source:** https://huggingface.co/mad-lab-ai/Qwen3.6-35B-A3B-tq-gguf

### BitNet b1.58 (Ma, Wang et al., Microsoft Research, 2024)
- **Approach:** Ternary weights {-1, 0, 1} match FP16 perplexity, eliminating floating-point multiply.
- **Key insight:** Theoretically enables CPU-based inference via integer addition.
- **For VITRIOL:** Foundation of the Fiddler CPU path. TQ1_0 GGUF type is the practical implementation.
- **Paper:** https://arxiv.org/abs/2402.17764

### T-MAC (Microsoft, 2024)
- **Approach:** Lookup-table-based CPU inference for low-bit models. Avoids even addition for ternary math.
- **For VITRIOL:** Potential optimization for Fiddler CPU path — LUTs could beat SIMD addition.
- **GitHub:** https://github.com/microsoft/T-MAC

### KIVI (Liu et al., 2024)
- **Approach:** 2-bit KV cache quantization with minimal accuracy loss.
- **For VITRIOL:** Informs `--kv-quant` flag direction and future TurboQuant KV integration.
- **Paper:** https://arxiv.org/abs/2402.02750

### vLLM PagedAttention (Kwon et al., 2023)
- **Approach:** Block-level KV cache management eliminating memory waste.
- **For VITRIOL:** Foundation of efficient KV cache handling. Not directly applicable to llama.cpp.
- **Paper:** https://arxiv.org/abs/2309.06180

### Prompt Lookup Decoding (Umang, 2024)
- **Approach:** N-gram matching from existing context — if a token sequence appeared before, reuse as draft.
- **For VITRIOL:** Already plumbed as `--lookup N`. Zero extra VRAM, free speed on code tasks.
- **GitHub:** https://github.com/apoorvumang/prompt-lookup-decoding

### MoQE (Kim et al., Microsoft, 2023)
- **Approach:** MoE experts are robust to extreme 2-bit quantization.
- **For VITRIOL:** Supports asymmetric quantization approach (keep base model high precision, experts low).
- **Paper:** https://arxiv.org/abs/2310.14713

### PowerInfer (SJTU-IPADS, 2024)
- **Approach:** Neuron-level offloading — predictor determines which neurons will fire, only loads those.
- **For VITRIOL:** More granular than expert-level offloading. Interesting future direction.
- **GitHub:** https://github.com/SJTU-IPADS/PowerInfer

### KTransformers (kvcache-ai, 2024)
- **Approach:** YAML-based layer placement, double-buffer prefetch pattern.
- **For VITRIOL:** Informed predictive prefetching design.
- **GitHub:** https://github.com/kvcache-ai/KTransformers

---

## 7. Bibliography

### Papers

| # | Citation | Link |
|---|----------|------|
| 1 | Leviathan et al. "Fast Inference from Transformers via Predictive Sampling." Google Research, 2022. | https://arxiv.org/abs/2211.17192 |
| 2 | Chen et al. "Accelerating Large Language Model Decoding with Speculative Sampling." DeepMind, 2023. | https://arxiv.org/abs/2302.01318 |
| 3 | Zhang, Sheng et al. "H2O: Heavy-Hitter Oracle for Efficient Generative Inference of Large Language Models." 2023. | https://arxiv.org/abs/2306.14048 |
| 4 | Xiao et al. "Efficient Streaming Language Models with Attention Sinks." 2023. | https://arxiv.org/abs/2309.17453 |
| 5 | Kwon et al. "Efficient Memory Management for Large Language Model Serving with PagedAttention." vLLM, 2023. | https://arxiv.org/abs/2309.06180 |
| 6 | Alizadeh, Mirzadeh et al. "LLM in a Flash: Efficient Large Language Model Inference with Limited Memory." Apple, 2023. | https://arxiv.org/abs/2312.11514 |
| 7 | Kim, Fahim, Awadalla. "Mixture of Quantized Experts (MoQE)." Microsoft, 2023. | https://arxiv.org/abs/2310.14713 |
| 8 | Cai et al. "Medusa: Simple LLM Inference Acceleration Service with Multiple Decoding Heads." 2024. | https://arxiv.org/abs/2401.10774 |
| 9 | Li et al. "EAGLE: Speculative Decoding Can Be Iron-Fast." 2024. | https://arxiv.org/abs/2401.15077 |
| 10 | Kamahori, Gu, Zhu, Kasikci. "Fiddler: CPU-GPU Orchestration for Fast Inference of Mixture-of-Experts Models." 2024. | https://arxiv.org/abs/2402.14103 |
| 11 | Ma, Wang et al. "The Era of 1-bit LLMs: All Large Language Models are in 1.58 Bits." Microsoft Research, 2024. | https://arxiv.org/abs/2402.17764 |
| 12 | Li et al. "SnapKV: LLM Knows What You are Looking for Before Generation." 2024. | https://arxiv.org/abs/2404.14469 |
| 13 | Edge, Trinh et al. "From Local to Global: A Graph RAG Approach to Query-Focused Summarization." Microsoft, 2024. | https://arxiv.org/abs/2404.16130 |
| 14 | Liu et al. "KIVI: 2-bit KV Cache Quantization." 2024. | https://arxiv.org/abs/2402.02750 |
| 15 | Umang. "Prompt Lookup Decoding." 2024. | https://github.com/apoorvumang/prompt-lookup-decoding |
| 16 | "Self-Speculative Decoding: Improving LLM Inference via Speedup and Sparse-to-Dense." 2023. | https://arxiv.org/abs/2307.13304 |
| 17 | "Mixture of Speculative Experts: High-throughput speculative decoding." 2024. | https://arxiv.org/abs/2402.13524 |

### Projects & Repositories

| # | Project | Link |
|---|---------|------|
| 1 | llama.cpp (ggml-org) | https://github.com/ggml-org/llama.cpp |
| 2 | AirLLM (lyogavin) | https://github.com/lyogavin/airllm |
| 3 | KTransformers (kvcache-ai) | https://github.com/kvcache-ai/KTransformers |
| 4 | PowerInfer (SJTU-IPADS) | https://github.com/SJTU-IPADS/PowerInfer |
| 5 | T-MAC (Microsoft) | https://github.com/microsoft/T-MAC |
| 6 | Aider (Paul Gauthier) | https://github.com/paul-gauthier/aider |
| 7 | Medusa (FasterDecoding) | https://github.com/FasterDecoding/Medusa |
| 8 | EAGLE (SafeAILab) | https://github.com/SafeAILab/EAGLE |
| 9 | ik_llama.cpp (ikawrakow) | https://github.com/ikawrakow/ik_llama.cpp |
| 10 | turbo-tan/llama.cpp-tq3 | https://github.com/turbo-tan/llama.cpp-tq3 |
| 11 | mad-lab-ai TQ GGUF | https://huggingface.co/mad-lab-ai/Qwen3.6-35B-A3B-tq-gguf |
| 12 | Unsloth (Daniel & Michael) | https://huggingface.co/unsloth |
| 13 | 3LTERN (ELX987) | https://github.com/ELX987/3LTERN |
| 14 | DocShotgun MoE offload guide | https://huggingface.co/blog/Doctor-Shotgun/llamacpp-moe-offload-guide |
| 15 | MemGPT | https://arxiv.org/abs/2310.08560 |

---

## Appendix: Git Log

```
6c415b6 docs: add AGENT_BRIEF.md with full optimization catalog + extend README citations
84a4079 phase3: add KV quant, prompt lookup, engine-mode CLI flags + TUI
33c5bba fix: move _store_turn to background thread
005a8dd db: increase busy_timeout to 30s
41668c7 fix: SSE streaming proxy + write mutex
4368432 fix: DB lock contention
02a047b fix: strip stream flag from forwarded requests, graceful archive check
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

*Generated: 2026-05-18 09:00 CEST*
