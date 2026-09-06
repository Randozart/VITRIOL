# The Ideal Picture: Flash-Next / 27B-class at Top Speed on Panther Lake

2026-09-06 ~23:30 CEST
Status: PLAN (evidence-backed, execution not started)
Author context: session 2026-09-06 (dispatch chain fix + zero-copy wrap landed;
E27/E28 committed `5fb6a67a9`, `9eba8229e`, `168d8ef`, `76fbba4`).

## 0. Measured device inventory (no assumptions)

| Engine | Measured | Evidence |
|---|---|---|
| CPU | 16c/16t (no SMT), 5.4 GHz, 24 MB L2, 63781 MiB RAM visible | lscpu, common_param log |
| Arc B390 iGPU | **96 EUs (4 Xe3 slices)**, LPDDR5X shared, SYCL reports 53.8 GB device memory / 33.5 GB free at load | clinfo, wrap6 -lv5 log |
| NPU5 "AI Boost" (8086:B03E) | **50 INT8 TOPS, native FP8 E4M3/E5M2**, FP16 at 1/2 rate, 12K MACs @ 2050 MHz, 3 NCE, 4.5 MB scratchpad; idle D3hot since boot | lspci, /sys/class/accel/accel0, OpenVINO 2026.3.1 enumerates CPU+GPU+NPU |
| NPU stack | `libze_intel_npu.so.1.35.0` + OpenVINO runtime w/ NPU plugin+compiler installed; `openvino_genai` NOT installed; no torch/transformers/optimum (venv needed) | find/dpkg/python checks |
| LPDDR5X | 62.3 GB shared by all engines (~85-90 GB/s effective) | E18, prior sessions |
| NVMe | 4TB Gen4 x4 "M.2 (P80) 4TG2-P", ~7 GB/s proven in prefill | /sys/class/nvme, E28 bench |

Prior standing facts that hold: iGPU+NPU do NOT add bandwidth (shared LPDDR);
dense 27B @ >6 t/s is bus-impossible on this SoC.

## 1. Current Flash-Next baselines (this box, E28)

| path | pp512 | tg64 | limiter |
|---|---|---|---|
| CPU-only, UD-Q2_K_XL | 82.1 | 7.69 | NVMe streaming 845-897 MB experts/token at ~6.9 GB/s |
| VITRIOL wrap LRU 4 GB | 76.6 +/- 7.6 | 3.36 +/- 0.01 | LRU 42.6% hit; misses disk-bound at ~1.7 GB/s effective |
| VITRIOL LRU 8 GB / async DMA | - | 2.94 / 2.95 | regressions (pool steals page cache; lru_sync single-slot bug) |

Per-token byte math (UD-Q2_K_XL): routed experts ~897 MB + dense ~500 MB
(dense: shared experts 61 MB + attention/indexer ~264 MB + lm_head ~100-150 MB).
Dense portion fits page cache entirely (26.5 GB); routed experts are the disk
traffic. Compute (3B active + shared) is ~10x faster than the disk - disk is
the wall on every route that streams.

## 2. THE key structural fact: the 51.2B n-gram table anchors every quant

Flash-Next true composition (unsloth model card, verified):
- 125B routed MoE (512 experts, 10 routed + 1 shared active, 6B active)
- **51.2B n-gram PLE embedding table (20M bigrams/trigrams, "128 shards")**
- 4B MTP head (1 layer, trained multi-step)
- 48 layers, hybrid Gated DeltaNet + Qwen Sparse Attention (indexer MQA)

Unsloth keeps the n-gram table high-precision in EVERY stock quant, so:

| stock quant | size | fits 62 GB RAM? |
|---|---|---|
| UD-Q4_XS | 93.7 GB | no |
| UD-Q2_K_XL (local) | 73.44 GB | no |
| UD-IQ1_S | **72.5 GB** | no (barely saves anything!) |
| Baekpica mixed (n-gram Q5_1) | 91.7 GB | no |

Consequence: every stock quant forces NVMe streaming. A custom requant that
drops the n-gram table to Q2_K/Q3_K is the unlock for full residency. The
n-gram table is a SPARSE LOOKUP (tiny rows per token, like the token
embedding) - its traffic cost is near zero; only its residency cost is high.

## 3. Route 1 (flagship): Flash-Next RESIDENT, ~25-45 t/s

### E30a - feasibility probe (zero download, ~1 h)
- Requant ONLY the n-gram PLE tensors in the LOCAL UD-Q2_K_XL (73.44 GB)
  from their current type (~Q4_K) to Q2_K via `llama-quantize --tensor-type`
  regex -> target ~55-58 GB.
- Run resident: `-ngl 99 --cpu-moe` (upstream flags, in our tree:
  tools/server README documents -cmoe/-ncmoe/-ot), mmap+mlock with raised
  RLIMIT_MEMLOCK (root; AGENTS.md CAP_IPC_LOCK note applies).
- Bench pp512/tg64 + LRU-free baseline. Quality caveat: double-quant
  (Q4->Q2) - this is a SPEED-feasibility measurement, not the final artifact.
- Decision gate: >= 20 t/s tg -> greenlight E30b. < 10 t/s -> investigate
  split overhead before giving up (the 470-split ping-pong also lives here).

### E30b - proper build (overnight fetch + requant)
- Fetch BF16 originals from unsloth/Qwen3.8-Flash-Next-GGUF (354 GB, ~5 h at
  the proven 21 MB/s; fetch-hf.py ranged-resume + sha256 discipline applies)
  + `imatrix_unsloth.gguf_file` (580 MB, published in the same repo).
- Custom ladder (llama-quantize per-tensor overrides + imatrix):
  - routed experts: IQ2_XXS w/ imatrix -> ~24 GB
  - n-gram PLE: Q3_K -> ~19 GB
  - attention/dense/shared/lm_head: Q8_0 -> ~5 GB
  - total ~52 GB -> RAM-resident (mlock), fits with KV + OS + headroom
- Serve: `-ngl 99 --cpu-moe` -> iGPU does attention/dense from device RAM,
  CPU reads routed experts from system RAM at bus speed (~32 GB/s effective,
  evidenced by Qwen3-Next-80B at 21.2 t/s). Zero disk traffic after load.
- MTP sidecar: `MTP/mtp-Qwen3.8-Flash-Next-Q4_K_M.gguf` (2.79 GB; shared
  variant 1.91 GB) via the sidecar mechanism (upstream PR 28243 route;
  sidecar machinery already in our tree: commits ff33779b8, 51227a121;
  qwen4exp nextn wired at src/llama-model.cpp:2579). MTP n=1: +30-40%
  effective decode (amortizes expert reads across 2 tokens).
- Quality A/B: small-corpus perplexity vs UD-Q2_K_XL + a coding eval.
  Note: n-gram Q4->Q3 from BF16 is single-quant (not double) - much safer
  than E30a's probe.

Expected: pp 150-300 t/s, tg 25-45 t/s (MTP upside beyond). Numbers grounded
in: 80B-A3B measured 21.2 t/s at higher active-bytes/token; here active bytes
drop ~1.6x and reads move from NVMe (6.9 GB/s) to bus (~32 GB/s effective).

### Risk register (Route 1)
- RAM edge: ~52 GB model + KV + OS ~= 62-63 GB. Mitigations: c modest
  (hybrid DeltaNet KV is tiny - only sparse-attn layers carry KV), n-gram
  at Q2_K if needed (~13 GB), existing OOM-hardening (swapfile, oom_score_adj
  -500, --ctx-checkpoints discipline). OOM-kingpin history demands paranoia.
- mlock limits: needs raised RLIMIT_MEMLOCK (sudo setrlimit / CAP_IPC_LOCK).
- qwen4exp arch is exotic (GatedDeltaNet, indexer, n-gram) - verify
  `--cpu-moe` regexes match its tensor names (ffn_.*_exps pattern may differ;
  inspect with llama-gguf first).

## 4. Route 2 (max quality): Flash-Next STREAMING, 6-13 t/s

The UD-Q2_K_XL (73.44 GB, local) stays the quality ceiling on this box.
VITRIOL wrap is the delivery mechanism (E28, committed). Pending phases:

| Phase | Work | Expected |
|---|---|---|
| E29-0 | instrument: touch/pread/DMA/sync/split-copy breakdown; NPU idle baselines | locate the 301 ms/token |
| E29 | pread fill (fd plumbed through wrap buffer), batched async fills (QD 3-10), fix lru_sync to wait ALL in-flight slots | tg 3.36 -> 6-8 |
| E31 | routing histogram -> Zipf curve -> frequency-based expert pinning (top-16/layer ~= 1.4 GB in 4 GB LRU) | hit 42 -> 60-70% |
| E32 | full-SYCL placement (dense 26 GB resident in 33.5 GB budget, experts streamed, CPU out of loop) | remove 470-split ping-pong |

Upstream context that validates this direction: ggml-org issue #20757
("Two-tier GPU+RAM expert cache") - community PoC found the SAME bottleneck
(synchronous staging copies for mmap'd data) and the SAME remedies
(GGML_OP_OFFLOAD_MIN_BATCH=1, hot-expert residency, +26% tg on Vulkan).

## 5. Route 3: the honest "27B" answer (fast tier)

- Dense Qwen3.8-27B: bus-capped ~5-6 t/s even with MTP (15.3 GB/token vs
  85-90 GB/s). SKIP on this SoC - standing fact, re-verified.
- What flies: fully-iGPU-resident MoE coders:
  - Qwen3-Coder-30B-A3B Q4_K_XL (~18-20 GB) -> 30-45 t/s expected
  - Qwen3-Next/Coder-Next-80B at IQ2_XXS (~20-22 GB) -> 30-40 t/s expected
  Both fit the 33.5 GB device budget entirely; zero disk; also natural MTP
  hosts. Verify exact unsloth quant availability/sizes at download time.

## 6. NPU track (third engine, now engaged)

Verified working today: OpenVINO enumerates the NPU; libze_intel_npu +
plugin + compiler installed.

| Exp | Work | Purpose |
|---|---|---|
| E24' | pip openvino-genai; small-model decode probe (Qwen3-0.6/1.7B class) on NPU | establish t/s + perf/W; de-blocked (was E24) |
| E25' | co-run contention: NPU aux task (STT/embeddings) while Flash-Next decodes on iGPU; use npu_busy_time_us + tg delta | validate NPU owning agent side-work |
| Phase 5 spike | NPU dense-offload via OpenVINO HETERO (needs torch/transformers/optimum-intel venv + qwen4exp->IR conversion, UNVERIFIED) | exploratory; kill fast |

Honest limits (unchanged): NPU is graph-compiler-only (no gguf kernels, no
SPIR-V), shares LPDDR5X (adds compute parallelism, not bandwidth), and
Flash-Next decode is disk-bound until Route 1/2 land.

## 7. Non-GGUF, answered

OpenVINO IR + GenAI is the right runtime for the NPU track (small models).
For Flash-Next itself, GGUF stays superior: imatrix quants, MTP sidecars,
VITRIOL integration, and per-tensor custom mixes (llama-quantize
--tensor-type). The freedom that matters is the custom per-tensor mix -
GGUF fully supports it.

## 8. Execution order

1. E30a probe (today, zero download) -> decision gate
2. E29-0 instrumentation (parallel, cheap)
3. E30b BF16 fetch overnight + custom ladder + MTP sidecar + A/B
4. E29 streaming pipeline fixes (Route 2 keeps improving)
5. E31 pinning, E32 full-SYCL
6. E33 fast-tier downloads (Coder-30B / 80B IQ2_XXS) + bench
7. E24' / E25' NPU track
8. Phase 5 NPU dense-offload spike

Disk headroom: 4 TB NVMe ~145 GB used -> 354 GB BF16 + 52 GB custom fits.

## 9. Sources (fetched 2026-09-06)

- unsloth/Qwen3.8-Flash-Next-GGUF (HF): model card (125B/6B active, 51.2B
  n-gram, 4B MTP), file tree (quant sizes: IQ1_S 72.5 GB, IQ4_XS 93.7 GB,
  BF16, imatrix 580 MB, mmproj, MTP sidecars 1.91-7.77 GB)
- unsloth docs: Qwen3.8 ladder (TQ2_0/TQ1_0/Q1_0 = UD-IQ1_XS/XXS/XXXS at
  1.4375/1.3125/1.1875 bpw; Qwen3.8-2.4T IQ1_XXXS 397 GB PPL table)
- HF blog: "Performant local MoE CPU inference with GPU acceleration"
  (-ot exps=CPU recipe, GGML_OP_OFFLOAD_MIN_BATCH, -b/-ub guidance)
- ggml-org llama.cpp issue #20757: two-tier expert cache PoC (SAM bypass
  +55% pp / +26% tg; SLRU + admission filter +8-15 pp hit rate; the
  synchronous-staging-bottleneck finding matches E28)
- tools/server README: -cmoe/-ncmoe/-ot flags (upstream, in our tree)
- Panther Lake NPU5: 50 INT8 TOPS, native FP8, 3 NCE/12K MACs @ 2050 MHz,
  4.5 MB scratchpad (Intel CES materials, LinuxLinks PTL-NPU CachyOS test:
  6x CPU throughput, 12.7x perf/W on ResNet-INT8 via OpenVINO)
- Local verification: OpenVINO device enumeration, /sys/class/accel/accel0,
  git log (sidecar commits), src/llama-model.cpp:2579 (qwen4exp nextn)

## 10. Open questions

1. E30a greenlight (zero-download probe) - default yes per user.
2. Fast-tier choice (Route 3): Coder-30B vs 80B-IQ2_XXS vs both.
3. E30b: 354 GB BF16 fetch acceptable overnight?
4. E32 scope: full-SYCL placement interacts with the -cpu-moe split of
   Route 1 - decide after E30a whether Route 1 makes E32 redundant for
   decode (likely: full residency beats streaming everywhere except the
   73 GB max-quality artifact).
