# Nemotron 3.5 Super VL + MiniCPM-V-4.7 Bring-up Plan

**Created**: 2026-10-16 12:22
**Status**: active
**Direction** (user-approved): merge upstream PR #30520 first, CPU-MoE baseline before any DMA revival; both text and vision profiles; MM1_M first then Q4_K_M; MiniCPM-V-4.7 included in scope.

## Progress log

- **2026-10-16 12:40 — Stage 0 done.** DEVIATION from plan text: instead of a
  full 849-commit `upstream/master` merge (5 weeks of churn, high conflict
  surface), cherry-picked the two model PRs surgically:
  - `a16b6318a` = PR #29416 (MiniCPM-V 4.7). 4 conflicts resolved:
    `constants.py` (kept incoming keys), `llama.h` (kept fork — fork has no
    Extended batch API), `llama-model.cpp` (merged: fork comment + PR's
    `LLAMA_ROPE_SECTION_ORDERS`), `llama-graph.cpp` (adapted PR's `set_input`
    to fork's ubatch: no `is_mixed`/`type` — fork batch broadcasts text pos
    `[p,p,p,p]`, graph normalizes 4th slot to 0; ZYXT permute kept for media).
  - `4fcdd0e92` = PR #30520 (Nemotron 3.5 Super VL + Bolt). 6 conflict hunks:
    dropped LiLiCorr draft conversion + dflash LiLiCorr hunks (upstream-only
    deps: `lilicorr_pass_hidden_proj`, `llm_graph_input_embd_h`, `embd_state`);
    kept fork's DSpark path; `modelopt_mixed` folded into fork's modelopt
    branch; `PROJECTOR_TYPE_NEMOTRON_OMNI` kept, `LING3VL` dropped (not in fork).
  - Build green (`61;86`), `test-batch-alloc` + `test-mtmd-impl` 0 failures,
    server reports commit `4fcdd0e92`. Config fingerprint regression check:
    byte-identical to `fingerprint.last` ✓.
- **2026-10-16 12:45 — Stage 4 (MiniCPM-V-4.7-1B) VERIFIED**:
  - text: `/v1/chat/completions` generates (fingerprint `b1792-4fcdd0e92`)
  - vision: `llama-mtmd-cli` + `mmproj-model-f16.gguf` correctly described a
    generated test image (red square / blue circle / "OPTANE" text) —
    `minicpmv4_7` projector graph works end-to-end. Minor M-RoPE
    `non-consecutive token position` warning observed, output correct.
  - sha256: both files verified vs tree-API `lfs.oid`.
- **2026-10-16 13:10 — Optane utilization check (user question)**: efficient —
  `~/Models` symlinks to `/scratch/models` (XFS P4800X), so all GGUFs (incl.
  in-flight Nemotron) + `kvtier` (143 G) + 64 G swap already ride Optane;
  DMA store tier deliberately dormant (`mode=off`).
- **2026-10-16 13:20 — launcher feature**: `[model] n_cpu_moe` (0..N →
  `--n-cpu-moe`) + `[model] mmap` (on/off → `--mmap`/`--no-mmap`, default
  off = current behavior) wired through both parse loops, writer template, all
  4 serve branches, and all 3 fingerprint sites (`ncmoe=`/`mmap=` ride only
  when active — blessed fingerprint unchanged).
- **Downloads** (`hf_fetch.py`: ranged-resume + sha256 vs `lfs.oid`): MiniCPM
  Q4 + mmproj ✓, Nemotron mmproj BF16 ✓, IQ1_M shard1 ✓; shard2 (46.1 G) in
  flight → `/scratch/models/`.
- **2026-10-16 15:30 — Addendum A: model-load fixes + GATE RESULTS.**
  - Shard layout: shard1 = 7.5 MiB **metadata-only** (0 tensors), shard2 =
    46.1 G with all 781 tensors. Loader handles it; our earlier "(7,6M) [MTP]"
    size display = shard1 only (cosmetic).
  - Engine fix 1 (`f4518576a`, inner): Unsloth UD quants omit
    `attention.layer_norm_rms_epsilon` (they emit only
    `attention.layer_norm_epsilon`) → required-key load failure. Fallback to
    1e-5 (HF config value) when absent.
  - Engine fix 2 (`90cb995a1`, inner): MTP block (blk.88) in Nemotron 3.5 has
    **latent MoE** (`ffn_latent_down/up`, experts 1024-wide) — upstream
    graph_mtp lacked the bottleneck (2 unclaimed tensors + would-be
    shape mismatch). Registered in `load_arch_tensors`, applied in
    `graph_mtp`.
  - Stale Qwen slot mirror renamed `/scratch/kvtier/slot0.bin` →
    `slot0.bin.qwen38-bak` (was rejected on Nemotron boot; kept for rollback).
  - **Path 1 — CPU-MoE baseline** (`nemotron35-super-vl` profile, ncmoe=88,
    mmap, threads 8): loads ~380 s, correct output. Warm decode
    **0.70–0.83 t/s** (4×256 tok), prefill 3.23 t/s @1452 tok batched,
    prompt cache works (1453 cached → 2.5 s). **GATE FAIL** (<2 t/s).
  - **Path 2 — DMA store-tier revival** (`nemotron35-super-vl-store`):
    root-caused the dormancy chain:
    1. expert buft (`vitriol_get_expert_buffer_type`) only exists in
       `VITRIOL_MODE=stream`; SYNC/ASYNC are placement no-ops.
    2. buft intercepts only tensors **already flagged for host placement**
       → requires `n_cpu_moe > 0` as a placement FLAG (CPU never computes —
       weights stream Optane→pinned bounce→VRAM LRU). With ncmoe=0+ngl99,
       experts hit plain cudaMalloc (31.9 G OOM on CUDA1).
    3. `disk_offload=1` tier pins the whole expert range
       (mmap+cudaHostRegister) → impossible >RAM; the raw-zone **store**
       tier (mkfs, split-aware, `--scan`) is the designed one: 38 GiB store
       built in 2m43s (`/scratch/vitriol-store-nemo.bin`), loader **skips
       the 46 G GGUF expert read**, load 145 s, lazy per-expert pin budget
       (`max_locked_mb=8192`), VRAM LRU 1024 M/device.
    4. Launcher: `vitriol.store/store_path/store_source/max_locked_mb` keys
       wired (5 exec sites + fingerprint `st=/mlm=`), profile
       `nemotron35-super-vl-store` = working config (ts 24,24, ncmoe=88 as
       placement flag, mmap on).
    - Result: warm decode **0.73–0.82 t/s** — statistically identical to
      CPU-MoE. **GATE FAIL** (<2 t/s).
  - **Verdict: gate fails on BOTH paths; the binding constraint is Optane
    bandwidth, not the path.** Decode needs ~1 GB expert bytes/token
    (41 MoE layers × ~23 experts × ~1 MB); P4800X delivers ~750 MB/s
    effective at that granularity → ~0.75 t/s ceiling for ANY per-token
    streaming scheme on this device. CPU-MoE hides reads under compute and
    lands on the same number. Prefill: CPU-MoE batched GEMM wins (3.2 vs
    ~1 t/s).
  - Hardware escape hatches (not present on box): 2nd Optane / NVMe stripe
    (~×2-3), VRAM-resident hot tier ≥12 G with skewed-expert LRU hits,
    or a smaller model. Not pursued today.

## Goal

Get `nvidia/NVIDIA-Nemotron-3.5-Super-VL-120B-A12B` running on this host, and
`openbmb/MiniCPM-V-4.7-1B` (+ optionally 35B-A3B), then measure and gate whether
the dormant VITRIOL DMA streaming architecture is worth reviving for the 120B.

## Model facts (from HF config + model card, 2026-10-16)

**Nemotron 3.5 Super VL 120B-A12B** (released 2026-10-15):
- HF arch: `NemotronH_Omni_Reasoning_V3`, `model_type nemotron_h_omni`
- Text: `NemotronHForCausalLM` / `nemotron_h` — hybrid Mamba-2 / attention / MoE,
  512 routed experts top-22 + 1 shared, **LatentMoE** (`moe_latent_size 1024`),
  native MTP (1 layer), 1M max ctx, vocab 131072, hidden 4096
- Vision: C-RADIO (RADIO v2.5-H, ViT-H patch16, 7 registers), InternVL-style
  image tags, OpenMDW-1.1 license (model), upsteam merged as llama.cpp #30520
- Sizes: NVFP4 ~79 GB · Q4_K_M 70.1 GB · UD-IQ1_M 46.1 GB (smallest)
- Day-0 GGUFs: `ggml-org/`, `unsloth/`, `lmstudio-community/` on HuggingFace
- Upstream PR: https://github.com/ggml-org/llama.cpp/pull/30520 (merged 2026-10-15)

**MiniCPM-V-4.7** (openbmb, Apache-2.0):
- 1B: LM ~1.01B (Qwen3.5 hybrid, 24 layers, 18 linear + 6 full attn, ctx 262144)
  + vision 0.55B (SigLIP + vit_merger + DownsampleMLP) — Q4_K_M **641 MiB**,
  mmproj 1 GiB. Image + video, no audio. Arch `MiniCPMV4_7ForConditionalGeneration`
  / `minicpmv4_7`; official GGUF: `openbmb/MiniCPM-V-4.7-1B-gguf` (arch `qwen35`).
- 35B-A3B MoE sibling: Q4_K_M **19.7 GiB** — candidate to fit combined VRAM resident.
- Upstream PR: https://github.com/ggml-org/llama.cpp/pull/29416 (merged 2026-10-10, b11551).

## Host reality (2026-10-16 probe)

- GPUs: **2× RTX 3060 12 GiB (sm_86, combined 24 GiB)** — AGENTS.md still
  documents "RTX 3060 + GTX 1070 Ti"; hardware drift flagged, live `ts 27,21`
  profile + `61` CMake arch need separate re-validation.
- Host RAM: **31 GiB total, ~5 GiB available**, swap 95 GiB (zram 31G + NVMe 64G).
- Optane: `nvme0n1` Intel P4800X 480G, `/scratch` 375G free — DMA store-tier capacity OK.
- Live config `~/.vitriol/config`: `[vitriol] mode = off` (residency rule).

## Feasibility verdict (research record)

- **DMA arch compat: YES** — expert tensors are `ffn_*_exps` → `MUL_MAT_ID`,
  routed by the stream arm (`ggml-cuda.cu:2090-2120`); LatentMoE `ffn_latent_*`
  is plain MUL_MAT (commit `328874d05`) → stays resident; LRU quant-agnostic.
- **Host hostile**: pinned-RAM tier impossible (31 GiB < 46-70 GB model);
  store-tier mkfs pass-2 bug OPEN (`ik-ab-2026-09-17`→`2026-09-27`); 16-token
  cliff OPEN (`gateb-close-perf-2026-09-26.md`); LRU thrash risk (~900 expert
  slices/token vs 2 GiB pool); mmproj kills persistence + governor RESIZE +
  Optane prompt mirror. Expected streaming perf: low single-digit t/s optimistic.
- **CPU-MoE baseline** (`--n-cpu-moe`): zero new code, but 46-70 GB experts
  page from NVMe swap → OOM discipline mandatory (engine unit oom_score_adj -500).

## Stages

### Stage 0 — upstream sync (unlocks both models)
1. `git -C llama.cpp fetch upstream` — confirm #30520 + #29416 in history.
2. Merge `upstream/main` into local `main` (+9 vs origin). Expected conflict
   files: `conversion/nemotron.py`, `conversion/minicpm.py`,
   `gguf-py/gguf/constants.py`, `tools/mtmd/clip-impl.h`, `clip.cpp`,
   `src/models/nemotron-h.cpp`, `src/llama-model.cpp`. Preserve VITRIOL patches
   (MTP wiring, `ffn_latent` buft-probe fix, hybrid-arch gates).
3. Canary greps: `NEMOTRON_OMNI`, `minicpmv4_7`, `rope.section_order`,
   `nemotron-omni.cpp`.
4. Rebuild `-DCMAKE_CUDA_ARCHITECTURES="61;86"`.
5. Commit stage alone.

### Stage 1 — Nemotron baseline, two profiles
1. Download Unsloth **UD-IQ1_M** (46.1 GB, 3 shards) + `mmproj-BF16.gguf`;
   sha256 verify against tree-API `lfs.oid` (HF CDN stall rule — ranged resume).
2. Bring-up: `llama-server -ngl 99 --n-cpu-moe <all MoE layers>`,
   `VITRIOL_MODE=off`; kill stale servers first; check GPU is free (user warned
   another process may hold it).
3. Profiles via `vitriol config`: `nemotron35-super-vl` (text-only — persistence
   intact) + `nemotron35-super-vl-vision` (mmproj, `vis=on`, persistence
   knowingly off).
4. Fingerprint every launch; results embed full argv.
5. After IQ1_M sanity: switch to **Q4_K_M (70 GB)**, re-bench.

### Stage 2 — measure & gate
- 64-token ×3 + one FILLED-context pass, both quants.
- **Gate**: ≥2 t/s → profiles stand. <2 t/s → Stage 3 (DMA revival eval)
  vs alternatives (Nemotron Bolt 74B A9B, MiniCPM-35B-A3B, off-box vLLM day-0).
- Report appended to this file (Addendum A).

### Stage 3 (conditional) — DMA revival for 120B
Only if gate fails. Pre-work: 16-token cliff, store-mkfs pass-2, Optane/mmap
tier only (no pinned RAM), LRU sizing vs expert working set. Residency rule
justified (weights ≫ 24 GiB). Experiment record must document justification.

### Stage 4 — MiniCPM-V-4.7 (parallel after Stage 0)
1. Verify #29416 landed; if delta remains, port: `conversion/minicpm.py`
   (`MiniCPMV4_7TextModel`/`VisionModel`), `constants.py` projector enum +
   `rope.section_order`, `clip.cpp` dispatch + `v.tok_embd_sep`, `mtmd.cpp`
   canvas M-RoPE rows, `conversion/__init__.py` registration.
2. Bring up **1B Q4_K_M resident** — vision testbed with full persistence stack.
3. Optional: **35B-A3B Q4_K_M (19.7 GiB)** resident vision+MoE candidate,
   bench vs Qwen3.8-27B.

## Risks / open items

- 120B on 31 GiB RAM = swap-thrash / OOM-kingpin risk → smallest quant first,
  OOM discipline (AGENTS.md rule 3), never bare `serve`.
- NVFP4 mmq kernels on sm_86 unverified (moot: using Q4/IQ quants).
- GPU may be occupied by another process (user notice 2026-10-16) — check
  `nvidia-smi` before any load; defer benches if busy.
- Hardware drift (2×3060) — separate re-validation of live profile required.
- MiniCPM 1B text gguf arch `qwen35` should load even pre-#29416; vision needs it.

## Decision log

- 2026-10-16: user chose "merge #30520 + CPU-MoE baseline first"; "both profiles
  text/vision"; "IQ1_M first, then Q4_K_M"; "include MiniCPM-V-4.7".
