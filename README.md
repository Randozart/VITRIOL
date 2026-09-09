# VITRIOL

<img src="assets/vitriol_logo.svg" alt="VITRIOL" width="200"/>

*"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*

*(Visit the Interior of the Earth, by Rectifying you will find the Hidden Stone)*

## Why VITRIOL?

VITRIOL is my attempt at using every optimization possible to run modern AI models on old hardware that would have no business running them otherwise. I am talking here specifically about my old desktop PC with a narrow PCIe bus, an outdated GPU, a CPU without AVX2 instructions, so offloading isn't even practical. With those constraints in mind I had a simple question: Could I stream directly from system RAM to GPU. It took a while, but I managed through a solid direct memory access system and a custom copy engine. Every optimization I am doing now is really one big mad computer science experiment to make old hardware punch above its weight, and possibly introduce those optimizations for newer hardware. This means VITRIOL is not a stable repo in the slighest. Not yet at least. I am trying to use the latest papers and research on LLM inference to squeeze every bit of performance out of my silicon, and I hope this comes to your benefit as well.

Regarding the name, in alchemy, vitriol was considered the ultimate catalyst for transmuting matter. This is something this application aims to do as well. To transform old hardware into a high performant AI transformer. Lead into gold. You know the drill. Aditionally, around the 15th century, the esoteric backcronym was formed: *"Visita Interiora Terrae Rectificando Invenies Occultum Lapidem"*. In a way, this is what we are doing. We are reaching down into the bowels of the computer, rectifying the data streams, and running inference on our newfound philosopher's stone. Yes, I know how incredibly silly this all sounds, but it makes me happy to be using archaic alchemical terminology. Regarding the logo: In alchemical texts and artwork, vitriol was often depicted as a "Green Lion devouring the Sun". This is a metaphor for sulfuric acid dissolving base metals (symbolically represented by the green lion) to extract and purify precious gold (the alchemical symbol for which is the sun).

## What VITRIOL Is

VITRIOL is a fork of [llama.cpp](https://github.com/ggml-org/llama.cpp) whose
goal is **optimal inference on machines that are normally too constrained to
run the model at all** — VRAM-starved GPUs, narrow PCIe buses, CPUs without
modern vector instructions, DDR3-era memory, single-digit-gigabyte boxes.
Every optimization is measured against one standard: does it make the model
run, or run faster, on silicon that the model has no business running on?

The operating point changes with the hardware:

- **Resident mode (default, `VITRIOL_MODE=off`).** When the model fits in
  combined VRAM, weights stay fully VRAM-resident. This is the residency rule:
  streaming a fitting model is a measured pessimization on narrow buses.
  Optimizations then target the *context* — TurboQuant KV, attention-probe
  sparse eviction, MTP speculative decoding — to stretch depth per byte.
- **Streaming mode (optional, `VITRIOL_MODE=stream`).** When weights exceed
  combined VRAM, MoE expert tensors live in page-locked host RAM and stream
  to the GPU over PCIe DMA, accelerated by an LRU VRAM cache, expert pinning,
  predictive prefetching, and an approximate expert output cache. This is the
  "RAM Shot" line of work — for the model class that does not fit, not for
  the one that does.

The same tree also ships **Officina**, a full-screen agentic coding workshop
welded to the engine over its HTTP surface, and an operational layer
(launcher, calibration tool, profiles, systemd units) that turns the engine
into a durable appliance instead of a process that dies with the first memory
crisis.

### What sets VITRIOL apart

| Capability | What it does | Upstream comparison |
|---|---|---|
| **RAM Shot streaming** | MoE experts in page-locked host RAM, GPU reads over PCIe DMA on demand | llama.cpp: all-or-nothing VRAM offload |
| **LRU VRAM cache + pinning** | Hot experts cached in VRAM; hot layers preloaded for fast-path kernels | Neither |
| **Predictive prefetch** | Cross-layer + temporal expert prediction, async DMA overlap (Fate/PreScope-style) | Neither |
| **Approximate expert output cache** | Reuses a routed expert's FFN output across consecutive tokens | Neither |
| **TurboQuant KV** | `tq3_0`/`tq3_1s`/`tq3_4s` — 3.5 bpw KV quant, ~22% smaller than q4_0, per-device overrides | llama.cpp: f16/q8_0/q4_0 only |
| **Probe-scored sparse eviction** | Attention-probe scoring decides which KV cells earn their keep; sinks + recent protected | vLLM PagedAttention keeps everything; H2O/StreamingLLM are static heuristics |
| **Dual-GPU tensor splits** | Mismatched GPU pairs share a model (`-ts 24,12` on a 3060 + 1070 Ti), per-device KV quant | Native multi-GPU exists; not tuned for mismatched consumer pairs |
| **MTP speculative decoding** | Embedded MTP head, `n_max=1` — +40% shallow, +31% at depth on Qwen3.8-27B | llama.cpp: external draft models |
| **Context lifecycle** | Slot save/restore, warm-resume on crash (~300 ms), sparse-KV preservation layer | Neither |
| **Durability / ops** | systemd units, oom-shield, hang watchdog, proactive bounce, flag-fingerprint journal, calibration tool | Neither |

**Depth, not window.** A context window allocation says nothing about usable
filled-context depth. Every VITRIOL capability claim carries a *filled* token
count measured by chunked/single-shot prefill plus decode-at-depth. Shallow
benchmarks do not certify deep-context operation.

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

## History

The project has lived three eras. Each one is preserved as history in
[`docs/VERDICTS.md`](docs/VERDICTS.md) — every dead idea carries its own
measurement and reason.

- **War I — fit the model (35B MoE era).** A Qwen3.6-35B-A3B needs ~12 GB of
  weights; the GPUs here had 8 and 12 GB. The answer was to *not* fit it: page-
  locked host RAM ("RAM Shot"), a custom copy engine, Chimera dual-backend
  (CUDA+Vulkan), expert streaming. It worked — 23.3 tok/s where the baseline
  was "doesn't boot". Then the project measured its own darling: on DDR3 +
  narrow PCIe, streaming a model that *fits* VRAM is a pessimization. That
  verdict became the **residency rule**, and most of War I's machinery became
  a tombstone — a correct answer for its era, outlived by hardware.
- **War II — afford the model (27B era).** A Qwen3.8-27B nearly fits across
  two GPUs. The answer was to buy the missing 8 GB (a secondhand GTX 1070 Ti
  beside the RTX 3060), then make the remainder behave: TurboQuant KV at
  3.5 bpw, LULL attention-probe scoring to decide which KV cells earn their
  keep, slot tenancy so tenants share one server without stealing context.
  Certified depth: **96,836 filled tokens @ 11.32 tok/s** — measured, not
  window-allocated.
- **War III — keep it alive (current).** A model that runs on a red-lined
  16 GiB box dies differently: OOM kills, swap-thrash hangs, task-queue jams.
  VITRIOL became an appliance: disk checkpoints that survive crashes, a
  sidecar that replays conversation warmth into fresh instances within
  seconds, a hang watchdog, a proactive bounce that restarts cleanly *before*
  memory exhaustion wedges the box, and an oom-shield that works backwards —
  since unprivileged users cannot protect their own processes, it marks
  everything else as more killable.

**Reading the repo:** `docs/ARCHITECTURE.md` is the single source of truth
for current behavior. `docs/VERDICTS.md` holds every dead idea and why it
died. Sections of this README dated to Wars I–II are preserved as history.

### Hardware assumptions (what's tuned where)

This tree is tuned for one specific machine: i7-3770 (no AVX2), 16 GiB DDR3 +
zram, RTX 3060 12 GiB + GTX 1070 Ti 8 GiB. Tensor splits (`ts 24,12`), KV
quant choices, cache caps, and sidecar thresholds are all *this-box* numbers.
They live in `profiles/` (personal) and `profiles/examples/` (generic
starting points) — re-tune there, not in code.

### Upstream posture

`main` is the canonical daily-driver branch in **both** repos: the outer
repo and the inner `llama.cpp/` fork. The inner `vitriol` branch is a frozen
pre-port archive; `master` tracks the fork's published state. Merge cadence:
when upstream grows something we want, not before.

## Quick Start

```bash
# 1. Clone with submodule (llama.cpp is pinned)
git clone --recursive https://github.com/Randozart/VITRIOL.git
# Or if already cloned: git submodule update --init --recursive

# 2. Build. The dual-GPU daily driver needs BOTH archs (sm_61 Pascal + sm_86 Ampere).
cd vitriol/llama.cpp && cmake -B build -DGGML_CUDA=ON -DGGML_NATIVE=ON \
  -DCMAKE_CUDA_ARCHITECTURES="61;86" \
  -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF \
  && cmake --build build -j$(nproc)

# 3. One-time capability grant (CAP_IPC_LOCK; optional on hosts where mlock is free)
./vitriol setup

# 4. Configure + calibrate
./vitriol calibrate --quick
./vitriol config

# 5. Run (or load a blessed profile)
./vitriol config load qwen38-mtp-131k
./vitriol run
```

The engine is designed to run as a managed service:

```bash
sudo systemctl restart vitriol-server.service   # the daily driver
```

The launcher refuses bare `vitriol serve` while the systemd unit is active —
restart via the unit, never bare.

## Configuration

VITRIOL's feature flags control memory, context efficiency, and retrieval.
Each has measurable trade-offs between throughput, context size, and recall
quality. Config lives in `~/.vitriol/config`; profiles (`vitriol config
save|load`) switch between blessed operating points.

| Flag | Effect | Notes |
|------|--------|-------|
| `VITRIOL_MODE` | `off` (default) resident · `stream` RAM Shot MoE streaming | Residency rule: stream only when weights exceed combined VRAM |
| `-ts 24,12` / `--tensor-split` | Split model across mismatched GPUs | Per-device VRAM headroom balancing |
| `--spec-type mtp --spec-draft-n-max 1` | MTP speculative decoding, single draft | `n_max=1` is load-bearing; `>=2` regresses (acceptance decay) |
| `--cache-type-k/-v tq3_0` | TurboQuant KV, 3.5 bpw | ~22% smaller than q4_0; per-device overrides via `VITRIOL_KV_QUANT[_K\|_V]_GPU<d>` |
| `--cache-ram 256` | Host RAM KV ring | Never pass `--cache-ram 0` (no readiness) |
| `--ctx-checkpoints 4` | RAM ring of in-slot KV copies | Never pass `--ctx-checkpoints 0` (heap corruption) |
| `[kv] score=probe, score_every=16` | Attention-probe sparse eviction | Sink + recent cells protected; `VITRIOL_KV_FLOOR` (eager sweep) off by default |
| `VITRIOL_POOL_RESET=1` | Rewind compute pools at graph end | Recovers ~20% usable depth |
| `-ngl 99 --main-gpu 0 -ub 64` | Full offload + batch size | Depth-certified operating point |
| `vitriol.pin_first_n_layers` | Pin hot layers' experts in VRAM | Streaming mode only |
| `vitriol.predictive_prefetch` | Cross-layer + temporal expert prefetch | Streaming mode only |
| `vitriol.lru_mb` | LRU VRAM cache size | Streaming mode only |

**Production speed profiles use q4_0 KV**; the master deep-context profile uses
`tq3_0`. Measure per profile — TurboQuant trades a decode penalty for depth on
some configs. See `docs/CONFIG_REFERENCE.md` for every flag and
`docs/RECOMMENDED_SETTINGS.md` for the blessed operating points.

**Memory mode** (the Flask shim + SQLite emulated-memory era) is **superseded
by Officina's built-in memory** — see the Officina manual.

## How It Works

### Resident mode (the daily driver)

Weights live in VRAM, split across both GPUs. The context is where the squeeze
is:

- **TurboQuant KV.** K and V caches quantized to 3.5 bpw (`tq3_0`) with
  Walsh–Hadamard-rotated quantizers, per-device asymmetric overrides.
- **Sparse KV eviction.** An attention-probe graph (q·K softmax, exponential
  decay) runs on a sibling GPU during decode lulls, every 16 steps. Lowest-
  scored middle cells are evicted first; sinks and recent cells are protected.
  The eviction counter surfaces as `⤓ Nk` in Officina's context row.
- **MTP speculative decoding.** The embedded MTP head drafts one token ahead;
  verification is parallelized. Single-token decode measured +40% shallow,
  +31% at depth on Qwen3.8-27B. Draft tokens are excluded from probe scoring.
- **Context checkpoints + slot save/restore.** Disk checkpoints (`slotN.bin`)
  carry KV + recurrent state; warm-resume after a crash takes ~300 ms. The
  persistence chain is gated on a text-only engine — loading a multimodal
  projector (`--mmproj`) shuts it off.

### Streaming mode (MoE that does not fit)

When weights exceed combined VRAM, MoE expert tensors live in page-locked
host RAM and the GPU reads them over PCIe DMA:

```
VITRIOL buffer type (CUDA experts)
  ├─ 1. Allocation
  │     mmap(hugepage) → mlock → cudaHostRegister (page-locked, DMA-accessible)
  ├─ 2. Model load: experts land in the host-RAM buffer, base model in VRAM
  └─ 3. Inference: is_host=true routes MUL_MAT_ID to CUDA; GPU reads experts
        over PCIe (~12 GB/s), accelerated by:
        ├─ LRU VRAM cache   — hot experts cached in VRAM, async DMA prefetch
        ├─ expert pinning   — first N layers' experts preloaded to VRAM
        ├─ predictive prefetch — cross-layer + temporal union, zero training
        └─ output cache     — approximate reuse of routed experts' FFN outputs
```

Streaming is gated per-op: hooks fire only when the expert tensor lives in a
VITRIOL buffer. Resident operation sees zero routing change.

### Dual-GPU operation

`-ts 24,12` splits the model across the RTX 3060 (12 GB) and GTX 1070 Ti
(8 GB). Per-device KV quantization (`VITRIOL_KV_QUANT_K_GPU<d>` etc.) and
per-device pin ranges (`VITRIOL_PIN_FIRST_N_LAYERS_GPU<d>`) let each card be
tuned to its own headroom. The calibration tool (`vitriol calibrate --quick`)
computes VRAM from GGUF tensor data — no hardcoded model constants.

### Distributed operation (row-split over RPC)

A second machine can host a slice of the layers over the llama.cpp RPC
backend (raw TCP on a Tailscale mesh): `[gpu] rpc_servers = <host:port>` and
a 3-way `-ts [boxB, GPU0, GPU1]`. Load `qwen38-distributed` profile to get the
measured split (5% to box B). The win is **capacity** — box B's large unified
memory holds KV/context box A's VRAM cannot — at a decode cost (~10-11 t/s vs
18.94 local; prefill is box-B bandwidth-bound). Full setup, measured table, and
limits: `docs/DISTRIBUTED_INFERENCE.md`.

## Hardware & Compatibility

### Tested daily driver

- **GPU pair:** RTX 3060 12 GiB (sm_86) + GTX 1070 Ti 8 GiB (sm_61), PCIe Gen3
- **CPU:** Intel i7-3770 (Haswell, no AVX2) — orchestrates, does not compute experts
- **RAM:** 16 GiB DDR3 + zram
- **OS:** Ubuntu 24.04, kernel 6.17+, NVIDIA driver 535.288.01, CUDA 12.2
- **Model:** Qwen3.8-27B-Q3_K_M (unsloth, qwen35 arch, embedded MTP head)
- **Certified depth:** 96,836 filled tokens @ 11.32 t/s (IQ3_S, `tq3_0`, ts 26,10)

### Likely works

- **NVIDIA GPUs CC ≥ 5.0:** Pascal, Turing, Ampere, Ada, Blackwell. The
  `cudaHostRegister` + PCIe DMA path is architecture-agnostic.
- **Maxwell (CC 5.x):** may lack some kernels — exclude or set
  `exclude_secondary = true`.
- **ROCm/HIP (AMD):** mechanical port of CUDA driver calls (`hipHostRegister`,
  `hipMalloc`); ggml already has `GGML_USE_HIP` guards.
- **Windows (NVIDIA):** NVCC + `cudaHostRegister` work identically; the CLI
  launcher needs a PowerShell wrapper.
- **Intel GPUs (SYCL):** a full VITRIOL SYCL integration exists (LRU cache,
  predictive prefetch, hot-expert profile, zero-copy mmap wrap) — for Intel
  Arc / oneAPI hardware. Requires a SYCL compiler + MKL; not built by default.

### Likely won't work

- **Intel iGPU (SYCL unified memory):** no discrete PCIe bus to cross —
  VITRIOL's trick is unnecessary there.
- **Apple Silicon (Metal):** unified memory; the entire model fits if it fits.
- **Nouveau:** lacks the `cudaHostRegister` GMMU page-table path.

## Durability

- **systemd units (system scope):** the engine runs under
  `/etc/systemd/system/` with a real `OOMScoreAdjust=-500`; `Restart=always`;
  a polkit rule lets the owner manage exactly the two vitriol units.
- **oom-shield:** unprivileged users cannot lower their own OOM score, so
  VITRIOL marks *other* consumers more killable (+300) — the kernel eats
  browsers first, never the engine.
- **Persistence sidecar:** startup restore, autosave with churn guard
  (frozen `/metrics` counters ⇒ nothing happened ⇒ skip), clobber protection
  (staged writes; an empty save can never replace a rich checkpoint),
  hang watchdog (~60 s health-deaf → restart), proactive bounce (clean
  restart before memory exhaustion wedges the box).
- **Flag provenance:** every launch emits a `VITRIOL-FINGERPRINT:` line and
  journals it with a per-field diff against the previous launch and the
  blessed operating point. Silent flag drift is a review blocker — config
  keys are flags too. Speed-bearing keys (`[spec]`, `[kv] score*`, `ts`,
  `ubatch`) are provenance-bearing.

## Performance

Numbers are depth-certified unless labeled shallow. **Window ≠ depth.**

### Current operating point (2026-09)

| Scenario | t/s | Notes |
|----------|-----|-------|
| Shallow, MTP n=1, ts 22,14 | 16.49 | A/B: +40% vs no-MTP (11.79) |
| Shallow, MTP n=1, ts 24,12 (blessed) | ~13.1 | Both GPUs ~81% VRAM |
| Depth 26K filled, ts 22,14 | 11.16 | Depth recert 2026-09-04 |
| Depth 36K filled, ts 22,14 | 10.45 | |
| Depth 26K, MTP off | 8.54 | MTP = +31% at depth |
| **Max certified depth** | **96,836 tok @ 11.32** | IQ3_S, `tq3_0`, ts 26,10 (2026-08-24) |
| 262K ctx (MTP off) | ~11.0 | Max native context profile |

### Historical (War I — single GPU, 35B MoE, Chimera)

| Config | t/s |
|--------|-----|
| PCIe x8, no VITRIOL | 5.7 |
| PCIe x16, before MTP/pin/Chimera | 8.9 |
| IQ2_M + MTP n=2 + pin 8 | 12.82 |
| + Chimera + CAP_IPC_LOCK | **23.3** |

See `docs/BENCHMARKS.md`, `docs/FINDINGS_2026-05-19.md`, and
`docs/plans/COMPUTE_OPTIMIZATIONS.md`.

## Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│ TENANTS     hermes-agent (slot 0, 73k)   ontic forge (slot 1, 8k) │
├──────────────────────────────────────────────────────────────────┤
│ ENGINE     llama.cpp fork ("main")                              │
│            residency rule: weights VRAM-resident by default      │
│            LULL attention-probe KV scoring + eviction + reset    │
│            TurboQuant KV tq3_0/1s/4s (3.5 bpw), per-device       │
│            MTP draft head (n_max=1)                              │
│            slot save/restore (--slot-save-path, warm resume)     │
├──────────────────────────────────────────────────────────────────┤
│ RUNTIME    scripts/vitriol launcher (profiles → argv, fingerprint)│
│            systemd units: vitriol-server + persistence sidecar   │
│            oom-shield · hang watchdog · proactive bounce         │
├──────────────────────────────────────────────────────────────────┤
│ TRUTH      libvitriol (Rust calibrator, GGUF-derived VRAM math)  │
│            certification reports: FILLED-depth benchmarks only   │
└──────────────────────────────────────────────────────────────────┘
```

```
        ┌────────────── RTX 3060 (12 GiB) ──────────────┐
        │  ts split 24: model layers + KV (tq3_0)       │
        │  compute buffers · MTP head (resident mode)   │
        └───────────────────────────────────────────────┘
                          ▲ PCIe
        ┌────────────── GTX 1070 Ti (8 GiB) ────────────┐
        │  ts split 12: model layers + KV (tq3_0)       │
        │  LRU pool (stream mode) · probe scorer        │
        └───────────────────────────────────────────────┘
                          ▲ PCIe
        ┌──────────── CPU / 16 GiB DDR3 + zram ─────────┐
        │  VITRIOL host buffer (stream mode: experts)   │
        │  slot checkpoints (slotN.bin, warm resume)    │
        └───────────────────────────────────────────────┘
```

## Project Structure

```
├── vitriol                  ← CLI entry point (symlink to scripts/vitriol)
├── scripts/
│   ├── vitriol              ← launcher: config TUI + run + serve + profiles
│   ├── build-llama-server.sh
│   ├── vitriol-oom-hardening.sh
│   └── lull_slot_persist.py ← persistence + watchdog sidecar
├── libvitriol/              ← Rust calibrator (GGUF parser, VRAM estimator)
│   ├── src/{gguf,probe,estimator,main}.rs
│   ├── gguf_reader.py       ← Python fallback
│   └── sweep_controller.py  ← automated HTTP benchmark sweeps
├── profiles/                ← canonical configs (personal + examples/)
├── officina/                ← the built-in coding workshop
├── llama.cpp/               ← Git submodule (pinned, "main" branch)
│   └── ggml/src/ggml-cuda/
│       ├── vitriol-buffer.{cpp,h}          ← RAM Shot buffer type
│       ├── vitriol-cuda-integration.{cpp,h}← LRU + pin + predictor + output cache
│       ├── vitriol_copy_engine.{cpp,h}     ← copy-engine DMA (Phase 1)
│       └── tq3-native.cu, turbo-wht.cu     ← TurboQuant KV kernels
├── vitriol-daemon/          ← experimental NVMe→GPU DMA kernel module
├── systemd/                 ← unit files (system scope)
├── docs/                    ← living documentation
├── .opencode/plans/         ← agent session reports (the lab notebook)
└── EXPERIMENT_LOG.md
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

### Chunked Recurrence Inference

| Paper / Project | What We Learned |
|-----------------|-----------------|
| **[SwarmLLM](https://github.com/Nehanth/swarmllm)** (MIT) — Narendrula (2026) | A WebGPU/WebRTC engine running the same model class VITRIOL serves (Qwen3.8-27B, Gated-DeltaNet + MTP) — rejected as a *runtime* (browser stack, none of VITRIOL's CUDA machinery runs there) but mined as *inspiration*. Two measured, golden-test-gated techniques transfer to CUDA: (1) **chunked Gated-DeltaNet prefill** — E1–E7 running-product decays + C-step triangular solve breaks the serial token recurrence (verified vs f64 oracle 4e-15; 2.6–3× on the recurrence kernel, groundwork for 16+ column passes); (2) **register-resident recurrence tiling** (private array with literal indices, shared-mem partial reduce, 1.7–2.9× on the delta kernel). Also: row-stationary packed-nibble prefill GEMM (bank-conflict-padded shared tile), device kernel autotune with a 3% noise guard, 2-D dispatch for tall matvecs past the 65,535-workgroup cap. Full record: `.opencode/plans/swarmllm-mining-assessment-2026-09-08.md`. Their measured rejections (Q4 KV → −92.5% prefill; external draft model → vocab mismatch) validate VITRIOL's own verdicts. |
| **[Gated Delta Net](https://arxiv.org/abs/2412.06464)** — Yang et al. (2024) | The chunkwise-parallel delta-rule prefill identity (Sec 3.3) that E1–E7 above instantiate: `(I+A)D=R` unit-lower substitution turns the serial state recurrence into C independent reductions. Primary citation for the chunk algorithm; SwarmLLM is the verified implementation reference. |

### Agent Harness & Context Efficiency (Officina)

| Project | What We Learned |
|---------|-----------------|
| **[Claude Code](https://github.com/anthropics/claude-code)** (Anthropic) | Context editing: evict consumed tool results behind a small keep-window instead of carrying them forever; externalized task lists (TodoWrite) that survive compaction because they live on disk, not in history; permission-gate UX. Patterns only — no code. |
| **[Aider](https://github.com/Aider-AI/aider)** — Paul Gauthier | tree-sitter symbol graph + PageRank repo map: ~500 structural tokens replace 5–10K of blind file reads. Implemented in `repo-map`. |
| **[OpenCode](https://github.com/opencode-ai/opencode)** | Per-edit diagnostic loop (check → auto-repair → re-check, ~300-token injected verdicts) and per-turn git snapshots under a private ref. Implemented in `diagnostics-loop` and `snapshot`. |
| **[Crush](https://github.com/charmbracelet/crush)** v0.91.2 (FSL-1.1-MIT) | Small-model compaction lane: summarization runs on the fast local model while the big model only handles agent turns (our `small-lane`, mellum2 @ 11–12 t/s); Crush-grade live status presentation. PATTERNS only, no code (license incompatible). |
| **[RTK](https://github.com/rtk-ai/rtk)** | Entry-side reduction of command output (exit status + error lines + tail, 60–90% smaller) before it costs a context token; full payload parked on disk. Implemented in `rtk-output`. |
| **trismegistus / hermes-plugins** (owner-authored, MIT) | Injection guard (untrusted-content discipline), caveman deterministic compressor (−65% measured), memory-extractor candidate rules with human curation. Ported to TypeScript, headers cite origin. |
| **[pi-coding-agent](https://github.com/earendil-works/pi-coding-agent)** (MIT, pinned 0.83.0) | Runtime substrate: extension/event API (`tool_call` gates, `context` middleware, tail ride-alongs), fork-rebrand hook. Library per the First-Party Mandate — mined, never patched in place. |
| **KV-cache prefix discipline** (empirical, this repo) | Per-turn guidance travels as hidden tail messages, never system-prompt edits — editing the prefix invalidated 120K cached tokens mid-conversation (caught with llama.cpp request tracing). See `.pi/extensions/_shared/inject.ts`. |

Measured result of the stack (2026-08-31): **~20K live context against ~200K
cumulative offloaded** in a working session (~10:1 discard ratio) — full
record in `.opencode/plans/officina-context-efficiency-record-2026-08-31.md`.

See `docs/PROVENANCE.md` for the file-level citation registry, and
`docs/provenance/` for per-module headers. VITRIOL is licensed
`Apache-2.0 OR MIT` (see `LICENSE`, `LICENSE-MIT`); upstream projects are
mined for insight, never depended on as runtimes, and GPL sources are
re-derived only.

See `docs/OPTIMIZATION_PLAN.md` for the full V2 roadmap with implementation phases.