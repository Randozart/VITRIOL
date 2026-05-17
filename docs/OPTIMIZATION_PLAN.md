# VITRIOL V2: Optimization Roadmap

> **Date**: 2026-05-17
> **Status**: Planning — RAM Shot + LRU CE DMA cache operational (6.9 tok/s)
> **Target**: Multi-layer heterogeneous engine with modular modes, scalable from 8 GB VRAM to 24 GB+
> **See also**: `ROADMAP.md`, `CONTEXT_OFFLOADING_STRATEGIES.md`, `VECTOR_DB_IMPLEMENTATION.md`,
>   `OPTIMIZATION_PLAN_V1.md` (preserved predecessor)
>
> **Prior art**: All optimizations below are grounded in published research. See [Citation Index](#citation-index)
> for full bibliography. These will be moved to a README `ars priori` section once implemented.

---

## Table of Contents

1. [Citation Index](#citation-index)
2. [Current Baseline](#current-baseline)
3. [Layer 1: The Heterogeneous Engine](#layer-1-the-heterogeneous-engine)
4. [Layer 2: Context & VRAM Management](#layer-2-context--vram-management)
5. [Layer 3: The Cognitive Orchestrator](#layer-3-the-cognitive-orchestrator)
6. [Layer 4: System Stability](#layer-4-system-stability)
7. [Modular CLI Modes](#modular-cli-modes)
8. [Implementation Roadmap](#implementation-roadmap)

---

## Citation Index

All optimizations in this document are grounded in published research and proven open-source projects.
When a feature is implemented, its citation is moved to the README's `ars priori` section.

| Paper / Project | Abbrev. | Year | ArXiv / Link | Contribution | Applied In |
|-----------------|---------|------|--------------|-------------|------------|
| **LLM in a Flash** — Alizadeh, Mirzadeh et al. (Apple) | Flash | 2023 | [`2312.11514`](https://arxiv.org/abs/2312.11514) | Proved windowing + zero-copy streaming from flash/host memory enables LLM inference on severely memory-limited hardware | Layer 1 — RAM Shot base |
| **Fiddler** — Kamahori, Gu, Zhu, Kasikci | Fiddler | 2024 | [`2402.14103`](https://arxiv.org/abs/2402.14103) | Demonstrated moving *activations* to CPU for MoE expert computation is faster than pulling weights to GPU via PCIe DMA | Layer 1 — `fiddler-cpu` mode |
| **MoQE** — Kim, Fahim, Awadalla (Microsoft) | MoQE | 2023 | [`2310.14713`](https://arxiv.org/abs/2310.14713) | Showed MoE experts robust to extreme low-bit quantization (2-bit) without losing base model coherence | Layer 1 — asymmetric quantization |
| **BitNet b1.58** — Ma, Wang et al. (Microsoft Research) | BitNet | 2024 | [`2402.17764`](https://arxiv.org/abs/2402.17764) | Proved ternary weights {-1, 0, 1} match FP16 perplexity, eliminating need for floating-point multiply. CPU integer addition only. | Layer 1 — TQ1_0 format support |
| **SnapKV** — Li et al. | SnapKV | 2024 | [`2404.14469`](https://arxiv.org/abs/2404.14469) | Demonstrated attention heads focus on clustered features; safe eviction of filler tokens reduces KV cache 8.2x without accuracy loss | Layer 2 — `--kv-mode sparse` |
| **H2O** — Zhang, Sheng et al. | H2O | 2023 | [`2306.14048`](https://arxiv.org/abs/2306.14048) | Pioneered dropping tokens from KV cache by identifying "Heavy Hitter" tokens that contribute most to attention scores | Layer 2 — `--kv-mode sparse` |
| **GraphRAG** — Edge, Trinh et al. (Microsoft) | GraphRAG | 2024 | [`2404.16130`](https://arxiv.org/abs/2404.16130) | Replaced flat vector DBs with LLM-derived knowledge graphs for multi-hop retrieval (spreading activation) | Layer 3 — cascading retrieval |
| **Aider** — Paul Gauthier | Aider | 2023 | [github.com/paul-gauthier/aider](https://github.com/paul-gauthier/aider) | Gold standard for tree-sitter AST-based repo mapping; intelligently formatting dependency relationships for limited context windows | Layer 3 — AST code graphing |

### Target Models

| Model | Family | MoE Config | VRAM (Q2) | Relevance |
|-------|--------|-----------|-----------|-----------|
| **Qwen3.6-35B-A3B** | Qwen | 256 experts, 8 active | 11.44 GiB | ✅ Currently running at 6.9 tok/s. Primary near-term target. |
| **Qwen3.6-72B-A3B** | Qwen | 256 experts, 8 active | ~22 GiB | Future target. Will require fiddler or stronger quantization. |
| **DeepSeek V4 Flash** | DeepSeek | Hyper-sparse MoE + Latent Attention | TBD | Bleeding-edge 2026 architecture. Targets KV cache efficiency natively. |

---

## Current Baseline

Measured on: **GTX 1070 Ti (8 GB)**, **i7-3770**, **PCIe 3.0 x16**, **Qwen3.6-35B-A3B-UD-Q2_K_XL (11.44 GiB)**

| Metric | RAM Shot (VITRIOL) | LRU CE DMA (VITRIOL) | All-VRAM (baseline, partial) |
|--------|-------------------|----------------------|------------------------------|
| **Text generation** | 6.31 tok/s | **6.9 tok/s** | 6.52 tok/s* |
| **Prompt eval** | 33.86 tok/s | 22.4 tok/s | 4.89 tok/s |
| **VRAM used** | 1.3 GiB (model only) | ~2.0 GiB (model + 512 MB LRU) | ~7.5 GiB |
| **System RAM used** | +10 GiB (experts) | +10 GiB (experts) | 0 GiB |
| **Model load time** | ~64 s | ~64 s | ~30 s |
| **Graph splits** | 17 | 17 | 2 |
| **Sched copies** | 4 | 4 | 0 |

*\*Baseline established with partial model that fit in VRAM; full model does not fit at all without VITRIOL.*

### Profiling Breakdown

From `OPTIMIZATION_PLAN_V1.md` (verified profiling, 2026-05-12):

| Operation | Per-Token Time | % of Total |
|-----------|---------------|------------|
| **Expert loading** (PCIe DMA) | ~100–120 ms | **60–70%** |
| Attention compute (GPU) | ~30–40 ms | 20–25% |
| Expert compute (GPU) | ~20–30 ms | 10–15% |
| Router + overhead | ~10 ms | 5% |
| **Total** | **~160–200 ms** | **100%** |

GPU utilization during 35B inference: **15–18%** — the GPU is idle >80% of the time waiting for expert data over PCIe.

### Key Insight

Data movement, not compute, is the bottleneck. Every optimization below targets either:
- **Reducing the data moved** (quantization, sparse KV, signature compaction)
- **Hiding the data movement latency** (predictive prefetching, activation offloading)
- **Avoiding unnecessary data movement** (frozen prompt caching, Hebbian filtering)

---

## Layer 1: The Heterogeneous Engine

*Physical execution of the model — splitting the workload between GPU and CPU.*

### 1.1 `vitriol-dma` (Existing — RAM Shot + LRU CE DMA Cache)

**Status**: ✅ Operational at 6.9 tok/s on GTX 1070 Ti (8 GB) with Qwen3.6-35B-A3B.

**How it works**:
1. **Allocation**: Expert weights stored in page-locked host RAM via three-step pinning:
   `mmap` → `madvise(MADV_HUGEPAGE)` → `mlock` → `cudaHostRegister`
2. **Scheduler hook**: Buffer reports `is_host=true`, triggering llama.cpp's intelligent MoE offload path. GPU reads expert weights over PCIe DMA transparently during MUL_MAT_ID.
3. **LRU VRAM cache**: A small VRAM pool (default 512 MB, configurable via `VITRIOL_LRU_MB`) keeps hot experts at native GDDR5 speed. `cuMemcpyHtoDAsync` copies from host to VRAM on dedicated LRU stream with `cuStreamWaitEvent` sync.
4. **Fallback**: On cache miss, GPU reads directly from page-locked host RAM via PCIe DMA — no stall.

**Configuration**:
| Env/Flag | Description | Default |
|----------|-------------|---------|
| `VITRIOL_MODE=stream` | Enable RAM Shot + LRU cache | `off` |
| `VITRIOL_LRU_MB=512` | VRAM pool size for hot experts | 512 |
| `VITRIOL_VERBOSE=1` | Log cache hits/misses/evictions | 0 |
| `-ngl 99` | Offload all layers (experts land in VITRIOL buffer) | — |

**Fast-path note**: Generation uses MMVQ (batch <= 8) which reads experts directly from page-locked host RAM. The LRU pointer swap only applies on the cuBLAS slow path. PCIe overhead for small-batch reads is negligible.

### 1.2 `fiddler-cpu` (Planned — Activation Offloading)

**Status**: 🟡 Planned — requires Ternary-quantized MoE experts (TQ1_0) and ggml graph routing.

**How it works** (per [Fiddler, Kamahori et al. 2024](https://arxiv.org/abs/2402.14103)):
1. MoE expert weights remain permanently in system RAM (no VRAM pressure)
2. GPU computes attention layers as normal (VRAM-resident)
3. After attention, GPU ships the **activation vector** (≈2 MB) across PCIe to CPU
4. CPU processes the MoE expert using AVX-512 / SIMD integer addition (ternary: -1, 0, 1 — no floating-point multiply)
5. CPU ships the mutated activation vector back to GPU for next attention layer

**Why it wins**: 2 MB activations vs 1.5 GB weights across PCIe. Transforms PCIe from the bottleneck into negligible overhead.

**PCIe bandwidth comparison**:

| Approach | Per-Expert Transfer | BW Utilization | Time |
|----------|-------------------|----------------|------|
| vitriol-dma (current) | 1.5 GB weights | 12 GB/s (PCIe 3.0) | ~125 ms |
| fiddler-cpu (planned) | 2 MB activations × 2 | 12 GB/s | ~0.33 ms |
| **Speedup** | **~750x less data** | — | **~380x faster** |

**Implementation plan**:
1. Add TQ1_0 GGUF quantization support (see §1.3)
2. Register a CPU backend in ggml for expert node execution
3. Modify ggml graph evaluation to split: `Attention → Backend_CUDA`, `MoE Expert → Backend_CPU`
4. Insert CUDA-to-Host / Host-to-CUDA memory copies at graph boundaries
5. Bind CPU expert threads to isolated cores with `nice` scheduling (see Layer 4)

### 1.3 Ternary Quantization TQ1_0 (Planned — BitNet b1.58 Format)

**Status**: 🟡 Planned — requires GGUF spec extension and ggml CPU kernels.

**Background** (per [BitNet b1.58, Ma et al. 2024](https://arxiv.org/abs/2402.17764)):
- Weights constrained to {-1, 0, 1} — 1.58 bits per parameter
- Eliminates need for floating-point multiplication entirely
- Matches FP16 perplexity when trained from scratch
- 35B model at TQ1_0: ≈6 GB — fits in most system RAM configurations

**Why it matters for VITRIOL**:
- Ternary experts can be processed on CPU using integer addition only
- No GPU tensor cores needed for expert math
- CPU's AVX-512 can process hundreds of additions/clock — ideal for this workload
- Combined with Fiddler: GPU does attention, CPU does experts, PCIe carries only activations

**Implementation**:
1. Extend GGUF format with `GGML_TYPE_TQ1_0` (ternary, 1.58-bit)
2. Write `ggml` CPU kernels using bit-packed AVX-512 integer addition
3. Write PTQ (Post-Training Quantization) script to convert existing FP16/Q4 MoE → TQ1_0
4. Validate perplexity and accuracy trade-off

### 1.4 Predictive Prefetching (Planned — Lookahead Expert Routing)

**Status**: 🟡 Planned — leverages existing async CUDA stream infrastructure.

**How it works**:
- Current behavior (sequential): `Attention N` → `Router` → `Load Expert N` → `Compute Expert N`
- Target behavior (overlapped): `Attention N` + `Load Expert N+1` (concurrent)

**Key insight**: MoE routing is highly predictable from Layer N-1's output. A tiny linear predictor (one matrix multiply on the router input) can guess which experts Layer N will activate before attention finishes.

**Implementation**:
1. Store the router's hidden state from the previous layer
2. Train/calibrate a lightweight linear probe (≈1K parameters) that maps previous-layer router output → expert IDs
3. Before attention begins: run predictor → trigger `cuMemcpyHtoDAsync` for predicted experts on LRU stream
4. After attention finishes: router confirms actual experts; any misses are fetched synchronously (rare)
5. Entire DMA latency hidden behind attention compute

**Expected impact**: +20–40% tok/s — GPU never waits for expert data.

### 1.5 Alka NVMe→GPU DMA (Existing Infrastructure)

**Status**: ✅ Pipeline complete. Alka stream generation + executor + kernel module operational.
See `OPTIMIZATION_PLAN_V1.md` and `docs/ALKA_EXECUTOR_DESIGN.md`.

**What exists**:
- `vitriol.ko` v0.2 with Alka ABI (0xA1 magic) — 5 IOCTLs, 11 opcode handlers
- `gguf-offset-resolver` — Parses GGUF v3, extracts real tensor offsets (733 tensors)
- `generate-alka-recipe.sh` — Generates `.alka` from GGUF + `.alkavl` hardware vial
- `alka-executor` — Validates and executes `.alkas` streams via `/dev/vitriol`

**Future**: Replace staged copy with direct NVMe→GPU DMA (`blkdev_direct_read()`) for zero-copy tensor streaming.

### 1.6 Double-Buffer Expert Prefetch (From V1 Plan)

**Status**: ⏳ Related to Predictive Prefetching (§1.4). Original V1 concept preserved here.

**Original V1 approach**:
```
Token N:  [load experts N] → [compute N] → [done]
Token N+1:                [load experts N+1] → [compute N+1] → [done]
                              ↑ happens during compute N (overlap)
```

The Predictive Prefetching approach (§1.4) supersedes this by also predicting *which* experts to load — not just overlapping blindly.

### 1.7 Speculative Decoding with GTX 960 (From V1 Plan)

**Status**: ⏳ BLOCKED — requires compatible draft model (e.g., Qwen2.5-0.5B Q4_K_M).

**Architecture**:
```
GTX 960 (2 GB)          GTX 1070 Ti (8 GB)
────────────────        ─────────────────
Qwen 0.5B draft         Qwen3.6-35B-A3B target
~500 MB → fits VRAM     ~775 MB base + expert swap
100+ tok/s              6.9 tok/s
     │                        ▲
     │   P2P DMA via PCIe     │
     │   (8 tokens guessed)   │
     └────────────────────────┘
```

**Expected impact**: If acceptance rate = 60%: effective tok/s = 6.9 × (1 + 8 × 0.6) = **~40 tok/s**.

---

## Layer 2: Context & VRAM Management

*Ensuring the context window doesn't OOM on 8 GB VRAM or trigger the PCIe prefill penalty.*

### 2.1 Frozen Prompt Caching (Near-term win)

**Status**: 🟡 Planned — requires modifying how the client builds prompts.

**The problem**: A 20K token context prefill at 20 tok/s over PCIe = **~1000 seconds (16+ minutes)** waiting for the first token. This makes large-context inference unusable.

**The solution**: Split the context window into two strict zones:

| Zone | Tokens | Content | Update Frequency | Cache Behavior |
|------|--------|---------|-----------------|----------------|
| **Zone 1 (Static)** | 0–18,000 | Repo map, AST graph, project rules | Only on explicit `/reindex` | **Frozen** — llama.cpp KV cache never invalidated |
| **Zone 2 (Active)** | 18,001–20,000 | User prompt, AI response, recent history | Every turn | Re-evaluated normally |

**Implementation**:
1. In the client (Python shim or Rust orchestrator), build Zone 1 once and keep it at the top of every prompt
2. Only append/change Zone 2 between turns
3. llama.cpp's KV cache shifting recognizes the prefix as unchanged and skips re-evaluation
4. Prefill time drops from 16 minutes to ≈1 minute (only 2K new tokens processed)

**The KV cache trap**: If any agent (Aider, OpenCode) changes a single character in the prefix, the entire 18K KV cache is invalidated. This is why the client must be modified to **never** mutate Zone 1 automatically.

### 2.2 Zero-Copy KV Cache Offloading (Near-term win)

**Status**: 🟡 Planned — reuses RAM Shot infrastructure.

**How it works**:
- Apply the same `mlock` + `cudaHostRegister` pattern from RAM Shot to the KV cache
- Keep newest N tokens (e.g., 2K–4K) in VRAM for fast attention
- Page-lock older KV blocks (tokens N+1 → M) into system RAM
- On deep-sequence attention (where query attends to old tokens), stream those KV blocks back to VRAM over PCIe DMA
- LRU policy determines which blocks stay hot in VRAM

**VRAM budget breakdown** (8 GB card, 35B MoE at 5–10 MB/token KV):

| Component | VRAM | Tokens Supported |
|-----------|------|-----------------|
| Model base (q2_K_XL) | 1.3 GiB | — |
| LRU expert cache | 0.5 GiB | — |
| Compute buffers | 1.0 GiB | — |
| **Subtotal** | **2.8 GiB** | — |
| Remaining for KV cache | 5.2 GiB | **~500–1000 tokens** (no offload) |
| With offload (all KV in system RAM) | 0.5 GiB (hot) | **20,000+ tokens** |

**Implementation**:
1. In `llama-kv-cache.cpp`, add a tiering layer that intercepts KV cell allocation
2. When VRAM usage exceeds threshold, migrate oldest cells to page-locked system RAM
3. On attention to an offloaded cell, DMA it back to VRAM (async, with prefetch for sequential access)
4. Existing `vitriol-cuda-integration.{cpp,h}` provides the page-locking primitives

### 2.3 Sparse KV Caching — SnapKV / H2O (Near-term win)

**Status**: 🟡 Planned — attention-score monitoring in `llama-kv-cache.cpp`.

**Background** (per [SnapKV 2024](https://arxiv.org/abs/2404.14469), [H2O 2023](https://arxiv.org/abs/2306.14048)):
- In a 10K-token prompt, ≈95% of words are "filler" (articles, prepositions, boilerplate)
- The model only attends to ≈5% of tokens — these are "Heavy Hitters"
- Attention scores are predictive: tokens with low scores in early layers will have low scores in all layers

**How it works**:
1. After each attention layer, record the cumulative attention score per token
2. Maintain a priority queue of tokens sorted by attention score
3. When KV cache is full: evict the lowest-scoring tokens
4. Always preserve "attention sinks" (first few tokens — they anchor positional encoding)
5. Store evicted tokens' metadata separately (they can be re-fetched from the raw prompt if needed)

**Expected compression**: 4–8x KV cache reduction with <1% perplexity loss.

**Comparison with offload** (§2.2):

| Approach | Mechanism | Latency Impact | Max Context (8 GB) |
|----------|-----------|---------------|-------------------|
| Offload only | Move to system RAM | ~3–5 ms DMA fetch | ~20K tokens |
| Sparse only | Evict from cache | 0 ms (no fetch) | ~8–16K equiv. |
| **Offload + Sparse** | Both | Depends on hit rate | **32K+ equiv.** |

### 2.4 Python Shim with RAG (Existing — `vitriol_shim.py`)

**Status**: ✅ Operational — serves as the application-layer context manager.

**What it does**:
- Sits between OpenCode/Aider and llama.cpp as a Flask proxy on port 5010
- Implements context "rectification" (alchemical terminology for compression):
  - **Calcination**: Truncate to last N messages (default: 4)
  - **Sublimation**: Strip `<reasoning>` blocks, tool calls, metadata (80–95% token reduction)
  - **Streaming**: RAG retrieval from SSD archive
  - **Coagulation**: Enforce max_tokens cap
- Archives old messages to JSONL on SSD
- FAISS-based vector search (`IndexFlatIP`) for relevance retrieval
- GPU thermal guardrail (halts at 85°C)

**Enhancement planned**: Upgrade keyword scoring to `sentence-transformers` (all-MiniLM-L6-v2) for semantic search.

---

## Layer 3: The Cognitive Orchestrator

*A new Rust CLI tool that sits between the user and the inference engine. Manages memory, context, and retrieval.*

### 3.1 State-Bound `.vitriol/` Directory

**Design**: Git-style hidden directory in each project root.

```
project_root/
├── .vitriol/
│   ├── config.toml      # Per-project engine/kv/retrieval mode preferences
│   ├── state.db          # SQLite — synaptic weights, episodic memory, graph metadata
│   ├── ast.msgpack       # Serialized tree-sitter AST
│   └── archive/          # Archived message chunks
├── src/
├── Cargo.toml
└── ...
```

**State isolation**: The CLI simply checks for `./.vitriol/` on startup. No global DB, no fuzzy matching, no cross-project pollution. Memory travels with the project folder.

**OpenCode integration**: OpenCode supports custom HTTP headers in API calls. The orchestrator (or shim) reads `X-Project-Id: <project_name>` from incoming requests to select the correct `.vitriol/` database — no URL or model-name hacks needed:

```
POST /v1/chat/completions
X-Project-Id: vitriol-engine
X-Session-Id: session_abc123
```

The shim routes the request to the `.vitriol/` state directory matching the `X-Project-Id` header.

### 3.2 AST-Aware Code Graphing

Using Rust's first-class `tree-sitter` bindings to parse the working directory into a dependency graph.

| Element | Node Type | Edge Type |
|---------|-----------|-----------|
| Function definition | `function` | `calls`, `called_by` |
| Struct / trait | `type` | `uses`, `used_by` |
| Import / include | `import` | `depends_on` |
| Module / file | `module` | `contains` |

Graph stored in `petgraph::DiGraph` with string internment for node labels. Serialized to MessagePack for fast loading on subsequent invocations.

### 3.3 Cascading Multi-Hop Retrieval (GraphRAG)

**How it works** (per [GraphRAG, Edge et al. 2024](https://arxiv.org/abs/2404.16130)):

```
User: "Fix the null pointer in connection_handler"

Hop 1: Vector search → finds connection_handler() node
  ↓
Hop 2: Graph traversal → follows "calls" edge → finds socket_connect() and buffer_alloc()
  ↓
Hop 3: Follows "uses" edge → finds BufferPool struct and its methods
  ↓
Result: [connection_handler, socket_connect, buffer_alloc, BufferPool, BufferPool::alloc]
         ↳ Injected into context window as a coherent dependency slice
```

**Why it beats flat RAG**: Code is inherently a graph (dependencies, inheritance, FFI calls). Flat vector search cannot model topology.

### 3.4 Token-Budgeted Signature Compaction

**How it works**:
1. Define a strict prefill budget (configurable, e.g., 1500 tokens)
2. Retrieve nodes from the cascade graph in priority order
3. Run each through `tiktoken-rs` to count tokens
4. If adding the full node would exceed budget: inject only signature + docstring
   - `fn socket_connect(addr: SocketAddr) -> Result<TcpStream> — establishes TCP connection with timeout`
5. Priority based on:
   - Node centrality in dependency graph (PageRank-like)
   - Hebbian edge weights (see §3.5)
   - Recency of edit (from git log)

### 3.5 Synaptic Weighting (Hebbian Learning)

**How it works**:
- Every edge in the AST graph has a weight (float, initialized to 1.0)
- After each LLM response, check if the cascaded memory was referenced by the AI
  - If yes: **increase** edge weight (+0.1, "neurons that fire together wire together")
  - If no: **decrease** edge weight (−0.05)
- Over time, the graph learns which files are frequently edited together
- Weights persist in SQLite across sessions

### 3.6 Memory Consolidation ("Sleep")

**How it works**:
1. Background thread runs when GPU is idle (low priority, cores 2–7)
2. Reads last N raw episodic memories from SQLite (e.g., *"User asked about null pointer"*, *"User refactored socket code"*)
3. Feeds to a tiny CPU model (Qwen 0.5B) for summarization: *"User was debugging network connection issues in the socket layer"*
4. Creates a dense "consolidated node" in the graph — connected to all source nodes
5. Deletes the granular raw memories to save space

**Analogy**: Human sleep consolidation — short-term episodic memories → long-term semantic knowledge.

---

## Layer 4: System Stability

*Ensuring the PC remains usable while the CPU processes ternary math.*

### 4.1 Thread Affinity (Core Pinning)

- **Reserve cores 0–1** for OS interrupts, mouse polling, UI rendering, IDE responsiveness
- **Pin ternary math threads** to cores 2–N using Rust's `core_affinity` crate (Linux) or `SetThreadAffinityMask` (Windows)
- Prevent `ggml` from guessing thread count and saturating all cores
- Configurable via `--sys-priority background` (reserve cores) vs `dedicated` (all cores)

### 4.2 Priority Scheduling

- Set inference threads to `nice +5` (Linux) or `BELOW_NORMAL_PRIORITY_CLASS` (Windows)
- User keystrokes and IDE scrolling preempt AI math instantly
- Zero perceived system lag — cursor never stutters

---

## Modular CLI Modes

VITRIOL V2 exposes toggleable modes so the same binary scales across any hardware profile.

### `--engine-mode`

| Mode | Description | Quantization | Data Flow | Hardware Target |
|------|-------------|-------------|-----------|-----------------|
| `vitriol-dma` | RAM Shot + LRU CE DMA cache. Experts in host RAM, GPU reads via PCIe DMA. | Q4, Q6, FP16 | GPU ←PCIe DMA← Host RAM | 8 GB VRAM |
| `fiddler-cpu` | Activation offloading. GPU does attention, CPU does ternary expert math. | TQ1_0 (ternary) | GPU →2MB→ CPU →2MB→ GPU | 8 GB VRAM |
| `native` | Standard llama.cpp, no offloading. All in VRAM. | Any | GPU only (VRAM) | 24 GB+ VRAM / Apple Silicon |

### `--kv-mode`

| Mode | Description | Context Supported |
|------|-------------|------------------|
| `standard` | Default llama.cpp prompt caching. Re-evaluates only new tokens. | 512–2K tokens |
| `offload` | Zero-copy KV cache to system RAM via `cudaHostRegister`. | 20K+ tokens |
| `sparse` | SnapKV/H2O attention-guided token eviction. | 32K+ equiv. |

### `--retrieval-mode`

| Mode | Description | Prefill Cost | Best For |
|------|-------------|-------------|----------|
| `budget` | Strict AST signature compaction. Hard token limit. | Low (fast) | PCIe-bottlenecked systems |
| `unbound` | Full file injection into context. | High (slow) | Mac / cloud GPU (fast prefill) |
| `flat` | Disable cascading. Standard vector RAG. | Medium | Simple Q&A on markdown/docs |

### `--sys-priority`

| Mode | Core Pinning | OS Priority | Best For |
|------|-------------|-------------|----------|
| `background` | Reserve cores 0–1 for OS/UI | `nice +5` | Developer workstation (VS Code + terminal) |
| `dedicated` | All cores available | `nice 0` | Headless server via SSH/API |

### Example Usage

```bash
# 8 GB GPU, ternary model, dev workstation
vitriol run --engine-mode fiddler-cpu --kv-mode sparse --retrieval-mode budget --sys-priority background

# 24 GB GPU, standard model, headless server
vitriol run --engine-mode native --kv-mode standard --retrieval-mode unbound --sys-priority dedicated

# Per-project config persisted
vitriol init --engine-mode fiddler-cpu --kv-mode offload --sys-priority background
# => writes .vitriol/config.toml
```

---

## Implementation Roadmap

### Phase 1: Context Wins (Top Priority)

**Goal**: Larger usable context window with existing Qwen3.6-35B-A3B Q2_K_XL model.
**No new quantization formats required** — works with current RAM Shot infrastructure.

| # | Task | Files to Modify | Est. Effort | Depends On |
|---|------|----------------|-------------|------------|
| 1 | **Zero-Copy KV Cache Offload** — replicate `mlock`+`cudaHostRegister` pattern for KV cache cells | `llama-kv-cache.cpp`, `vitriol-cuda-integration.{cpp,h}` | 1–2 sessions | — |
| 2 | **Sparse KV Caching (SnapKV)** — attention-score monitoring, low-attention eviction | `llama-kv-cache.cpp`, `llama-graph.cpp` | 2–3 sessions | — |
| 3 | **Frozen Prompt Caching** — split context into static + active zone in Python shim | `vitriol_shim.py`, client config | 1 session | — |
| 4 | **Python shim enhancement** — upgrade to sentence-transformers semantic search | `vitriol_shim.py`, `vector_store.py` | 1 session | — |

### Phase 2: Engine Optimization

| # | Task | Files to Modify | Est. Effort | Depends On |
|---|------|----------------|-------------|------------|
| 5 | **Predictive Prefetching** — linear expert predictor + async DMA | `ggml-cuda.cu`, `vitriol-cuda-integration.cpp` | 2–3 sessions | Existing LRU stream infra |
| 6 | **Graph Split Optimization** (ROADMAP Phase 4) — reduce 17→2 splits | `vitriol-buffer.cpp`, `ggml-cuda.cu` | 1 session | — |

### Phase 3: Rust Cognitive Orchestrator

| # | Task | Est. Effort | Depends On |
|---|------|-------------|------------|
| 7 | `.vitriol/` state directory + `clap` CLI scaffold | 1 session | — |
| 8 | tree-sitter AST graph (`petgraph`) | 2 sessions | — |
| 9 | Cascading multi-hop retrieval | 2 sessions | §8 |
| 10 | Token-budgeted compaction + Hebbian weighting | 2 sessions | §9 |
| 11 | Memory consolidation (background summarization via tiny CPU model) | 2 sessions | §7 |

### Phase 4: Heterogeneous Compute (Requires Ternary Models)

| # | Task | Est. Effort | Depends On |
|---|------|-------------|------------|
| 12 | TQ1_0 GGUF format + ggml type registration | 3–5 sessions | — |
| 13 | AVX-512 / SIMD CPU kernels for ternary expert math | 3–5 sessions | §12 |
| 14 | ggml graph split: Attention→CUDA, Expert→CPU | 2–3 sessions | §12, §13 |
| 15 | Thread affinity + priority scheduling | 1 session | — |
| 16 | Speculative decoding with GTX 960 | 3–5 sessions | Draft model available |

### Dependency Graph

```
Phase 1                    Phase 2              Phase 3              Phase 4
─────────                  ─────────             ─────────             ─────────
KV Offload ──────┐
                  ├──> Predictive Prefetch ──┐
Sparse KV ───────┘                          ├──> Graph Split ───┐
                                            │                    │
Frozen Prompt ──┐                           │                    ├──> Fiddler CPU
                 ├──> Shim Enhancement ─────┘                    │       │
Shim RAG ───────┘                                                │       ├── TQ1_0
                                                                 │       └── AVX-512 Kernels
                                                                 │
. vitriol Dir ──┐                                                │
                 ├──> AST Graph ──> Cascade ──> Compaction ─────┘
Rust Library ───┘       └──> Hebbian ──> Consolidation
```

---

## README `ars priori` Section (Template)

Once a feature is implemented, add its citation to the README:

```markdown
## Ars Priori

VITRIOL stands on the shoulders of giants. The following research directly influenced
our architecture:

- **LLM in a Flash** (Alizadeh et al., Apple, 2023) — Windowing and zero-copy streaming
  from host memory enables inference on VRAM-limited hardware. [arXiv:2312.11514]
- **Fiddler** (Kamahori et al., 2024) — Activation offloading for MoE models;
  moving activations to CPU outperforms moving weights over PCIe. [arXiv:2402.14103]
- **BitNet b1.58** (Ma et al., Microsoft, 2024) — Ternary weights {-1, 0, 1} match
  FP16 performance with zero floating-point multiplication. [arXiv:2402.17764]
- **SnapKV** (Li et al., 2024) — Attention-guided KV cache compression achieves
  8.2x reduction without accuracy loss. [arXiv:2404.14469]
- **H2O** (Zhang, Sheng et al., 2023) — Heavy-Hitter Oracle for efficient KV cache
  eviction. [arXiv:2306.14048]
- **GraphRAG** (Edge, Trinh et al., Microsoft, 2024) — Multi-hop retrieval via
  knowledge graphs for interconnected datasets. [arXiv:2404.16130]
```

---

*Last updated: 2026-05-17 18:00 CEST*
*See also: `ROADMAP.md` (Phase 4–7 implementation status), `OPTIMIZATION_PLAN_V1.md` (preserved predecessor)*
