# VITRIOL

<img src="assets/vitriol_logo.svg" alt="VITRIOL" width="200"/>

*"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*

*(Visit the Interior of the Earth, by Rectifying you will find the Hidden Stone)*

## Why VITRIOL?

VITRIOL is my attempt at using every optimization possible to run modern AI models on old hardware that would have no business running them otherwise. I am talking here specifically about my old desktop PC with a narrow PCIe bus, an outdated GPU, a CPU without AVX2 instructions, so offloading isn't even practical. With those constraints in mind I had a simple question: Could I stream directly from system RAM to GPU. It took a while, but I managed through a solid direct memory access system and a custom copy engine. Every optimization I am doing now is really one big mad computer science experiment to make old hardware punch above its weight, and possibly introduce those optimizations for newer hardware. This means VITRIOL is not a stable repo in the slighest. Not yet at least. I am trying to use the latest papers and research on LLM inference to squeeze every bit of performance out of my silicon, and I hope this comes to your benefit as well.

Regarding the name, in alchemy, vitriol was considered the ultimate catalyst for transmuting matter. This is something this application aims to do as well. To transform old hardware into a high performant AI transformer. Lead into gold. You know the drill. Aditionally, around the 15th century, the esoteric backcronym was formed: *"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*. In a way, this is what we are doing. We are reaching down into the bowels of the computer, rectifying the data streams, and running inference on our newfound philosopher's stone. Yes, I know how incredibly silly this all sounds, but it makes me happy to be using archaic alchemical terminology. Regarding the logo: In alchemical texts and artwork, vitriol was often depicted as a "Green Lion devouring the Sun". This is a metaphor for sulfuric acid dissolving base metals (symbolically represented by the green lion) to extract and purify precious gold (the alchemical symbol for which is the sun).

## ⚗️ Officina — the built-in coding workshop

VITRIOL is not just an inference engine — it ships **Officina**, a
full-screen agentic programming environment that is *detached-but-coupled*:
it runs as its own workspace in whatever project directory you point it at,
yet it is welded to the engine over the HTTP surface and uses that coupling
for real optimization.

```bash
vitriol serve          # the engine wakes up (any lane, any model)
cd ~/my-project        # the workshop opens for THIS directory
vitriol officina       # the stone greets you; start working
```

What the tight coupling buys you — none of which a generic agent harness
can do:

- **Live engine telemetry in your editor.** Braille gradient gauges (the
  same ones the engine TUI draws) show slot occupancy, decode tok/s, and
  boot decode totals, streamed from `/metrics` and `/slots`. You never
  open a second terminal to know how far along your model is.
- **Context tuned to the real window.** Officina knows the engine's
  certified context — it probes `/props` at startup and registers the
  *actual* `n_ctx`, not a catalog guess.
- **Checkpoint-aware.** Warm KV checkpoint/restore and `/rewind` (worktree
  + context from one turn key) ride the engine's checkpoint endpoints.
- **The composer, the panel, the stone.** A braille watermark of the
  VITRIOL logo greets an empty workshop and dissolves on first input; a
  cordoned panel shows your coupling, folder, token flow, and touched
  files; the composer sits pinned at the floor of the screen.
- **Couplings.** The default coupling is *Lapis Occultus – VITRIOL*:
  whatever the engine is serving, we use. Alternative providers (e.g. a
  euro-capped cloud escalation) hot-swap mid-session with the conversation
  intact.

Officina is self-contained in [`officina/`](officina/) and [
`docs/OFFICINA.md`](docs/OFFICINA.md) is its manual: extensions, Plan/Build
agent modes (`TAB`), `/history`, `/panel`, `/resume`, couplings, and the
design law — upstreams are mined, the workshop is ours. Related reading:
[`docs/SELF-SUFFICIENCY-2026-08-31.md`](docs/SELF-SUFFICIENCY-2026-08-31.md)
(the standalone guarantee),
[`docs/PROVENANCE.md`](docs/PROVENANCE.md) (every borrowed thing, cited),
[`docs/SYSTEMS-MAP-2026-09-01.md`](docs/SYSTEMS-MAP-2026-09-01.md) (who
consumes what), [`docs/LAYOUT-FORK-2026-08-31.md`](docs/LAYOUT-FORK-2026-08-31.md)
(the docked-shell build), and [`docs/HANDOFF-2026-08-31.md`](docs/HANDOFF-2026-08-31.md)
(session handoff).

## The three wars (chapter two)

Everything above was written during war one and is kept for the record.
What actually happened since is a story in three acts:

**War I — fit the model (35B era).** A Qwen3.6-35B MoE needs ~12 GB of
weights; the GPUs here have 8 and 12 GB. Answer: don't fit it — stream it.
Page-locked host RAM ("RAM Shot"), a custom copy engine, Chimera dual-backend
(CUDA+Vulkan). It worked: 23.3 tok/s where the baseline was "doesn't boot".
Sections below describe this era; treat them as history.

**Verdict interlude.** Then the project did the rare thing: it measured its
own darling. On DDR3 + narrow PCIe, *streaming a model that fits in VRAM* is
a pessimization — resident execution beat every streamed configuration. That
verdict is codified as the residency rule and most of War I's machinery is
now a tombstone. See [docs/VERDICTS.md](docs/VERDICTS.md).

**War II — afford the model (27B era).** Qwen3.8-27B nearly fits across both
GPUs. Answer: buy the missing 8 GB (a secondhand 1070 Ti next to the 3060),
then make the remainder behave — TurboQuant KV at 3.5 bpw, LULL attention-
probe scoring to decide which KV pages earn their keep, slot tenancy so two
tenants share one server without stealing each other's context. Certified
result: **96,836 filled tokens @ 11.32 tok/s** — depth measured, not
window-allocated ([docs/BENCHMARKS.md](docs/BENCHMARKS.md)).

**War III — keep it alive (now).** A model that runs on a red-lined 16 GiB
box dies differently: OOM kills, swap-thrash hangs, task-queue jams. VITRIOL
became an appliance: disk checkpoints that survive crashes by default, a
sidecar that replays conversation warmth into fresh instances within seconds,
a hang watchdog, a proactive bounce that restarts cleanly *before* memory
exhaustion wedges the box, and an oom-shield that works backwards — since
unprivileged users cannot protect their own processes, it marks everything
else as more killable. See [docs/OPERATIONS.md](docs/OPERATIONS.md).

> **Reading the repo**: `docs/ARCHITECTURE.md` is the single source of truth
> for current behavior. `docs/VERDICTS.md` holds every dead idea and why it
> died. Sections of this README dated to Wars I–II are preserved as history.

### Hardware assumptions (what's tuned where)

This tree is tuned for one specific machine: i7-3770 (no AVX2), 16 GiB DDR3 +
zram, RTX 3060 12 GiB + GTX 1070 Ti 8 GiB. Tensor splits (`ts 24,12`), KV
quant choices, cache caps (`--cache-ram 1024` against an 8 GiB default
entitlement), and sidecar thresholds are all *this-box* numbers. They live in
`profiles/` (personal) and `profiles/examples/` (generic starting points) —
re-tune there, not in code.

### Upstream posture

`master` tracks upstream ggml-org for periodic merges only. The `vitriol`
branch is the canonical daily driver and drifts deliberately; `vitriol-mellum2`
is a frozen alias of the same history. Merge cadence: when upstream grows
something we want, not before.

## Quick Start

```bash
# 1. Clone with submodule (llama.cpp is pinned)
git clone --recursive https://github.com/Randozart/VITRIOL.git
# Or if already cloned: git submodule update --init --recursive

# 2. Build (tests and examples excluded — see issue #1)
cd vitriol/llama.cpp && cmake -B build -DGGML_CUDA=ON -DGGML_NATIVE=ON \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
  && cmake --build build -j$(nproc)
# Vulkan: optional — VITRIOL auto-detects at runtime. CUDA-only is fully supported.
# Install Vulkan SDK and add -DGGML_VULKAN=ON for Chimera dual-backend mode.

# 3. One-time capability grant
./vitriol setup

# 4. Configure
./vitriol config

# 5. Run
./vitriol run
```


## Configuration

VITRIOL has several feature flags that control memory, context efficiency, and retrieval. Each has measurable trade-offs between throughput, context size, and recall quality.

| Flag | Effect | tok/s impact | Use Case |
|------|--------|-------------|----------|
| `--spec-type mtp` / `--spec-draft-n-max 2` | MTP speculative decoding (+20% gen) | +20% | Max throughput (requires MTP-capable model) |
| `--chimera-mode auto` | Dual-backend: CUDA for MoE, Vulkan for dense ops | **+80%** | Auto-detected when both backends available |
| `--cache-type-k q4_0` | K cache quantized to 4-bit | Required at 256K | Reduces host KV. V cache stays f16 — **do not use `--cache-type-v`** |
| `--kv-quant-v f16` | V cache precision (default f16) | — | **f16 only** — q8_0/q4_0 corrupt output with VITRIOL |
| `--pin-layers N` | Pin first N layers' expert tensors in VRAM | +5-10% | Covers early MoE layers in VRAM (default 8) |
| `--kv-mode offload` | KV cache in host RAM | Enables 256K context | Long context coding |
| `--kv-mode sparse` | Attention-score eviction (4-8x compression) | Saves host RAM | Extreme context length |
| `--frozen-prompt on` | Cache KV prefix across requests | Prefill saved | Repeated requests, same system prompt |
| `--memory-mode on` | Cross-session persistent memory via SQLite | ~5.0 | Multi-session projects |
| `--semantic-mode on` | Cosine similarity retrieval | ~5.0 | Large memory databases |
| `--disk-offload` | File-backed mmap for models > system RAM | — | Large models (e.g., 24 GB Coder-Next) |

All flags can be set via CLI flag, env var, or the TUI (`vitriol config`).

**Configuration defaults guide:** [`docs/CONFIG_DEFAULTS_GUIDE.md`](docs/CONFIG_DEFAULTS_GUIDE.md) — why each default was chosen, measured performance impact, and when to diverge.

**Full reference:** [`docs/CONFIG_REFERENCE.md`](docs/CONFIG_REFERENCE.md) — every flag explained with trade-offs, use cases, and recommended combinations.

**Recommended settings:** [`docs/RECOMMENDED_SETTINGS.md`](docs/RECOMMENDED_SETTINGS.md) — exact optimal config for the GTX 1070 Ti system.

**Optimization plans:** [`docs/plans/`](docs/plans/) — 5 master plans covering compute, memory, speculative decoding, early exit, and graph optimizations.

**Findings log:** [`docs/FINDINGS_2026-05-19.md`](docs/FINDINGS_2026-05-19.md) — detailed benchmark sweep results, floundering log, and lessons learned.

**OpenCode setup:** [`docs/OPENCODE_SETUP.md`](docs/OPENCODE_SETUP.md) — configuring VITRIOL as an OpenCode provider, why `vitriol setup` is required, workflow recommendations.

**Integration** — VITRIOL exposes an OpenAI-compatible API at `http://0.0.0.0:8279/v1`, compatible with OpenCode, little-coder, and any OpenAI SDK client. Config profiles (`vitriol config save|load`) let you switch between presets like `balanced` (136K ctx, MTP2) and `little-coder` (65K ctx, pin10, MTP3) without rebuilding. See [`docs/OPENCODE_SETUP.md`](docs/OPENCODE_SETUP.md) for detailed setup.

**Test results:** [`docs/TEST_REPORT_2026-05-17.md`](docs/TEST_REPORT_2026-05-17.md) — measured tok/s, VRAM savings, and bug fixes.


## What Is It

VITRIOL is a **VRAM extension layer** for [llama.cpp](https://github.com/ggml-org/llama.cpp) that lets **old consumer GPUs** run modern MoE language models they have no business running.

The problem: the best open-weight models are MoE architectures (Mixture of Experts) with 200+ expert weight matrices. A Qwen3.6-35B-A3B needs ~12 GB VRAM for weights alone. A GTX 1070 Ti has 8 GB. An RTX 3060 has 12. A GTX 960 has 2. These GPUs are in millions of machines — perfectly capable of fast matrix math, but VRAM-starved.

VITRIOL's insight: MoE models only activate ~2-8 out of 256 experts per token. The expert weights don't need to live in VRAM. Keep them in **page-locked system RAM** instead — the GPU reads them over PCIe DMA on demand. The base model, attention weights, KV cache, and compute buffers stay in VRAM. Only the experts are offloaded.

**Result:** 23.3 tok/s on a GTX 1070 Ti (8 GB) with a Qwen3.6-35B MoE model via the Chimera dual-backend (CUDA+Vulkan) — **+309% vs the pre-VITRIOL x8 baseline** (5.7 tok/s). The model doesn't fit at all without VITRIOL. **27.1 tok/s** on the same hardware with Mellum2-12B-A2.5B (Q4_K_M).

### Recommended Models

| Model | Quant | VRAM (model) | t/s | Notes |
|-------|-------|-------------|-----|-------|
| [Mellum2-12B-A2.5B-Instruct](https://huggingface.co/CodeFault/Mellum2-12B-A2.5B-Instruct-GGUF) | Q4_K_M (7.5 GB) | 6228 MiB | **27.1** | 64 MoE experts / 8 active, native MTP head built-in, sliding-window attention. Fastest option. |
| [Qwen3.6-35B-A3B-MTP](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-MTP-GGUF) | IQ2_M (~12 GB) | ~2500 MiB | **23.3** | 256 MoE experts, Chimera dual-backend, MTP speculative. Most params per VRAM. |
| [Qwen3.6-35B-A3B-UD](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-UD-GGUF) | Q2_K_XL (~12 GB) | ~2500 MiB | 20.0 | Baseline quant (non-MTP). Lower bit-width, higher throughput on very low VRAM. |

**Bundled profiles:**
```bash
vitriol run --profile mellum2    # Mellum2 optimized (ngl=24, ctx=32768, lru=2048)
vitriol run --profile icarus     # Qwen3.6 optimized (ngl=99, ctx=65536, pin=12, mtp=5)
```

| Metric | Value |
|--------|-------|
| Model | [Qwen3.6-35B-A3B](https://huggingface.co/unsloth/Qwen3.6-35B-A3B-MTP-GGUF) (34.66B, 256 MoE experts, IQ2_M by Unsloth) |
| GPU | GTX 1070 Ti (Pascal, 8 GB VRAM, PCIe Gen3 x16) |
| CPU | Intel 4th gen (Haswell, no AVX2) |
| Backend | **Chimera dual-backend** (CUDA VITRIOL for MoE + Vulkan for dense ops) |
| Generation (Chimera + MTP N=2 + pin 8) | **23.3 tok/s** ✅ (2026-05-22, server) |
| Generation (no opts) | 8.9 tok/s |
| VRAM saved | ~10 GB (experts stay in host RAM) |
| System RAM used | ~10 GiB (VITRIOL buffer + KV cache) |
| VRAM used | ~2.5 GiB (model + KV cache + compute buffers) |


## How It Works

### The Trick: Making CUDA Think Host Memory Is Fine

CUDA kernels can read from **page-locked host memory** over PCIe DMA transparently — the GPU's memory controller handles the cross-PCIe access as if it were VRAM, just slower (~12 GB/s PCIe 3.0 vs ~256 GB/s GDDR5). llama.cpp normally avoids this because it's bandwidth-inefficient. But for MoE experts — where each matmul uses only 2-8 of 256 experts per layer — the effective bandwidth needed is low enough that PCIe latency is acceptable.

VITRIOL implements this through a custom **ggml backend buffer type**:

```
VITRIOL buffer type (CUDA experts)
  │
  ├─ 1. Allocation
  │     mmap(10 GB anonymous)         ← reserve address space
  │     madvise(MADV_HUGEPAGE)        ← hint for 2 MB pages (lower GPU TLB pressure)
  │     mlock                         ← pin to RAM, prevent swapping
  │     cudaHostRegister              ← register with CUDA for DMA access
  │
  ├─ 2. Model load
  │     memcpy from GGUF file → VITRIOL buffer (one-time 10 GB copy, ~64 s)
  │
  └─ 3. Inference
        Set is_host=true on the buffer type → llama.cpp scheduler routes
        MUL_MAT_ID to CUDA backend → GPU reads expert weights over PCIe DMA

VITRIOL VK buffer type (Vulkan dense ops, Chimera only)
  │
  ├─ 1. Allocation
  │     posix_memalign(64K alignment)  ← aligned for VK_EXT_external_memory_host
  │     mlock                         ← pin to RAM
  │
  └─ 2. Inference
        VkBuffer created lazily via ggml_vk_buffer_from_host_ptr()
        → Vulkan reads dense tensor weights over PCIe DMA (zero-copy)
```

The key flag is **`is_host=true`**. When the graph scheduler sees this on a buffer type it supports (via `supports_buft`), it treats the tensor as host-resident and keeps it in system memory. The CUDA backend accesses `src0->data` directly — the GPU's DMA engine fetches the bytes over PCIe when the kernel reads from that address.

### Chimera Dual-Backend (CUDA + Vulkan)

The Chimera hybrid backend routes each operation to the optimal GPU backend:

| Operation | Backend | Why |
|-----------|---------|-----|
| MoE expert matmuls (`MUL_MAT_ID`) | CUDA VITRIOL | Page-locked DMA + pin pool + predictor |
| SSM scan (d_state=16) | Vulkan | Pre-baked command buffers, Mamba-1 shader |
| SSM scan (d_state=128/256) | Vulkan | Already supported |
| Attention (KQ, PV) | Vulkan | Pre-baked pipelines |
| Dense matmuls | Vulkan | Pre-baked pipelines |

**Architecture:**
```
                    Page-locked Host RAM
                    ┌──────────────────────┐
                    │ Expert weights       │ Dense weights        │
                    │ (CUDA VITRIOL type)  │ (VITRIOL VK type)    │
                    └──────────────────────┘
                               │                    │
                    CUDA: cudaHostRegister    VK: VK_EXT_external_memory_host
                    reads via PCIe DMA        reads via imported VkBuffer

Activations: CUDA VRAM ←→ CPU staging ←→ Vulkan VRAM
             (~0.001ms per 16KB copy, ~0.13% overhead per token)
```

**Auto-detect:** Set `VITRIOL_CHIMERA_MODE=auto` (default). The model loader
automatically detects if the Vulkan backend is available via dlsym. If both
CUDA and Vulkan are present, Chimera activates. No manual config needed.

**Modes:**
| Mode | Effect |
|---|---|
| `auto` (default) | Chimera if both backends available, CUDA-only otherwise |
| `cuda` | CUDA-only (standard VITRIOL, no Vulkan) |
| `vulkan` | Vulkan-only (all tensors via VK_EXT_external_memory_host) |
| `off` | CUDA-only (same as cuda) |

### Fast Path vs Slow Path (CUDA)

llama.cpp's `ggml_cuda_mul_mat_id` has three paths for MoE matmuls:

| Path | Trigger | Expert Data Access | VITRIOL? |
|------|---------|-------------------|----------|
| **MMVQ** | Batch ≤ 8, quantized weights | Reads `src0->data` directly with expert bounds | Yes — host DMA |
| **MMQ** | Large batch, quantized | Reads `src0->data` directly with expert bounds | Yes — host DMA |
| **cuBLAS (slow path)** | Everything else | Per-expert tensor slices | No — FP16 only |

For quantized models, the MMQ/MMVQ fast paths handle all MoE inference on CUDA.
Dense ops (SSM, attention) run on Vulkan with pre-baked command buffers when
Chimera mode is active.

### Why Not Just Load Everything Into VRAM?

Because it doesn't fit. Qwen3.6-35B-A3B at IQ2_M is ~3.5 GB. The GTX 1070 Ti has 8 GiB total.
Without VITRIOL, llama.cpp crashes with `cudaMalloc failed: out of memory` at `-ngl 99`.

With VITRIOL (Chimera mode):
- Base model weights (dense, non-expert): ~1.3 GiB in VRAM
- Expert weights: 0 GiB in VRAM (host RAM only, CUDA DMA)
- Dense weights: 0 GiB in VRAM (host RAM only, Vulkan VK_EXT)
- KV cache + compute: ~28 MiB (8K ctx K q4_0) + ~304 MiB (compute buffers)
- MTP head (optional): +302 MiB in VRAM
- Pin pool (optional): +0-4096 MiB in VRAM (expert tensors cached in VRAM)
- **Total VRAM: ~1.6 GiB (MTP, no pin pool) / ~5.7 GiB (MTP + pin 8)**


## CLI Reference

```
vitriol run [options]      interactive inference session
vitriol serve [options]    persistent HTTP API server
vitriol stop               stop running server
vitriol bench [options]    quick throughput benchmark (uses llama-bench)
vitriol config             interactive configuration TUI
vitriol config show        print current configuration
vitriol config init        create config file with defaults
vitriol config reset       restore defaults
vitriol config edit        open config in $EDITOR
vitriol config set <key> <val>  set a config value
vitriol setup              set CAP_IPC_LOCK capability
vitriol help               this message

Run options:
  -m PATH        model file path
  -c N           context window (tokens)
  -t N           CPU threads
  -ngl N         GPU layers to offload
  -lru MB        LRU VRAM cache size (inactive for quantized models)
  --memory-mode MODE  emulated memory: on | off (default: off)
  --kv-quant MODE     K cache quantization: f16 | q8_0 | q4_0. **Warning:** only K cache; V cache stays f16 by default.
  --kv-quant-v MODE   V cache quantization: f16 (default, safe) | q8_0 | q4_0 (RISK: corrupts output with VITRIOL)
  --spec-type TYPE    speculative decoding: mtp | draft (default: disabled)
  --spec-draft-n-max N  tokens to draft per cycle (default: 0, recommended: 2 for IQ2_M)
  --pin-layers N       pin first N layers' expert tensors in VRAM
  --prune-experts N    drop bottom N of 8 active experts (0-7, experimental)
  --predictive-prefetch on|off  cross-layer expert prefetch (DMA stream overlap)
  --chimera-mode MODE  backend routing: auto | cuda | vulkan | off (default: auto)
  --disk-offload       enable file-backed mmap for models larger than system RAM
  --verbose      enable debug logging
  --dry-run      print config without launching

Serve options:
  -m PATH        model file path
  -c N           context window (tokens)
  -t N           CPU threads
  -ngl N         GPU layers to offload
  -lru MB        LRU VRAM cache size (inactive for quantized models)
  --host ADDR    bind address (default: 127.0.0.1)
  -port N        server port (default: 8279)
  -p N           parallel slots (default: 1)
  --memory-mode MODE  emulated memory: on | off (default: off)
  --kv-quant MODE     K cache quantization: f16 | q8_0 | q4_0. K only; V stays f16.
  --kv-quant-v MODE   V cache quantization: f16 (safe) | q8_0 | q4_0 (RISK)
  --spec-type TYPE    speculative decoding: mtp | draft (default: disabled)
  --spec-draft-n-max N  tokens to draft per cycle (default: 0, recommended: 2)
  --pin-layers N       pin first N layers' expert tensors in VRAM
  --prune-experts N    drop bottom N of 8 active experts (0-7, experimental)
  --predictive-prefetch on|off  cross-layer expert prefetch (DMA stream overlap)
  --chimera-mode MODE  backend routing: auto | cuda | vulkan | off (default: auto)
  --disk-offload       enable file-backed mmap for models > system RAM
  --detach       run server in background
  --verbose      enable debug logging
  --dry-run      print config without launching

Bench options:
  vitriol bench -n 100        generate 100 tokens, report t/s
  All VITRIOL_MODE, --prune-experts, --pin-layers flags apply
```

Config persisted in `~/.vitriol/config`. Precedence: CLI flag > Config > Env var > Default.

Use `vitriol serve --detach` for background API mode, `vitriol stop` to shut down.

## VITRIOL Modes

| Mode | What it does |
|------|-------------|
| **stream** | **(Default.)** RAM Shot + VITRIOL DMA. All expert weights in page-locked host RAM. GPU reads them over PCIe DMA on demand. MTP head (if enabled) loads through the same buffer. |
| **Chimera** | **(Auto-detect, VITRIOL_CHIMERA_MODE=auto.)** Dual-backend hybrid. Expert tensors → CUDA VITRIOL DMA (page-locked host RAM). Dense tensors (SSM, attention, norms) → Vulkan via `VK_EXT_external_memory_host`. Cross-backend activation copies handled automatically by the scheduler (~0.13% overhead). 23.3 tok/s verified. |

### Memory Mode (Experimental)

When enabled (`--memory-mode on` or `VITRIOL_MEMORY_MODE=on`), a Python Flask shim intercepts all requests before they reach llama-server. On each request:

1. **Extract** user intent from the last message
2. **Retrieve** relevant context from a project-local SQLite memory database (episodic + semantic scoring with cascading multi-hop retrieval)
3. **Inject** retrieved context as a system message, staying within a token budget
4. **Forward** the compact prompt to llama-server
5. **Store** the response back into the memory database
6. **Update** Hebbian edge weights (post-response connection strengthening)

Ports swap automatically: llama-server moves to `PORT-1` (8278), the shim listens on `PORT` (8279). OpenCode's baseURL never changes.

```
Memory OFF:  llama-server → port 8279
Memory ON:   llama-server → port 8278, shim → port 8279
```

Configure via `vitriol config` (TUI option 4) or `vitriol serve --memory-mode on`.

## Performance

### Current Best: 23.3 tok/s (Server, Verified Clean, Chimera Dual-Backend)

System: GTX 1070 Ti (PCIe Gen3 x16), 15 GB RAM, IQ2_M model, Chimera dual-backend
(CUDA VITRIOL for MoE + Vulkan for dense ops), MTP N=2, pin=8, `--cache-type-k q4_0`.

| Config | Gen (tok/s) | vs x8 baseline | Notes |
|--------|------------|----------------|-------|
| PCIe x8 (GTX 960 present, no VITRIOL) | 5.7 | — | Pre-VITRIOL baseline |
| PCIe x16 (GTX 960 removed) | 8.9 | +56% | Before MTP/pin/Chimera |
| + Q2_K_XL + pin 15 | 9.88 | +73% | ✅ Clean (no MTP) |
| + IQ2_M + MTP N=2 + pin 8 | 12.82 | +125% | ✅ Clean (2026-05-21) |
| **+ Chimera + CAP_IPC_LOCK** | **23.3** | **+309%** | ✅ Clean (2026-05-22) |

**Key findings:**
- **Best config:** IQ2_M model with Chimera dual-backend, `--spec-type mtp --spec-draft-n-max 2 --cache-type-k q4_0` at **23.3 t/s** (verified clean, server-side)
- MTP acceptance rate: ~100% (19/19 drafts accepted per typical cycle at 8K context)
- Chimera auto-detects: `VITRIOL_CHIMERA_MODE=auto` enables CUDA+Vulkan hybrid automatically
- `--cache-type-v` **must remain at f16** — V cache quantization corrupts output with VITRIOL (see Experiment 17)
- Pin pool (`VITRIOL_PIN_FIRST_N_LAYERS=8`) covers first 8 expert layers in VRAM

See full sweep data in [`docs/BENCHMARK_RESULTS.md`](docs/BENCHMARK_RESULTS.md#mtp-draft-n-max-sweep-2026-05-19), [`docs/FINDINGS_2026-05-19.md`](docs/FINDINGS_2026-05-19.md), and [`docs/plans/COMPUTE_OPTIMIZATIONS.md`](docs/plans/COMPUTE_OPTIMIZATIONS.md).

### VRAM at 8K Context (Chimera Dual-Backend)

| Component | Non-MTP | With MTP Head |
|-----------|---------|---------------|
| Model weights (GPU, dense) | ~1337 MiB | ~1337 + 302 MiB |
| VITRIOL buffer (host RAM, experts) | ~10040 MiB | ~10040 MiB |
| VITRIOL VK buffer (host RAM, dense) | ~0 MiB (imported) | ~0 MiB (imported) |
| KV cache (K q4_0 + V f16, 8K ctx) | ~28 MiB | ~28 MiB |
| Compute buffers | ~304 MiB | + overhead |
| Pin pool (expert VRAM cache) | ~0-4096 MiB | ~0-4096 MiB |
| **Total VRAM** | **~1.9-5.9 GiB** | **~2.2-6.2 GiB** |
| **Total system RAM** | **~10 GiB** | **~10.3 GiB** |
| **VRAM headroom** | **~2-6 GiB** | **~2-6 GiB** |


## Hardware Targets

> **PCIe warning:** If you have a secondary GPU in the second PCIe slot, the primary slot
> may drop from x16 to x8. This halves PCIe bandwidth and reduces gen speed by ~60%.
> VITRIOL is PCIe-bound — every token transfers ~40 MB of expert weights across the bus.
> x8 bottleneck: ~5.7 tok/s. x16: ~9.1 tok/s (before MTP).

| GPU | VRAM | Status | Notes |
|-----|------|--------|-------|
| GTX 1070 Ti | 8 GB | ✅ Verified | PCIe 3.0 x16, **27.1 tok/s** (Mellum2 12B Q4_K_M) / **23.3 tok/s** (Qwen3.6 35B IQ2_M, Chimera, MTP N=2, pin=8) |
| RTX 3060 | 12 GB | ✅ Supported | More VRAM for larger KV cache |
| RTX 4090 | 24 GB | ✅ Supported | PCIe 4.0 x16 → higher bandwidth |
| AMD RX 7000 | varies | ✅ Chimera (Vulkan) | Vulkan backend handles dense ops; MoE via CUDA not available |
| Intel Arc | varies | ✅ Chimera (Vulkan) | Vulkan backend via `VK_EXT_external_memory_host` |

**CPU requirement:** VITRIOL uses the GPU as the primary MoE compute engine (experts
streamed over PCIe DMA). The CPU is only an orchestrator — no AVX2 or fast CPU is
required. This is in contrast to CPU-based expert offloading (e.g., KTransformers),
which depends heavily on CPU vector extensions.

## Compatibility

### Tested
- **GPU:** NVIDIA GeForce GTX 1070 Ti (CC 6.1, 8 GB VRAM, PCIe Gen3 x16)
- **CPU:** Intel 4th gen (Haswell, no AVX2) — only orchestrates, does not compute experts
- **RAM:** 15 GB DDR3 system, NVMe SSD
- **OS:** Linux — Ubuntu 24.04, kernel 6.17+, NVIDIA driver 535.288.01, CUDA 12.2
- **Models:** Qwen3.6-35B-A3B-UD-Q2_K_XL (baseline, ~2.2 bpw) / IQ2_M (MTP-capable, ~2.6 bpw)
- **llama.cpp:** Pinned submodule with VITRIOL CUDA integration
- **Context:** 128,000 tokens at K q4_0 + V f16 KV quant (`--cache-type-k q4_0`; `--cache-type-v` is forbidden — see Experiment 17)

### Likely works
- **NVIDIA GPUs with CC ≥ 5.0:** All Pascal, Turing, Ampere, Ada, Blackwell cards. The `cudaHostRegister` + PCIe DMA path is architecture-agnostic. Higher VRAM GPUs benefit from larger LRU cache or keeping more layers in VRAM.
- **NVIDIA GPUs with CC 5.x (Maxwell):** Some GPU kernel ops may be missing — set `CUDA_VISIBLE_DEVICES` to exclude them or use `vitriol config` to set `exclude_secondary = true`.
- **ROCm/HIP (AMD):** Would need a mechanical port of the CUDA driver API calls to HIP equivalents (`cudaHostRegister` → `hipHostRegister`, `cuMemAlloc` → `hipMalloc`, etc.). llama.cpp already has `GGML_USE_HIP` guards. Estimated ~400 lines changed across 3 files.
- **Windows (NVIDIA):** NVCC + `cudaHostRegister` works identically. Untested but no fundamental blocker. The `vitriol` CLI launcher currently uses bash — would need a PowerShell/batch wrapper.

### Likely won't work
- **Intel GPUs (SYCL/oneAPI):** SYCL doesn't expose page-locked host DMA in the same way. The unified memory model on integrated GPUs also makes VITRIOL's trick unnecessary — there's no discrete PCIe bus to cross.
- **Apple Silicon (Metal):** Unified memory architecture. CPU and GPU share the same physical RAM pool. VITRIOL provides no benefit — the entire model fits in unified memory if it fits at all.
- **Nouveau (open-source NVIDIA driver):** Lacks `cudaHostRegister` equivalent. The GMMU page tables populated by the proprietary NVIDIA RM driver are required for PCIe DMA from system memory.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                    GPU (GTX 1070 Ti, 8 GB VRAM)              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  Base model (1.3 GiB)  │  KV Cache  │  Compute buffers │  │
│  │  Embeddings, Attention,│  (512 ctx) │  (sched: ~215 MB)│  │
│  │  RMS Norm, Output      │  (10 MB)   │  LRU pool (opt)  │  │
│  │                         │            │  (512 MB)        │  │
│  └────────────────────────────────────────────────────────┘  │
│                    ▲ PCIe DMA (~12 GB/s)                      │
└────────────────────┼─────────────────────────────────────────┘
                     │
┌────────────────────┼─────────────────────────────────────────┐
│           CPU / System RAM (DDR3, ~20 GB/s)                  │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  VITRIOL buffer (10 GiB, page-locked, never swapped)   │  │
│  │  256 expert weight tensors, 0% VRAM footprint           │  │
│  │  mmap → madvise(HUGEPAGE) → mlock → cudaHostRegister    │  │
│  └────────────────────────────────────────────────────────┘  │
│  ┌────────────────────────────────────────────────────────┐  │
│  │  GGUF mmap (11.44 GiB file, page cache)                │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

## Emulated Memory Architecture (Experimental)

When **memory mode** is enabled (`--memory-mode on`), a Flask proxy shim sits between the client and llama-server. Every request is intercepted, memory is queried for relevant context, and the prompt is compacted *before* reaching the inference engine — eliminating OpenCode's expensive context compaction loop.

```
OpenCode ──POST /v1/chat/completions──► vitriol_shim.py (port 8279)
                                              │
                                        1. Parse X-Project-Id header
                                        2. Extract user intent from last message
                                        3. Query .vitriol/<project>/memory.db
                                           ├─ Scorer: keyword overlap + recency
                                           ├─ Hebbian weight → edge strength
                                           └─ Cascading multi-hop retrieval
                                        4. Inject retrieved context as system msg
                                        5. Forward to llama-server (port 8278)
                                              │
                                              ▼
                                        llama-server (port 8278)
                                        (8192-token context, never compacts)
                                              │
                                        Post-response:
                                        6. Store response as new episode
                                        7. Hebbian weight update on co-occurring edges
```

Ports swap transparently — OpenCode always talks to port 8279:

```
Memory OFF:  llama-server → port 8279
Memory ON:   llama-server → port 8278, shim → port 8279
```

See `docs/EMULATED_MEMORY_ARCHITECTURE.md` for the full design (DB schema, scoring function, spreading activation, token-budgeted compaction, Hebbian updates, consolidation/sleep).

## Project Structure

```
├── vitriol                  ← CLI entry point (symlink to scripts/vitriol)
├── scripts/
│   └── vitriol              ← Main CLI: config TUI + run + serve + stop + setup
├── libvitriol/
│   ├── vitriol_shim.py      ← Flask proxy with memory mode toggle
│   └── memory/              ← Emulated memory subsystem (7 modules)
│       ├── __init__.py
│       ├── db.py            ← SQLite schema + CRUD
│       ├── scorer.py        ← Composite relevance scoring
│       ├── retrieval.py     ← Intent classification + cascading retrieval
│       ├── compact.py       ← Token-budgeted compaction
│       ├── hebbian.py       ← Post-response edge weight updates
│       └── consolidate.py   ← Background summarization + pruning
├── assets/
│   ├── vitriol-header.txt   ← ASCII art banner
│   └── vitriol_logo.svg     ← SVG logo
├── llama.cpp/               ← Git submodule (pinned commit)
├── llama.cpp/ggml/src/ggml-cuda/
│   ├── vitriol-buffer.{cpp,h}              ← RAM Shot buffer type (CUDA)
│   ├── vitriol-cuda-integration.{cpp,h}    ← Pin pool + predictor + config
│   ├── vitriol_copy_engine.{cpp,h}         ← CE DMA (standalone)
│   └── ggml-cuda.cu                        ← supports_buft + pin pool hooks
├── llama.cpp/ggml/src/ggml-vulkan/
│   ├── vitriol-vk-buffer.{cpp,h}           ← VITRIOL VK buffer type (Chimera)
│   └── vulkan-shaders/
│       └── ssm_scan_mamba1.comp            ← Mamba-1 SSM scan GLSL shader
├── llama.cpp-patches/       ← Tracked diffs for all VITRIOL changes
├── docs/
│   ├── OPTIMIZATION_PLAN.md (V2)           ← 4-layer roadmap with citations
│   ├── OPTIMIZATION_PLAN_V1.md             ← Preserved original
│   ├── OPTIMIZATIONS_2026-05-19.md         ← Optimization catalog with prior art
│   ├── RECOMMENDED_SETTINGS.md             ← Optimal config for this system
│   ├── FINDINGS_2026-05-19.md              ← Benchmark sweeps, floundering log
│   ├── BENCHMARK_RESULTS.md               ← All benchmark data across configs
│   └── EMULATED_MEMORY_ARCHITECTURE.md     ← Memory design doc
├── EXPERIMENT_LOG.md        ← Complete test history (15 experiments)
├── docs/plans/              ← Master optimization plans (5 consolidated docs)
│   ├── COMPUTE_OPTIMIZATIONS.md  ← T-MAC, Top-K Prune, all ALU-bypass approaches
│   ├── MEMORY_OPTIMIZATIONS.md  ← Expert Pinning, LRU, Output Cache, Asymmetric
│   ├── EARLY_EXIT.md            ← Residual stagnation detection (implemented, negative result)
│   ├── SPECULATIVE_DECODING.md  ← MTP + speculative routing plans
│   └── GRAPH_OPTIMIZATIONS.md   ← Graph split fix, scheduler improvements
├── .opencode/plans/         ← OpenCode-local copies (not on GitHub)
├── SESSION_LOG_2026-05-17.md ← This session's progress report
├── ROADMAP.md               ← Phased development plan
├── MILESTONE_1.md           ← Failed approaches archive (7 approaches)
└── MILESTONE_2.md           ← RAM Shot: success report
```

## Ars Priori & Acknowledgements

VITRIOL stands on the shoulders of giants. Every core insight — DMA over PCIe, metapage completion signaling, async expert prefetching, extreme quantization on legacy hardware — was reverse-engineered from the following works. We document our debt explicitly.

### Inference Engine

| Project | What We Learned |
|---------|-----------------|
| **[llama.cpp](https://github.com/ggml-org/llama.cpp)** (ggml-org) | The core inference engine. GGUF format, CUDA backend, tensor loading pipeline. The `-ot` (override tensor) flag in PR #11397 was the breakthrough that enabled expert streaming. Our `vitriol-cuda-integration.cpp` hooks into `ggml-cuda.cu` at the tensor-copy boundary. Vulkan backend (PR #16463) provides SSM operation support and `VK_EXT_external_memory_host` for zero-copy host memory import. |
| **[PR #16463](https://github.com/ggerganov/llama.cpp/pull/16463)** (giuseppe) | Added SSM_SCAN and SSM_CONV to the Vulkan backend. Our Mamba-1 shader extends this with Mamba-2-only to add d_state=16 support for Qwen3's Gated Delta Net and Jamba2. |
| **[GGUF Format](https://github.com/ggerganov/llama.cpp/blob/master/ggml/include/gguf.h)** | Binary model format with tensor offsets accessible via `gguf_get_tensor_offset()`, `gguf_get_tensor_name()`, `gguf_get_tensor_type()` — the foundation of our expert parser. |
| **[PR #11397](https://github.com/ggerganov/llama.cpp/pull/11397)** (slaren) | Added `--override-tensor` (`-ot`) for per-tensor-type buffer placement. The exact mechanism we use: `-ot ".*exps.*=CPU"` keeps 8GB of experts on CPU while attention layers run on GPU. |
| **[PR #11571](https://github.com/ggerganov/llama.cpp/pull/11571)** (fairydreaming) | Load-all-experts-during-warmup; `llama_set_warmup()` API for ensuring all expert tensors are resident before inference. |
| **[PR #6387](https://github.com/ggerganov/llama.cpp/pull/6387)** (slaren) | Changed expert storage from per-expert tensors to a single 3D tensor — critical for our approach since all 256 experts are now in one contiguous block. |

### GPUDirect Storage & DMA

| Project | What We Learned |
|---------|-----------------|
| **[gds-nvidia-fs](https://github.com/NVIDIA/gds-nvidia-fs)** (NVIDIA) | Official GPUDirect Storage source code. We studied `nvfs-core.c`, `nvfs-pci.c`, and `nvfs-dma.c` to understand: kiocb completion callbacks for NVMe, shared metapage (4KB) for fast completion signaling, `wmb()` memory barriers before DMA. |
| **[open-gpu-kernel-modules](https://github.com/NVIDIA/open-gpu-kernel-modules)** (NVIDIA) | NVIDIA's open kernel module source for PCIe register-level operations — reference for understanding BAR mapping and GPU PCI config space. |
| **[hw-nvdla](https://github.com/NVIDIA/hw-nvdla)** (NVIDIA) | Hardware DLA documentation for understanding direct memory access patterns on NVIDIA silicon. |

### Async Scheduling & MoE Orchestration

| Project | What We Learned |
|---------|-----------------|
| **[KTransformers](https://github.com/kvcache-ai/KTransformers)** (kvcache-ai) | YAML-based layer placement across CPU/GPU, double-buffer prefetch pattern (compute layer N while streaming N+1), MoE-specific async scheduling. KTransformers targets modern CPUs (AMX/AVX512); VITRIOL inverts this — GPU as primary compute, CPU as orchestrator only. |
| **[PowerInfer](https://github.com/SJTU-IPADS/PowerInfer)** (SJTU-IPADS) | Neuron-level offloading with predictor for which neurons will fire — only loads those into GPU. Informs our predictive prefetching approach. |
| **Qwen3.6-35B-A3B MoE** | 256 experts, 8 active per token — the exact sparsity architecture that makes expert streaming viable. The MoE router (`ffn_gate_inp`) determines which 8 experts to load; only those need to be in VRAM. |

### 🏆 VITRIOL-Implemented Techniques

| Technique | Prior Art | VITRIOL Implementation |
|-----------|-----------|----------------------|
| **Chimera Dual-Backend** | N/A (VITRIOL-original) | CUDA+Vulkan hybrid: MoE experts on CUDA DMA, dense ops on Vulkan command buffers |
| **Mamba-1 Vulkan SSM Shader** | PR #16463 (Mamba-2 only) | GLSL compute shader for d_state=16 SSM scan, 128 threads/workgroup |
| **VITRIOL VK Buffer Type** | VK_EXT_external_memory_host spec | Page-locked host RAM imported into Vulkan via external memory extension |
| **Auto-Detect Backend Routing** | N/A (VITRIOL-original) | `VITRIOL_CHIMERA_MODE=auto` — dlsym-based detection of available backends |
| **RAM Shot** (page-locked host RAM) | LLM in a Flash (Apple, 2023) | `vitriol-buffer.cpp` — mmap+mlock+cudaHostRegister |
| **LRU VRAM cache** | HOBBIT (2024), KTransformers | Composite key (tensor_base, expert_idx), dedicated CUDA stream |
| **Fire-and-Forget DMA overlap** | PreScope (2025), Fate (2025) | `vitriol_lru_prefetch_async` — async H2D, cuStreamWaitEvent on cache hit |
| **Predictive prefetching** | Fate (2025), PowerInfer | Cross-layer + temporal heuristic, no training needed |
| **Expert Pinning** (tensor VRAM preload) | HOBBIT (2024) | Monolithic VRAM pool, scoped src0 redirect before fast-path |
| **Top-K Expert Pruning** | MoQE (Microsoft, 2023) | Drop bottom N of 8 experts before matmul, forces sorted path |
| **Approximate Output Cache** | Hidden State Sluggishness | Per-expert per-layer output float vector reuse across tokens |
| **Graph Split Fix** | N/A (VITRIOL-specific) | Share CUDA host buft identity to reduce scheduler splits from 17 to 2 |
| **Early Exit Infrastructure** | DeeBERT, PABEE | `n_build_layers` graph param, residual delta detection |

### Speculative Decoding

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[Fate](https://arxiv.org/abs/2502.12224)** — Fang et al. (2025) | Cross-layer expert prefetching: gate inputs from adjacent layers are ~99% correlated, enabling 97%+ prefetch accuracy with zero GPU overhead. Working third-party llama.cpp fork at `github.com/ongunm/llama-moe-cache` reports 1.91× on Qwen3-30B-A3B. |
| **[PreScope](https://arxiv.org/abs/2509.23638)** — Yu et al. (2025) | LLaPor lightweight predictor (0.5-2.8MB, 0.12-0.48ms), AsyncIO optimizer for overlapping PCIe transfers with GPU compute, cross-layer scheduler. 141% throughput improvement on Qwen3-30B-A3B. |
| **[HOBBIT](https://arxiv.org/abs/2411.01433)** — Tang et al. (2024) | Mixed-precision expert offloading on llama.cpp (~8000 lines). Token-level dynamic loading, layer-level adaptive prefetching, multi-dimensional expert cache. Up to 9.93× decoding speedup on edge devices. Code not open-sourced. |
| **[SP-MoE](https://arxiv.org/abs/2510.10302)** — Chen et al. (2025) | First SD-aware expert offloading: uses draft model's attention outputs to predict target model's expert activations. Combines MTP with expert prefetching. 1.07×-3.5× TPOT speedup. |
| **[MTP](https://arxiv.org/abs/2404.19737)** — Gloeckle et al. (Meta, 2024) | Proved that training models to predict N tokens at once improves reasoning and enables parallel decoding. Foundation of our MTP speculative decoding via Unsloth IQ2_M model. |
| **[Speculative Sampling](https://arxiv.org/abs/2211.17192)** — Leviathan et al. (Google, 2022) | Proved that verification of token sequences is parallelizable — checking 5 tokens takes the same time as checking 1. Foundation of all speculative decoding. |
| **[Speculative Sampling](https://arxiv.org/abs/2302.01318)** — Chen et al. (DeepMind, 2023) | Established rejection sampling math ensuring fast/slow model pair output is identical to the slow model alone. |
| **[Medusa](https://github.com/FasterDecoding/Medusa)** — Cai et al. (2024) | Multiple lightweight decoding heads on a single model to predict +1, +2, +3 tokens ahead. No second model needed. |
| **[EAGLE](https://github.com/SafeAILab/EAGLE)** — Li et al. (2024) | Predicts feature vectors (hidden states) instead of tokens — current SOTA for self-speculative decoding. |
| **[Self-Speculative Decoding](https://arxiv.org/abs/2307.13304)** — (2023) | Layer skipping: run a subset of layers for draft generation, full model for verification. Highly relevant for VITRIOL's DMA layer — skip PCIe transfer for 80% of MoE layers during draft phase. |
| **[Mixture of Speculative Experts](https://arxiv.org/abs/2402.13524)** — (2024) | Top-1 expert draft for MoE: generate guesses using 1/8 experts, verify with all 8. Directly applicable to VITRIOL's expert routing. |
| **[Prompt Lookup Decoding](https://github.com/apoorvumang/prompt-lookup-decoding)** — Umang (2024) | N-gram matching from existing context — if a token sequence appeared before, reuse it as a draft. Zero extra VRAM, "free" speed on code tasks. |

### KV Cache & Context Management

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[vLLM PagedAttention](https://arxiv.org/abs/2309.06180)** — Kwon et al. (2023) | Block-level KV cache management enabling near-zero memory waste. Foundation of efficient serving. |
| **[KIVI](https://arxiv.org/abs/2402.02750)** — Liu et al. (2024) | 2-bit KV cache quantization with minimal accuracy loss. Informs `--kv-quant q4_0` and future KV compression. |
| **[StreamingLLM](https://arxiv.org/abs/2309.17453)** — Xiao et al. (2023) | Identified "attention sinks" (first few tokens) that must be preserved for stable long-context generation. Core insight behind sparse KV caching. |

### CPU Offload & Ternary Compute

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[Fiddler](https://arxiv.org/abs/2402.14103)** — Kamahori, Gu, Zhu, Kasikci (2024) | Demonstrated that moving *activations* to CPU for MoE expert computation can be faster than pulling weights to GPU via PCIe DMA. Informs future `--engine-mode fiddler-cpu`. |
| **[T-MAC](https://github.com/microsoft/T-MAC)** (Microsoft, 2024) | Lookup-table-based inference for low-bit models. Originally CPU-focused (LUTs in L1 cache), but the concept applies to GPUs: replace ALU multiply with SRAM lookup for 1-2 bit weights. **VITRIOL plan:** Implement GPU LUT matmul (see [`docs/plans/T-MAC_LUT_MATMUL.md`](docs/plans/T-MAC_LUT_MATMUL.md) and [`docs/plans/COMPUTE_OPTIMIZATIONS.md`](docs/plans/COMPUTE_OPTIMIZATIONS.md)). |

### Extreme Quantization & Compute

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[T-MAC](https://github.com/microsoft/T-MAC)** (Microsoft, 2024) | Lookup-table-based matmul for low-bit models. On GPU: pre-compute all possible dot products (~768 for INT8 act × ternary weight) into shared memory LUT, replace 16-bit multiply with 2-cycle SRAM fetch. Bypasses ALU bottleneck entirely on Pascal. **Potential: 2-3× throughput for TQ1_0/IQ2 models.** |
| **[3LTERN](https://github.com/ELX987/3LTERN)** (ELX987) | W1.58A8 (1.58-bit ternary) CUDA kernel for Pascal. 16 weights packed per uint32, branchless decode via `bit0 - bit1`, `__dp4a` instruction on sm_61. Future optimization path for compute-bound layers. |
| **[Unsloth](https://huggingface.co/unsloth)** (Daniel & Michael) | Dynamic quantization formats (UD-Q2_K_XL) that are structurally superior to raw 1.58-bit. Ungated model distribution — their Qwen 3.6 releases don't require HF authentication. The model we target was quantized and distributed by them. |
| **[MoQE](https://arxiv.org/abs/2310.14713)** — Kim, Fahim, Awadalla (Microsoft, 2023) | MoE experts are robust to extreme low-bit quantization (2-bit) without losing base model coherence. Supports our asymmetric quantization approach. |
| **[BitNet b1.58](https://arxiv.org/abs/2402.17764)** — Ma, Wang et al. (Microsoft Research, 2024) | Ternary weights {-1, 0, 1} match FP16 perplexity, eliminating floating-point multiply. Future TQ1_0 format support. |

### Emulated Memory & Context Retrieval

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[LLM in a Flash](https://arxiv.org/abs/2312.11514)** — Alizadeh, Mirzadeh et al. (Apple, 2023) | Proved that windowing + zero-copy streaming from flash/host memory enables LLM inference on severely memory-limited hardware. Foundation of the RAM Shot base. |
| **[Fiddler](https://arxiv.org/abs/2402.14103)** — Kamahori, Gu, Zhu, Kasikci (2024) | Demonstrated that moving *activations* to CPU for MoE expert computation can be faster than pulling weights to GPU via PCIe DMA. Informs our `fiddler-cpu` mode. |
| **[SnapKV](https://arxiv.org/abs/2404.14469)** — Li et al. (2024) | Attention heads focus on clustered features; safe eviction of filler tokens reduces KV cache 8.2x without accuracy loss. Informs `--kv-mode sparse`. |
| **[H2O](https://arxiv.org/abs/2306.14048)** — Zhang, Sheng et al. (2023) | Pioneered dropping tokens from KV cache by identifying "Heavy Hitter" tokens that contribute most to attention scores. Informs `--kv-mode sparse`. |
| **[GraphRAG](https://arxiv.org/abs/2404.16130)** — Edge, Trinh et al. (Microsoft, 2024) | Replaced flat vector DBs with LLM-derived knowledge graphs for multi-hop retrieval (spreading activation). Informs our cascading memory retrieval. |
| **[Aider](https://github.com/paul-gauthier/aider)** — Paul Gauthier (2023) | Gold standard for tree-sitter AST-based repo mapping. Informs future AST code graphing for context injection. |

See `docs/OPTIMIZATION_PLAN.md` for the full V2 roadmap with implementation phases.
