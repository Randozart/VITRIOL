# Plan: Three-tier expert streaming (VRAM/RAM/Optane) + Qwen3.8-Flash-Next qualification

**Created**: 2026-09-23 08:02
**Status**: executing
**Owner**: opencode (user-directed)

## Objective

Qualify one 100GB+ MoE for daily-driver use on 2x RTX 3060 12GB + 32GB RAM +
Optane 900p 480GB via VITRIOL three-tier expert streaming, and (user-mandated,
parallel) implement Phase 2 direct-doorbell DMA in the copy-engine path.

## Decisions locked (user, 2026-09-23)

1. Primary model: **Qwen3.8-Flash-Next** (Qwen3.8 Flash). DeepSeek-V4.1-Flash
   and GLM-5.3-Flash remain candidates; not first.
2. **No RAM upgrade** — 32GB is the binding budget; file-backed mmap + page
   cache is the RAM tier, not full RAM Shot.
3. **Community quants** (converter lacks qwen4exp/deepseek4/glm-dsa).
4. Optane `/scratch` (XFS) is the model store; keep swap layout.
5. **Build doorbell Phase 2 now** as a parallel workstream (measure-first
   rejected by user).

## Hardware facts (measured 2026-09-23)

- 2x RTX 3060 12GB (second card seated; `/dev/nvidia0/1`), driver 615.71.09
  open kernel module, CUDA 13.4. sm_86.
- RAM 31GiB total (~13GiB free during prior session).
- Optane 900p (SSDPED1D480GA, 447G): nvme0n1p2 `/scratch` XFS, ~368G free.
  Gen3 x4 ~2.2GB/s seq; ~10us read latency (user's claim, unverified this
  session — verify in A3 with probe).

## Model facts (verified 2026-09-23)

- **Qwen/Qwen3.8-Flash-Next**: 125B main + 4B MTP + 51B n-gram embedding
  (20M rows x 2560, bigrams/trigrams @ layer 2), 6B active/token, 512 experts
  (10 routed + 1 shared), 48 layers, hidden 2560, embd 248320, GDN:QSA = 3:1,
  arch `qwen4exp`, native 262,144 ctx (1M YaRN), multimodal (vision encoder).
- Vendor: **unsloth/Qwen3.8-Flash-Next-GGUF** (arch qwen4exp — matches fork's
  `LLM_ARCH_QWEN4EXP`). Sharded 3-way (Q3) / 4-way (Q4).
- Quant ladder: UD-IQ1_S 72.5 / UD-IQ1_M 74.5 / UD-Q2_K_XL 78.9 /
  **UD-IQ3_XXS 82 / UD-Q3_K_XL 90** / UD-Q4_K_XL ~105 GB (est).
- User floor: **3-bit**. A1 picks UD-Q3_K_XL (90G, standard Q3_K family —
  matches repo's 27B Q3_K_M precedent); fallback UD-IQ3_XXS (82G).

## Fork support (verified 2026-09-23)

- `llama.cpp/src/models/qwen4exp.cpp` (1283 lines): GDN/DeltaNet
  (`llm_build_delta_net_base`), QSA (`build_qsa_top_k`, lightning indexer),
  PLE n-gram hash embeddings (host-side hash + gather, `:1024`), HC, MTP.
- Converter (`convert_hf_to_gguf.py`) has no qwen4exp entry — community GGUF
  only (decision 3).
- N-gram table is a gather (not MUL_MAT_ID): VITRIOL expert buft does NOT
  stream it. Placement (VRAM vs host) determined empirically in A2; if VRAM,
  gap work = host routing for PLE tables.

## Three-tier mapping (all mechanisms pre-existing)

| Tier | Mechanism | Control |
|---|---|---|
| Hot: VRAM | `vitriol_lru_ensure` + `cuMemcpyHtoDAsync` on dedicated stream | `VITRIOL_LRU_MB[_<dev>]` |
| Mid: RAM | lazy per-expert mlock + cudaHostRegister | `VITRIOL_MAX_LOCKED_MB` |
| Cold: Optane | file-backed mmap, page-cache faults | `VITRIOL_DISK_OFFLOAD=1` + `VITRIOL_MODEL_PATH` |
| Prefetch | cross-layer + temporal predictor, fire-and-forget on LRU stream | `VITRIOL_PREDICTIVE_PREFETCH=1` |
| Dense base | first-N-layer expert pinning | `VITRIOL_PIN_FIRST_N_LAYERS[_GPU<d>]` |

Residency rule (AGENTS.md rule 1) satisfied: all candidate models are
100GB+ class, far above 24GiB combined VRAM — streaming justified, record
the justification in every experiment record.

## Workstream A — model qualification (value path)

- **A1** Fetch UD-Q3_K_XL shards -> `/scratch/qwen38fn/`. Ranged-resume
  downloader (HF CDN stalls ~16MiB on this host), verify sha256 against HF
  tree-API `lfs.oid` (resolve-HEAD etag is xetHash, NOT sha256 — AGENTS 6).
  Commit: fetch tool run notes only if new.
- **A2** Load-verify: `llama-server` smoke (kill stale servers first),
  confirm arch `qwen4exp`, n-gram tensor placement, MTP/MTP-head detection,
  KV types, 3-shard stitch. Record fingerprint. GATE: clean load + 1 token.
- **A3** Three-tier profile: `VITRIOL_MODE=stream`, `VITRIOL_DISK_OFFLOAD=1`,
  `VITRIOL_MAX_LOCKED_MB=10240`, `VITRIOL_LRU_MB` split across 2 devs,
  `VITRIOL_PREDICTIVE_PREFETCH=1`, `ts 24,24`, ub 64. Profile file under
  `profiles/qwen38fn-stream/`. Config write => fingerprint diff (AGENTS 4).
  Also: Optane bandwidth probe (seq + rand read on /scratch) — none exists in
  `libvitriol/` today; add to `probe.rs` if probe stage reaches calibrator.
- **A4** Sweep (lock budget x LRU x prefetch) + shallow 3x64 bench, then
  depth-fill certification (window != depth — AGENTS 5). Report:
  `.opencode/plans/qwen38fn-stream-cert-*.md` with fingerprints + full argv.

## Workstream B — doorbell Phase 2 (user-mandated, parallel, gated)

Findings: `vitriol_copy_engine.{h,cpp}` is a Phase-1 stub only
(`cuMemcpyDtoDAsync` + blocking `cuStreamSynchronize`, unwired). Phase 2
("direct doorbell, planned") unimplemented. User rejected measure-first;
building now.

- **B1** Correct class defs: RTX 3060 copy engine = **NV_C6B5**
  (`AMPERE_DMA_COPY_A`, clc6b5.h local at
  `/usr/src/nvidia-615.71.09/src/common/sdk/nvidia/inc/class/clc6b5.h`),
  not Pascal C0B5. Register offsets identical (sem 0x240, LAUNCH_DMA 0x300).
  Update header (C6B5 primary, C7B5 second CE noted). PROVENANCE: header
  already cites open-gpu-kernel-modules (MIT — compatible, attribution
  retained). Commit after build green.
- **B2** **Feasibility spike — hard abort gate** (standalone harness, not in
  engine): allocate CUDA stream, locate UserD page + GPFIFO + channel token
  from `/proc/self/maps` (CUDA 13.4 UMD layout), build C6B5 pushbuffer, ring
  doorbell @ UserD+0x90, spin metapage fence with timeout. Abort criteria:
  ring not findable / driver rejects / channel hang / GPU error counter
  increments => doorbell capped, verdict written to `docs/VERDICTS.md`,
  CUDA-API path stands. Success => B3.
- **B3** Real impl: `vitriol_ce_build_pushbuffer` / `vitriol_ce_submit` /
  `vitriol_ce_wait` (async — remove `cuStreamSynchronize` stub semantics),
  wire into `vitriol_lru_ensure` behind `VITRIOL_CE_DOORBELL=1`; default
  stays `cuMemcpyHtoDAsync`. Commit.
- **B4** A/B: per-transfer latency + end-to-end t/s, doorbell vs default
  under real expert load. Keep doorbell only if it wins; report to
  `.opencode/plans/`. Commit.

## Risks (accepted)

1. Channel-hijack premise unverified; GeForce + 615 may block or race the
   UMD on a shared GPFIFO (B2 gate — abort is a valid, pre-authorized
   outcome; user approved the abort criteria in the approved plan).
2. Raw doorbell has no error path: malformed command = silent GPU hang.
   Spike runs first on a disposable CUDA context with nvidia-smi error
   counters watched.
3. Driver update breaks /proc/self/maps UMD layout assumptions.
4. Expected doorbell upside <=1ms/token at 250-500ms/token budget;
   tiering carries the real wins. Doorbell loses nothing if it aborts.
5. Qwen3.8-Flash-Next is multimodal: keep mmproj unloaded (persistence
   gate — AGENTS, vis=off contract).

## Conformance

- Commit per completed stage (AGENTS workflow 1).
- Every launch emits `VITRIOL-FINGERPRINT:` (rule 4); every bench result
  embeds argv; config writes followed by fingerprint diff.
- Reports in `.opencode/plans/` with ISO 8601 timestamps (rule 1-4).
- Build targets `CMAKE_CUDA_ARCHITECTURES="61;86"` (key facts).
- Kill stale `llama-server` before any launch (rule 3).
