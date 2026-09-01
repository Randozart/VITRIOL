# VITRIOL Mining & Experiment Master Plan — 2026-09-01

Status: ACTIVE
Author: session 2026-09-01 (opencode)
Supersedes: nothing; complements `lull-certification-report-2026-08-24.md`,
`branch-consolidation-2026-09-01.md`.

---

## 0. Context and motivation

VITRIOL development paused while upstream `ggml-org/llama.cpp` and sibling
repos in `~/Desktop/Projects/` advanced. A mining pass (this session) surveyed:

1. **Upstream ggml-org/llama.cpp** — 54 commits landed between the fork point
   (inner `main` at `9723942ad`, 2026-08-30 11:18 -0700) and 2026-09-01 11:55 UTC.
2. **OurobourOS** (`../OurobourOS`) — heterogeneous-cluster OS; forensics in
   `PLAN.md` §16.3c/d already explained and closed VITRIOL's 1.6× decode gap
   (ggml base age, not VITRIOL patches; fixed by the `vitriol-ku` rebase,
   +84% tg). Remaining ore is technique, not speed.
3. **bitshaper-ai** (`../bitshaper-ai/lab/`) — CUDA LUT GEMV kernels for
   TQ1_0/k-quants, tuned on this host's GTX 1070 Ti (sm_61).
4. **kimi-k3-in-c** — CPU MoE streaming engine; expert-cache science
   (histogram-pinned hot set, trace replay, ring prefetch).
5. Sibling sweep (imp, moore-kernel, kennel-kernel, SGNT, glue-ffi,
   hermes-agent, trismegistus, wetware-vr-interface, 2d-Kiosk-Avatar) —
   nothing minable; see session log for per-repo verdicts.

This document defines the experiment program that converts that ore into
measured, documented results. **We work scientifically**: each experiment has
a falsifiable hypothesis, a fixed control, a measurement protocol, and a
documentation target. Null results are results.

---

## 1. Non-negotiable protocol (applies to every experiment below)

These rules come from `AGENTS.md` and prior certification discipline; they are
restated here because every experiment must obey them:

- **P-1 Residency rule**: `VITRIOL_MODE=off` (weights fully VRAM-resident) is
  the default for resident-capable quants (≤ ~20 GiB combined). Stream/DMA
  offloading only for 35B-class models that exceed combined VRAM. Any
  streaming experiment must justify itself.
- **P-2 Stale-server rule**: `killall -9 llama-server` before every launch.
- **P-3 Flag provenance**: every launch emits `VITRIOL-FINGERPRINT:` (launcher,
  server main, runners). Every RESULT embeds full argv. Reports must trace to
  a fingerprint. Silent flag drift = review blocker.
- **P-4 Window ≠ depth**: context claims state FILLED token counts, never
  window size. Depth claims require depth-filled prefill + decode at depth.
- **P-5 Dual-arch builds**: `cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86"`.
  Native-only builds miss sm_86 → "no kernel image" on the 3060.
- **P-6 CAP_IPC_LOCK optional**: CUDA pinned allocs don't count against
  RLIMIT_MEMLOCK on this host. Never pass `--ctx-checkpoints 0` (heap
  corruption) or `--cache-ram 0` (no readiness).
- **P-7 Fixed controls**: every A/B pins model file (sha256 recorded), build
  commit, nvcc version, arch list, and full argv. One variable changes at a
  time. Baseline numbers are recorded in §3 and may not be edited after runs
  begin; corrections are appended, never overwritten.
- **P-8 Documentation**: findings land in `.opencode/plans/*.md` and
  `EXPERIMENT_LOG.md` with ISO 8601 timestamps (YYYY-MM-DD HH:MM), exact
  command output, tensor shapes, error strings. Session log updated with
  anchored summary (progress / blockers / decisions).
- **P-9 Licensing**: incoming code checked against the compatibility table
  (`AGENTS.md` §Licensing). `PROVENANCE` headers on algorithm-bearing modules.
  Per-source verdicts: upstream = MIT (fine, same tree); bitshaper-ai = own
  work, T-MAC-inspired re-derivation (record inspiration header);
  kimi-k3-in-c = Apache-2.0 (fine, attribute); OurobourOS = own work;
  Microsoft BitNet ladder kernel (referenced by OurobourOS `bitnet-cpp`) =
  MIT (fine, attribute).
- **P-10 User server courtesy**: the daily-driver `vitriol-server.service`
  unit (Lapis Occultus, port 8279) serves the user's Officina agent. Bench
  windows require stopping the unit (`systemctl --user stop vitriol-server`)
  and **restarting it afterwards**. VRAM must be free before llama-bench.

---

## 2. Sources and what was mined (evidence inventory)

### 2.1 Upstream commits since fork point (targets)

Fork point: `9723942ad` (hexagon CPY fence, 2026-08-30). Window surveyed to
2026-09-01 11:55 UTC, 54 commits. Promoted to A-list (port priorities):

| # | upstream PR | subject | VITRIOL relevance |
|---|---|---|---|
| 1 | #28011 | kv-cells: stop sequence scan once all sequences seen | deep-ctx decode; upstream: 56→74 t/s @55k ctx |
| 2 | #27991 | kv-cache: optimize restoring non-contiguous cells | 1.4M copies/25–63 s → 224 copies/<0.5 s; checkpoint/prompt-cache restore |
| 3 | #27621 | CUDA: extend MOE fusion to specdec, glu + topk-router fusion multi-token | Qwen MoE decode path |
| 4 | #27978 | CUDA: fast mm_ids_helper path for any n_expert_used | Qwen n_expert_used=10 hits it; PP gain |
| 5 | #28123 | qwen4exp: recurrent state rollback | MTP draft rounds stop serializing full state; pattern portable to qwen35_mtp |
| 6 | #28159 | model: load hparams.n_layer_nextn before n_layer() calls | qwen35_mtp arch load correctness (embedded MTP head) |
| 7 | #25635 | CUDA: XOR swizzle flash attn K,V smem fp16 tiles | FA decode; **must verify sm_61 applicability** (swizzle/ldmatrix paths often gate sm_70+) |
| 8 | #27837 + #27969 | TENSOR_READ_LAZY → `--lazy-mode -lzm` | lazy weight reads; feeds residency experiments (E6) |

Also merged opportunistically (B-list): qwen4exp fix pile #27941, RPC
buffer-serialization fix #26500, SWIGLU_CLAMP #27930, MUL_MAT alloc
accounting #28071, graph_optimize alloc deps #27301, Hadamard k_rot guard
#27967, cpp-httplib 0.54.0, AVX2 IQ batched-gemm #27402 (CPU offload path),
quantize row-slab stream #27830. Metal/SYCL/Vulkan/ROCm-only commits ride
along in the merge but are out of scope for measurement.

### 2.2 OurobourOS findings

- `PLAN.md:1239-1325` (§16.3c/d): vintage bisection — toolchain +17% tg;
  ggml base age explains the 1.6× gap; VITRIOL patches exonerated by a
  Vulkan control build; `build-ku` (rebase onto `9723942ad`) = 1509 pp /
  42.08 tg (9B Q6_K CUDA), 1734 / 36.36 (Q8_0). **This rebase is already the
  inner `main` line — harvested.**
- `bitnet-rs/src/lib.rs:63-199`: `cb_eval` oracle capture — dump every f32
  graph node (name + data) from one reference decode via llama.cpp's eval
  callback. Portable as-is. → E3.
- `bitnet-cpp/gpu/bitnet_kernels/bitnet_kernels.h:24-83`: lop3 2-bit→int8
  decode + `__dp4a` GEMV template (MIT, Microsoft BitNet). → E7 template.
- `ouro-wgpu/src/lib.rs`: alignment-padded weight repack (212-byte Q6_K rows,
  u32-aligned) worth 3× matvec; measured 9.76→3.3 ms then 2.61 ms @ 11 GB/s.
  Lesson: alignment-padding as load-time repack step for CUDA kernel inputs.
- `docs/CONTRACTS.md`: parity ladder L0 (bit-exact) → L5 (energy). Adopted as
  VITRIOL verification shape (E3).
- `docs/QWEN35_PORT.md` + `cluster/src/infer/qwen35.rs`: independent,
  differentially-verified description of GatedDeltaNet / conv / partial-MRoPE
  / interleaved q|gate — reference material when VITRIOL touches qwen35-arch
  paths (Qwen3.8-27B is qwen35 family).
- Vulkan-parity on Pascal: inherited upstream claims (#10879 scoreboard,
  #19817 1060: Vulkan 90.6 vs CUDA 61.7 tg) — unverified on our 1070 Ti. → E5.
- Not mined: BMTS (GGUF+mmap already equivalent), Beast, LUT CPU kernels
  (x86-only), RAPL probes as written (no delta sampling; delta-integration
  pattern is 5 lines if wanted later), sched_ext (design only),
  PlacementPlan (design only; MoE touch-rate replication idea recorded for
  future expert-placement work).

### 2.3 bitshaper-ai findings (mined for E2)

- `lab/include/kernels.h` — launcher API: `run_tq1_0_gemv_reference` (scalar
  ggml-exact), `run_tq1_0_gemv_lut` (LUT-build kernel + pure-lookup GEMM
  kernel), `run_tq1_0_gemv_lut_compacted` (per-column live-block list skips
  d==0 blocks; bit-exact because `fma(0,s,acc)==acc`).
- `lab/src/lut_matmul.cu` — `tq1_0_lut_build_kernel` (L74),
  `tq1_0_gemv_lut_kernel` (L96), `tq1_0_gemv_lut_compacted_kernel` (L162).
- Sibling GEMVs: iq2_s / iq2_xxs / iq3_xxs / iq4_xs / q4_k / q5_k / q6_k /
  q8_0, same pattern.
- Measured on 1070 Ti (`EXPERIMENT_LOG.md` there): TQ1_0×q8_K LUT GEMV,
  K=6912, C=2560 — R=1: 0.2578 ms/launch (3,879 t/s); R=16 knee: 0.0715
  ms/token (13,981 t/s) = **3.6× per-token amortization**; flat R≥32.
  Mechanism: 2 MB L2 serves repeated weight reads across R row-blocks.
- Negative result kept: weights-as-code (gen_fold.cu) refuted — 92.8× byte
  expansion, ~28 s/row nvcc compile. Dead end, do not revisit.
- Pitfalls: `BUGS.md` A-4 (Q5_K embedding dequant d1/d2 interleave), A-5
  (KV realloc-per-token).

### 2.4 kimi-k3-in-c findings (mined for E4)

- `src/cache/k3_cache.h` (117 lines): expert LRU cache — whole-expert slots
  holding **packed MXFP4** (never floats; matvec is memory-bound so packed is
  faster), per-(layer,expert) request histogram (82,432 counters) pins the
  hot set from measured data, 12 KB/token access trace enabling offline
  policy replay (`tools/sim_cache.py`), honest accounting rule (prefetched
  experts counted separately or hit rate lies).
- `src/core/k3_ops.c:981-1058` `k3_matmul_mxfp4`: consume packed nibbles
  directly; byte→two-E2M1 pair table (256×2 floats); E8M0 scale table;
  4-lane double reduction, fixed order for bit-parity.
- `src/io/k3_trunk.c:276-292,470`: ring double-buffer — pread layer L+1 over
  layer L's bytes while L computes. CUDA analog: `cudaMemcpyAsync` expert
  prefetch on a side stream.
- Measured: 12-budget cgroup ladder; expert hit 0% ≤64 GB → 29.9% @96 GB →
  43.8% @224 GB; **128 GB allocated well beats 224 GB allocated badly**.
  Transferable law: **hit rate, not capacity, is the decision variable.**

---

## 3. Controls and baseline table (P-7; frozen 2026-09-01)

| control | value |
|---|---|
| baseline build | inner `llama.cpp` `main` @ `590a4bb09` ("docs: record license layering") = vitriol-ku port of upstream `9723942ad` |
| baseline 9B Q6_K CUDA pp512/tg128 | 1509 / 42.08 t/s (build-ku, OurobourOS §16.3d) — **APPEND 2026-09-01: build-ku was 86-only @ nvcc 13.3. E1 control = `build/` @ 590a4bb09 (61;86 @ ~/toolkits/cuda-12.9, the running daily-driver build). Candidate `build-ku2` @ 04a4f5f12 mirrors toolchain+arch; single variable = merge.** |
| baseline 9B Q8_0 CUDA pp512/tg128 | 1734 / 36.36 t/s (build-ku, §16.3d) |
| daily-driver model | `~/Downloads/Qwen3.8-27B-Q3_K_M.gguf` (sha256 recorded at bench time) |
| baseline Qwen3.8 depth cert | Q3_K_M + tq3_0 KV, ts 26,10 ub64: 54,692 tok @ 9.21 t/s (2026-08-24; NOTE current live split is 22,14 — current-config cert is DEV-pending, re-baseline it in E1-P5) |
| bench harness | `llama-bench -m <gguf> -ngl 99 -p 512 -n 128 -fa {0,1} -r 2`, device pinned via `CUDA_VISIBLE_DEVICES` |
| depth harness | chunked/single-shot prefill + 3×64 decode at depth per `lull-phase0-report` Addenda 5–6 |
| builds | new-tree `llama.cpp/build-ku2/` (E1 candidate) vs existing `build-ku/` (control); both `61;86` |
| environment | driver 580.178.04; GPUs: dev0 RTX 3060 12 GB (sm_86), dev1 GTX 1070 Ti 8 GB (sm_61) |

Rule: baseline rows above are frozen. New runs append, never overwrite.

---

## 4. Experiment matrix

Order of execution (decision 2026-09-01): **E1 first** (biggest proven
payoff, upstream pipeline already proven by vitriol-ku), **E3 second** (cheap,
multiplies kernel-work verification), then E2, E6 folded into E1, E4-offline
when streaming work resumes, E5 truth-finding, E7 only after E3 exists.

### E1 — Upstream sync port (includes E6)

**Hypotheses**

- **H1a** (seq-scan early stop #28011): deep-ctx decode t/s improves and the
  gain grows with filled depth. Upstream evidence: 56→74 t/s @55k filled.
  Pass: ≥5% tg gain at ≥40k filled tokens on the daily-driver model; gain at
  54k ≥ gain at 43k.
- **H1b** (KV restore batching #27991): checkpoint/prompt-cache restore
  wall-time drops by ≥10× on a restore involving ≥1000 non-contiguous cells.
  Pass: measured restore time ratio ≥10× at matched state size.
- **H1c** (MoE fusion #27621 + mm_ids_helper #27978): pp512 and/or tg128 gain
  on Qwen MoE models. Pass: ≥3% on any of pp512/tg128 at matched flags.
- **H1d** (FA XOR-swizzle #25635): FA decode gain **or** a documented sm_61
  gate. Either outcome is a result; the gate must be cited from code, not
  assumed.
- **H1e** (MTP fixes #28123/#28159): correctness, not speed — qwen35_mtp arch
  loads cleanly and speculative slot machinery survives state round-trip.
  MTP remains **off** in daily profiles (zero measured benefit here); the fix
  protects future experiments.
- **H1f/E6** (`--lazy-mode`): load-time and shallow-bench parity (no
  regression); lazy mode is a tool for future residency work, adopted if
  neutral.
- **H0 (meta)**: the merge as a whole is non-regressing. Pass: no control
  metric regresses >2% beyond run-to-run noise (±2% established by -r 2 + a
  third repeat on ties).

**Method**

1. `git -C llama.cpp fetch upstream`; confirm tip ≥ `9d817213a0` (#28159).
2. Merge `upstream/master` into inner `main` (bulk merge, not 54
   cherry-picks). Conflict policy: VITRIOL-owned files (vitriol-cuda-*.cpp,
   TQ3/TurboQuant types, kv quant overrides, server-context checkpoints,
   qwen35_mtp arch wiring) keep VITRIOL semantics; upstream files take
   upstream. The 8 A-list commits get individual review during conflict
   resolution (`git log upstream/master --oneline` cross-checked against
   touched files).
3. Build `build-ku2/` with `"61;86"` (P-5). Record nvcc version. Watch for
   the CUDA 13 `compute_61` removal trap (OurobourOS §16.3c): if the host
   nvcc is 13.x, sm_61 needs `-code` shims or CUDA 12.x sidecar — record
   whichever path is taken in the fingerprint.
4. Static check of H1d: read the #25635 kernels for arch guards
   (`__CUDA_ARCH__ >= 700` style) before benching; cite lines.
5. Bench matrix (P-2, P-3, P-10: stop unit first, restore after):
   - 9B Q6_K + 9B Q8_0, `CUDA_VISIBLE_DEVICES=0`, pp512/tg128, fa 0/1,
     build-ku2 vs build-ku (frozen §3 baselines).
   - Repeat each point; ties get a third run.
6. Depth cert (H1a): Qwen3.8-27B Q3_K_M, current live config semantics
   (ts 22,14, tq3_0 KV, ub64, fa on, MTP off), chunked prefill to ~43k and
   ~54k filled, 3×64 decode at depth. Also re-baseline the **current**
   22,14 split at shallow depth (the 26,10-era cert doesn't transfer).
7. H1b: craft a ≥1000-cell non-contiguous restore (slot save → invalidate →
   restore), time it on build-ku vs build-ku2.
8. E6: one load-time + shallow-bench A/B with `--lazy-mode` on Qwen3.8.

**Document**: `.opencode/plans/e1-upstream-sync-2026-09-01.md` — merge map
(commit → files → conflicts → resolution), per-hypothesis pass/fail table,
fingerprint excerpts, full argv, raw bench output. EXPERIMENT_LOG entry.
On pass: this becomes the new daily-driver build; profiles regenerated only
if flags changed (they shouldn't).

**Cost/risk**: medium cost; low risk (proven pipeline). GPU-busy windows
require stopping the user's server unit (P-10).

### E3 — cb_eval oracle capture + parity ladder adoption

**Hypothesis**: H3 — a one-decode full-graph capture tool (llama.cpp
`cb_eval` callback; pattern proven in `bitnet-rs/src/lib.rs:63-199`) reduces
kernel-change verification from "build + bench + eyeball output" to a
scripted per-node tensor diff, and catches divergence that end-to-end text
comparison misses.

**Method**

1. Small tool (or server flag if the callback is already plumbed) that
   registers `cb_eval`, claims every node, dumps `name → tensor bytes (f32)`
   for one decode step to disk.
2. Capture reference decode on CPU backend and on CUDA (both builds) for a
   fixed prompt + greedy sampling.
3. Diff harness: per-node cos ≥ 0.999 (L1), max-abs-ε report, greedy token
   equality (L2). Output: HTML-less flat report, one line per node.
4. Adopt the ladder shape (L0 bit-exact → L5 energy) in
   `docs/provenance/` or a new `docs/parity-ladder.md` as VITRIOL's
   verification contract for all future kernel/port work.

**Pass**: capture+diff completes on both builds; ≥1 real divergence
demonstrated or the tool proven able to detect an injected perturbation
(sanity gate: flip one bit in a kernel-adjacent buffer, tool must flag the
node).

**Document**: `.opencode/plans/e3-cb-eval-oracle-2026-09-01.md` + ladder doc.

**Cost**: low. **Risk**: zero (no runtime behavior change; tool-side only).

### E2 — bitshaper LUT GEMV on sm_61 (gated)

**Hypothesis**: H2 — LUT-build-once + dead-block-compacted GEMV beats the
current vec_dot path for VITRIOL quant types on sm_61 **at the server's
actual batch size**. Amortization was measured at R≥16 (3.6×); the daily
driver now runs `--parallel 1` (R=1) and speculative ubatch decoding keeps
effective R small. Honest question: does it win at R=1–2? If not, the result
is "park until parallel/batched workloads return" — still documented.

**Method**

1. Port `lut_matmul.cu` kernels (TQ1_0 first — matches VITRIOL's TQ family —
   then q6_k as the k-quant exemplar) behind `VITRIOL_LUT_GEMV=1` env gate;
   zero default-path risk.
2. Parity gate FIRST (needs E3): bit-exact vs existing vec_dot on random
   activations and on real captured tensors; cite ladder rung.
3. Bench: kernel-level microbench at R∈{1,2,16} with Qwen3.8 layer shapes
   (K=6912-class, C=2560-class), then end-to-end tg128 if microbench wins.
4. `PROVENANCE` header: inspiration — bitshaper-ai lab (own work) and
   T-MAC arXiv:2407.00088 (learned, re-derived; not copied).

**Pass**: microbench win at R=1 or R=2 ≥10% with bit-exact parity AND
end-to-end tg gain ≥3%. Ship gated default-off either way; record numbers.

**Document**: `.opencode/plans/e2-lut-gemv-2026-09-01.md`.

**Cost**: medium-high. **Risk**: zero (gated, default off).

### E4 — Expert-cache science (offline; streaming-mode work only)

**Trigger**: only when 35B-class streaming (`VITRIOL_MODE=stream`) work
resumes per P-1. Until then: park. The transferable discipline is already
recorded in §2.4.

**Hypothesis**: H4 — histogram-pinned hot sets + trace replay predict
hit-rate-per-GiB better than capacity-proportional allocation ("allocated
well beats allocated big"; kimi: 128 GB well > 224 GB badly).

**Method (when triggered)**: record per-(layer,expert) access trace from a
real 35B stream run (12 KB/token — cheap); replay LRU/LFU/histogram-pin/
capacity-sweep offline; produce hit-rate-vs-budget curves; only then a live
A/B with the winning policy.

**Document**: curve set + policy verdict; feeds the residency-rule record.

### E5 — Vulkan sidecar truth-finding for sm_61

**Hypothesis**: H5 — on the 1070 Ti, proprietary-driver Vulkan decode ≥ CUDA
decode (upstream #19817: +47% on a 1060; OurobourOS measured only parity on
its 3060 — the Pascal claim is inherited, not replicated).

**Method**: glslc/Vulkan build of the same tree; `llama-bench` 9B Q6_K/Q8_0
pinned to GPU 1 (CUDA_VISIBLE_DEVICES/nvidia-smi pinning as appropriate);
identical flags to §3. Compare against build-ku2 CUDA numbers from E1.

**Pass/fail**: either number is the result; the claim gets closed with own
measurement. Escapes-hatch value: if CUDA 12.x is ever orphaned for sm_61,
this documents the fallback.

**Document**: `.opencode/plans/e5-vulkan-sm61-2026-09-01.md`. Cost medium;
schedule at leisure — after E1/E2/E3.

### E7 — dp4a/lop3 TurboQuant vec_dot (speculative; blocked on E3)

**Hypothesis**: H7 — the Microsoft-BitNet lop3 2-bit→int8 decode +
`__dp4a` + warp-reduce GEMV shape (OurobourOS `bitnet_kernels.h:24-83`,
MIT) beats current TQ-family vec_dot on sm_61, and generalizes to tq3_0 KV
dequant.

**Method**: only after E3 oracle exists. Re-derive (license: MIT, attribute;
PROVENANCE header) as a gated kernel; parity gate first; microbench
R∈{1,2,16}; end-to-end only on microbench win.

**Cost**: high. **Explicitly deferred.**

---

## 5. Schedule and decision gates

| step | work | gate to proceed |
|---|---|---|
| 1 | E1 merge + build | clean build both archs; A-list commits verified in tree |
| 2 | E1 bench matrix | H0 (no >2% regression) — **if fail: bisect merge, do not proceed to swap** |
| 3 | E1 depth cert + H1b + E6 | pass/fail recorded; daily-driver swap decision |
| 4 | E3 oracle | sanity gate (injected perturbation caught) |
| 5 | E2 LUT GEMV | parity gate before any bench |
| 6 | E5, then E4/E7 as triggered | each gated on its own hypothesis |

Blockers/decisions logged in the session log as they arise.

## 6. Risk register

| risk | mitigation |
|---|---|
| merge conflicts in VITRIOL-owned files | conflict policy in E1 step 2; per-file review; build + bench gate before swap |
| #25635 FA path is sm_70+-only | H1d expects this; static code check before bench; null result documented |
| CUDA 13 nvcc drops compute_61 on rebuild | record toolchain in fingerprint; CUDA 12.x sidecar path documented by OurobourOS §16.3c |
| benching collides with user's live Officina server | P-10: stop unit for bench windows, restart after; keep windows short |
| LUT GEMV only wins at high R | H2 anticipates it; result parked, not shipped |
| depth certs slow (hours) | run once per candidate build, only after shallow gate passes |
| upstream moves during program | re-sync deferred to next cycle; this plan pins surveyed tip `9d817213a0` |

## 7. Immediate next actions (this session)

1. E1 step 1–2: fetch + merge + build.
2. E1 step 3–5: static FA-arch check + bench matrix.
3. E1 step 6–8: depth cert, restore timing, lazy-mode A/B.
4. E3 tool.
5. Log everything; leave daily-driver server running on the best build.
