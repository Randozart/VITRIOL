# SwarmLLM Mining Assessment — full research record

Date: 2026-09-08
Source repo: https://github.com/Nehanth/swarmllm (MIT license, 77 commits, main)
Docs fetched: README.md, docs/kernels.md, docs/architecture.md, docs/deltanet-prefill-spec.md, docs/bench-log.md
Prior assessment superseded: `.opencode/plans/vram-pooling-options-2026-08-31.md` ("Skip SwarmLLM permanently") — that verdict judged SwarmLLM as a RUNTIME replacement, which is still correct. This doc records the follow-up: SwarmLLM as INSPIRATION to mine.

## 0. Executive summary

SwarmLLM is a from-scratch WebGPU + WebRTC inference engine that runs the
SAME model class VITRIOL serves daily (Qwen3.8-27B, 48 Gated-DeltaNet layers,
embedded MTP draft head). Its runtime (browser tabs, WebRTC rooms, WGSL) is
not usable inside VITRIOL's CUDA stack — and its own bench log shows native
llama.cpp prefill is 14x faster than its WebGPU path, so its "whole engine"
claim is not the point. The point is a set of MEASURED, DOCUMENTED kernel
techniques with bit-identity/golden-test discipline, MIT-licensed, directly
applicable to CUDA.

Single highest-value borrow: the **chunked Gated DeltaNet prefill algorithm**
(E1-E7, running-product decays + C-step triangular solve), fully specified
and numerically verified in `docs/deltanet-prefill-spec.md`. VITRIOL planned
this exact wall (`.opencode/plans/qwen38-c1b-parallel-scan-deltanet-2026-08-18.md`,
affine composition) but NEVER implemented it — the kernel still carries
`//TODO: Add chunked kernel for even faster pre-fill` (gated_delta_net.cu:180).

## 1. License and provenance status

- SwarmLLM is **MIT** — VITRIOL's licensing table (AGENTS.md) allows copying
  with attribution retained.
- Every ported file gets a `PROVENANCE` header:
  `// PROVENANCE: inspiration — Nehanth/swarmllm (MIT), what was learned, not copied`.
- The GDN chunk math is prior art from the Gated Delta Net paper
  (arXiv 2412.06464, Sec 3.3); SwarmLLM verified it vs the HF
  `torch_chunk_gated_delta_rule` reference and a float64 oracle. We cite the
  paper as primary, SwarmLLM as the verified implementation reference.
- README Ars Priori gains a SwarmLLM row (added this session).

## 2. What SwarmLLM is (for the record)

- Engine: ~50 WGSL kernels, own GGUF parser, quantization repacking, in-browser.
- Runtime: WebRTC rooms, each device holds a contiguous slice of layers,
  ~10 KB f16 hidden state per token hops between devices.
- Performance (GB10, Qwen3.8-27B Q4_0, greedy, bit-identical):
  - decode plain 9.0 t/s, decode speculative K=5 16.0-16.65 t/s
  - native llama.cpp on same machine/GGUF: 8.0 decode, 377 prefill
  - their prefill GEMM: 34.6 -> 56.7 t/s (1.64x) end-to-end 43.7
- Their reference for speculative graph: llama.cpp's own MTP graph.

## 3. Already present in the VITRIOL fork (NOT borrowable — done)

| Capability | Fork location |
|---|---|
| SSM scan snapshot rollback slots | `ggml-cuda/ssm-scan.cu:221-227` |
| GDN snapshot slot mapping (MTP rollback) | `ggml-cuda/gated_delta_net.cu:146,285` |
| GPU argmax | `GGML_OP_ARGMAX` in `ggml-cuda.cu` |
| FFN gate/up fusion + SiLU | llama-graph fusion |
| MTP speculation, embedded head | daily driver |
| f16 block scales end to end | GGUF native |
| One command submit per token | CUDA graphs |
| DeltaNet pre-pass (gates + L2 fused) | ssm-conv / gated_delta_net path |

## 4. Full borrow list, ranked by value

### 4.1 CHUNKED GATED DELTA NET PREFILL (the prize)

**Problem being solved.** `gated_delta_net.cu` runs the recurrence serially:
`for (int t = 0; t < n_tokens; t++)` (line 66). MTP verify batches ~6 tokens
through 48 recurrent layers: verify(5) measured 308 ms vs single-forward 91 ms
= **3.4x serial wall** (C1b plan, 2026-08-18). Both llama.cpp and SwarmLLM
hit this; SwarmLLM solved it two ways.

**Why chunkable.** Per token, delta depends on S only through the scalar
`kv = S·k`. State map is affine: `S' = g·(I - beta·k·k^T)·S + beta·k·v^T`.
Chunk composition folds the serial dependency into C independent reductions
plus a C-step triangular solve.

**SwarmLLM's E1-E7 formulation** (verified f64 oracle O err 4e-15, S err 8e-17;
vs shipped kernel ~5e-8 f32 reorder):
```
gamma_r  = prod_{m=0..r} alpha_m            (running product of decays, gamma_-1 = 1)
Lam[r][i] = prod_{m=i+1..r} alpha_m          (i <= r; Lam[r][r] = 1; 0 for i > r)
(E1) A[r][i] = beta_r * Lam[r][i] * (k_r . k_i),  i < r   (strictly lower; I PLUS A)
(E2) R[r] = beta_r v_r - beta_r gamma_r (S_0^T k_r)
(E3) (I + A) D = R   unit-lower forward substitution (serial, C steps)
(E4) P[r][i] = Lam[r][i] * (q_r . k_i),  i <= r
(E5) o_r = scale * ( gamma_r (S_0^T q_r) + sum_{i<=r} P[r][i] D[i] )
(E6) S_C = gamma_{C-1} S_0 + sum_r lamC[r] k_r D[r]^T
(E7) snapshot S_r = gamma_r S_0 + sum_{i<=r} Lam[r][i] k_i D[i]^T
scale = 1/sqrt(dState) from a UNIFORM read, never the folded literal
   (folding 128 changes 1 ULP in ~90% of outputs)
```
Numerical conventions that matter (all measured by SwarmLLM):
- decays as running products of per-token alpha, NEVER `exp(G_r - G_i)` of a
  cumsum (f32 cumsum drifts 1e-6..1e-5; WGSL exp allowed 3+2|x| ULP error)
- no inf*0 hazard, no select-masking (only i <= r products formed)
- f32 throughout; f16 S rejected (4.9e-4 per rounding, state-continuity break)
- k,r L2-normalized => |A[r][i]| < 1, |T| <= beta_r < 1; fp32 forward
  substitution at C=16 within ~4e-7 of fp64 over 3000 adversarial trials
- no pivoting; never form T explicitly

**Measured kernel speedups** (GB10, 48 layers, per pass):
| nCols | today | RG=1 | RG=2 | RG=4 |
|---|---|---|---|---|
| 1 | 5.6 ms | 4.5 (1.24x) | 3.2 (1.72x) | 3.3 |
| 4 | 9.2 | 6.0 (1.54x) | 4.3 (2.13x) | 4.7 |
| 8 | 14.0 | 7.8 (1.78x) | 5.8 (2.41x) | 6.4 |
| 16 | 23.7 | 11.6 (2.03x) | 8.7 (2.72x) | 9.8 |
| 32 | 43.0 | 19.1 (2.25x) | 14.6 (2.94x) | 16.5 |

Fit: today ~4.4 ms fixed + ~1.2 ms/col; RG=2 ~2.9 ms fixed + ~0.37 ms/col.
Remaining fixed ~2.9 ms = 48 dispatches (~1.3 ms) + one S load/store per pass
with only 48 WGs in flight.

**Honest ceiling.** The recurrence is NOT the prefill bottleneck by itself:
matvec_*_b is ~77% of an 8-col pass; dn_delta_mc is ~4% (9-14 ms of a
132-222 ms pass). Chunk kernel end-to-end gain is capped ~1.04-1.07x on
prefill tok/s. BUT: it is required groundwork for 16+ column passes and a
64-token chunk path, AND VITRIOL's real target is the MTP VERIFY pass serial
wall (3.4x), which this directly attacks.

**Implementation order recommended by SwarmLLM (adopt):**
1. Register-resident sequential rewrite first (RG=2: private array with
   LITERAL indices, shared-mem partial reduce) — bit-close ~5e-8, ~100 lines,
   1.7-2.9x on the kernel. Mandatory prerequisite: a WGSL `for` over a const
   bound does NOT unroll (naga); all unrolled code uses literal indices.
   Same principle applies to CUDA `#pragma unroll`.
2. Chunk kernel (E1-E7) only if/when widening passes to 16+ cols, or as
   groundwork for 64-token chunks.

### 4.2 REGISTER-RESIDENT dn_delta tiling (RG=2)

Details in 4.1 table. One workgroup per value head; thread `(r,j)` owns rows
`[r*R, (r+1)*R)` of S column j; q/k staged into 2 x 128 f32 workgroup arrays
(two barriers); RG>1 partial vh/sq through `RG x 128 x 2` f32 workgroup mem,
one barrier, fixed-order sum. S loaded once before column loop, stored once
after; snapshot slots written straight from registers per column.

### 4.3 ROW-STATIONARY PREFILL GEMM

SwarmLLM: 16-col tiled Q4 GEMM, packed-nibble weight tile in shared memory
(4 KB, never dequantized there), broadcast activations from a transposed
staging copy, prefetch next block pair into registers, split-K pinned per
shape (every peer computes identical hidden states), partials reduce in fixed
order. Pass-level 34.6 -> 56.7 t/s (1.64x). Bank-conflict padding (row stride
17 vec4s vs 16) was the only lever that moved it (1.25x); 4x4 thread tiles
slower than 2x4 (occupancy beat reuse). Fires only at full batch width.

For VITRIOL: llama.cpp MMQ/MMF already tile the prefill path; this applies
only if the MTP verify path (small-column GEMM) shows a gap. Low priority.

### 4.4 LOAD-TIME SCALE/NIBBLE REPACKING

GGUF interleaves `[scale][nibbles]`; SwarmLLM splits into separate nibble
array + scale array at load so cooperative-row stripes are contiguous.
Layout, not compression. Applies to VITRIOL's RAM Shot streaming buffer
(`vitriol-buffer.cpp`) — reduces bytes streamed over PCIe. Medium value for
streaming mode only.

### 4.5 DEVICE AUTOTUNE (WG/ROWS, 3% noise guard)

Times `(WG, ROWS)` candidates on the actual GPU at load (~1 s), keeps winner
with 3% noise guard. VITRIOL's `vitriol calibrate` does VRAM math, not kernel
config. Mismatched dual-GPU (3060 + 1070 Ti) is where per-device kernel
autotune pays. Low effort.

### 4.6 ADAPTIVE DRAFT DEPTH (probe 3/5/7 by lap time)

SwarmLLM locks room at depth by measured t/s. VITRIOL hardcodes MTP n_max=1
(correct on this box: n>=2 regresses, acceptance decay). The MECHANISM
(measure-and-keep-fastest) could generalize to ubatch/split tuning. Low.

### 4.7 2-D DISPATCH FOR TALL MATVECS

WebGPU silently drops dispatch over 65,535 workgroups in one dimension;
LM head at 2 rows/WG needs 124,160; kernels derive `row0 = wg.y*32768 + wg.x`.
CUDA grid.y caps differ (2^16 on grid dims for old archs) — worth a guard on
sm_61 for the LM head. Low.

### 4.8 ACCUMULATE-INTO-RESIDUAL MATVECS (`_acc`)

Fold residual add into o-proj/ssm-out/ffn-down: -128 to -192 dispatches/token.
Measured NEUTRAL on Vulkan (small dispatches ~free there). CUDA-graph path
already amortizes dispatch. SKIP unless a profile shows dispatch-bound.

## 5. SwarmLLM measured rejections (validate VITRIOL verdicts — file as don't-repeat)

- **Q4 KV cache: -92.5% prefill** — caution flag: VITRIOL ships q4_0/tq3_0 KV.
  Their quant is not tq3_0 (VITRIOL depth-certified tq3_0 2026-08-24), but the
  prefill-interaction warning is worth keeping in mind for KV-quant + prefill
  experiments.
- External 0.6B draft model: vocab mismatch (151,936 vs 248,320) can't verify
  => confirms VITRIOL's embedded-MTP choice (external draft is dead end for
  this arch).
- Tree/Medusa drafting: no trained heads + DeltaNet state per branch.
- Subgroup reductions: 0-8% (shared-memory tree not the bottleneck).
- RMSNorm fused into GEMV: 0.91x on Metal.
- 4x4-per-thread GEMM tiles: slower than 2x4 (occupancy beat reuse).

## 6. SwarmLLM methodology worth adopting

- **Golden tests gate every optimization**: speculative path must produce the
  same stream as plain decoding (bit-exact by construction). VITRIOL's oracle
  ladder (L0 byte-exact / L1 cosine / L2 greedy-equal) is the equivalent.
- **Kernel isolation timing** (`bench_breakdown.js`): re-times passes with one
  kernel family skipped at a time to attribute cost honestly. VITRIOL should
  do this before/after the GDN chunk kernel to avoid over-claiming.
- **Skip-timing under-attributes latency-hidden kernels** — keep a direct
  microbench alongside in-engine timing.
- **Numerical oracle first, kernel second**: float64 chunk-vs-sequential check
  (their verify.py, 4e-15) before writing the kernel. Port this pattern: a
  CPU f64 reference for E1-E7 vs serial, then the CUDA kernel vs both.

## 7. Implementation phases (proposed, not yet scheduled)

### Phase 1 — Record (this doc + provenance) [DONE 2026-09-08]
- [x] This assessment written
- [x] README Ars Priori + SwarmLLM row
- [x] vram-pooling-options verdict note appended

### Phase 2 — Chunked GDN prefill (the value; ~1-2 sessions)
1. Re-read `gated_delta_net.cu` fully; confirm exact recurrence/layout/KDA path.
2. f64 oracle test: E1-E7 vs serial recurrence on synthetic inputs (4e-15 gate).
3. Register-resident sequential kernel (RG=2 pattern) behind `GGML_CUDA_GDN_SCAN=auto|on|off`.
4. Extend `test-backend-ops.cpp` `test_gated_delta_net`: n_seq_tokens=6 (MTP
   verify shape), head_size=128, v_repeat 48/16, KDA + non-KDA, serial vs scan
   A/B, tolerance ~1e-5.
5. Full-model: MTP verify(5) 308ms -> target ~110-150ms; end-to-end 14 ->
   ~18-24 t/s. Depth-certify with fingerprint + filled-token depth (AGENTS.md
   1/4/5).
6. Commit each green stage (AGENTS.md workflow).

### Phase 2 — DONE 2026-09-08 (implemented + certified)
1. Implemented the **chunked GDN kernel** (E1-E7) directly (skipped the RG=2
   register-resident rewrite — the fork's serial kernel is ALREADY
   register-resident: s_shard/k_reg/q_reg in registers). Non-KDA path only
   (chunk identity assumes scalar decay; KDA stays serial).
2. Numeric validation: E1-E7 vs serial CPU reference at the MTP-verify shape
   (S_v=128, n_tokens=6, K=4 snapshots) — **passes on BOTH CUDA devices**
   (3060 + 1070 Ti). Full identity: delta_0/delta_1 expand identically to the
   serial recurrence (verified by hand in the kernel comments).
3. Dispatch: `GGML_CUDA_GDN_SCAN=off|on|auto` (default auto), non-KDA +
   n_tokens in {2,3,4,6,8} -> chunk; else serial. KDA untouched.
4. Tests added: n_tokens=6 (K=4) incl. KDA-serial control, n_tokens=2/8 with
   n_seqs=2. All pass.
5. **Full-model A/B (Qwen3.8-27B, blessed config, draft_n_max=1)**:
   - Correctness: chunk vs serial output **bit-identical** on real 120-token
     generation (validates the (I+A)D=R identity end-to-end).
   - Perf: 17.43 t/s (serial) vs 17.54 t/s (chunk) = +0.6%, parity within
     noise. Honest ceiling confirmed: with draft_n_max=1 the verify batch is
     C=2 tokens, serial wall shallow; the chunk kernel pays off at C=6+
     (larger draft chains), which this box rejects (n_max>=2 regresses).
6. Committed: llama.cpp 8382994a0, outer eb4c965.

### Phase 3 — Quick wins (assessed 2026-09-08; NOT implemented — see rationale)
- autotune probe (4.5), 2-D dispatch guard (4.7), load-time repack (4.4).

**Phase 3 assessment — all three weaker than the "quick wins" label; no implementation:**

- **4.7 2-D dispatch guard — NOT a real bug on CUDA.** The WebGPU 65535/dimension
  limit maps to grid.y/z on CUDA (capped 2^16 on old archs), but the "tall matvec"
  dim is the VOCAB (248,320), which lands in **grid.x** (`block_nums(nblocks, ...)`
  at mmvq.cu:996, `nblocks=(nrows_x+rpb-1)/rpb`, `nrows_x`=vocab). grid.x caps at
  2^31-1, so 248,320 blocks (or 124,160 at rpb=2) is fine. The small grid.y/z dims
  (`nchannels_dst`, `nsamples`) stay at 1-8. No launch exceeds a grid dim limit.
  Nothing to guard. (Confirmed by tracing `calc_launch_params` -> `block_nums` ->
  `mul_mat_vec_q` row0/channel_dst mapping.)

- **4.5 device autotune — premise weaker than doc assumed.** Pascal sm_61 (1070 Ti)
  AND Ampere sm_86 (3060) BOTH resolve to `MMVQ_PARAMETERS_GENERIC` (mmvq.cu:113-127
  `get_device_table_id` `#else` fallback; the table IS already per-`__CUDA_ARCH__`
  since the build targets both archs). So there is no mismatched-table situation to
  fix — both GPUs share the same already-tuned generic values (nwarps=4 for
  ncols_dst 1-4). Autotune would only marginally refine generic defaults; the gain
  is speculative, the path (calc_nwarps/calc_rows_per_block) is performance-critical
  and shared across every quant type, and the review/maintenance burden is high.
  Not worth it absent a measured regression.

- **4.4 load-time scale/nibble repack — no effect on the daily driver.** The daily
  driver runs `VITRIOL_MODE=off` (weights fully VRAM-resident, AGENTS.md residency
  rule). The repack only reduces bytes streamed over PCIe in `mode=stream`
  (vitriol-buffer.cpp). Zero payoff unless/when streaming is used for a model that
  exceeds VRAM (35B-class). Park until a streaming workload exists.

**Verdict:** Phase 2 (the chunked GDN kernel) was the real value and is done.
Phase 3 items were "low" in the doc's own ranking and, on inspection, are either
non-bugs (4.7), speculative-on-a-critical-path (4.5), or inapplicable-to-current-
mode (4.4). Recommend closing the SwarmLLM mining arc here; re-open 4.4 if/when a
35B-class streaming workload lands, re-open 4.5 if a profile shows a dispatch-bound
or warp-count-mismatch regression.

## 8. Open decisions

1. C1b (2026-08-18) used affine chunk-composition; SwarmLLM E1-E7 is a
   different, published, verified formulation. Recommend superseding C1b's
   formulation, keeping its dispatch/toggle/test scaffolding. Confirm.
2. Phase 2 split: "register-resident first (M session), chunk kernel second
   (L session)" — each its own commit + A/B. Confirm.
3. Whether to run the full 27B A/B now (server is down; 3060+1070 Ti available)
   or defer until the kernel unit tests are green.

## 9. Traceability

- SwarmLLM docs fetched 2026-09-08 (raw.githubusercontent.com/Nehanth/swarmllm/main/docs/*).
- Files: kernels.md (tricks 1-28 + rejections), architecture.md, deltanet-prefill-spec.md
  (full WGSL + E1-E7 + numeric validation), bench-log.md (all measured rows).
- Key line references in this fork: gated_delta_net.cu:57 (serial loop), :66
  (token loop), :146 (snapshot slots), :180 (TODO chunked), :285 (K slot param);
  ssm-scan.cu:221-227 (rollback snapshots); llama-context.cpp:192 (fused_gdn_ch).