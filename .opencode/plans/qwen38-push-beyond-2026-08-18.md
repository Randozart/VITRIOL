# Qwen3.8-27B Push-Beyond-14t/s — Comprehensive Plan

Date: 2026-08-18 19:30
Session goal: push past the 14.09 t/s (131K + MTP n5) plateau via free tests → upstream pulls → kernel engineering.

## Context / Established Baseline

Hardware: RTX 3060 12GB (sm_86, Ampere) + GTX 1070 Ti 8GB (sm_61, Pascal), PCIe gen3 x8 under load.
Model: `~/Downloads/Qwen3.8-27B-Q3_K_M.gguf` (qwen35 arch, dense, 64 layers, 48 recurrent GLA + 16 full-attn).

| Config | t/s | Notes |
|---|---|---|
| ts=24,12, no MTP, 262K | 11.03 | max ctx |
| **ts=24,12, MTP n5, 131K** | **14.09** | current winner |
| ts=24,12, MTP n8/n10 | 14.04/14.07 | plateau confirmed |
| MTP real-reasoning | 13.04 | thinking overhead |

### Root-cause of the plateau (confirmed)
- MTP verify of a 5-token draft batch = 308ms vs 91ms single-token forward = **3.4×**
- Cause: 48/64 layers are **serial-scan Gated Linear Attention** (`ggml-cuda/gla.cu` is a naive per-token f32 loop). Verify processes each draft token through all 48 recurrent layers sequentially → verify cost scales linearly with n → n=5/8/10 identical.
- Draft head is cheap (dense attention-only, 15ms/tok). PCIe NOT the bottleneck (gen3 x8 under load).

### Theoretical ceiling
Chunked/parallel-scan GLA would let verify batch the draft → verify(5) ≈ 80ms → ~155ms/5tok ≈ **31 t/s (2.2×)**.

## Phase A — RESULTS (2026-08-18 19:45)

| Test | t/s | Verdict |
|---|---|---|
| A0 baseline 24,12 n5 | 13.54 (14.09 earlier) | ~13.5-14.1 noise band |
| A1 ts=26,10 | 13.49 | worse — ts 24,12 optimal |
| A2 ts=28,8 | OOM (GPU0) | n/a |
| A3 n=3 | 13.02 | worse — n5 not over-drafting |
| A4 draft-ngl 0 | 13.57 | neutral |
| A5 ubatch 512 | OOM (MTP compute buffers) | n/a |
| A6 (no checkpoint) | = baseline | raw runs already checkpoint-free |

**Conclusion: config already optimal. Plateau is kernel-bound. Phase C is the only path.**

## Phase B — RESULTS (2026-08-18 19:55)

All three upstream candidates rejected after conflict analysis:
- `a035a8887` (spec counters) — depends on newer `server_slot` API (n_draft_verif_steps/n_accepted_per_pos) our fork lacks. Would need self-authored port.
- `9a688e51e` (MTP mem fit) — depends on `load_mtp` param + `llama_model_n_layer_nextn` (newer model API). Only affects auto-fit path (n_ctx=0) which we don't use.
- `1692f9e50` (RS rollback) — targets `ggml_ssm_scan` (Mamba/Nemotron), NOT our `ggml_gated_linear_attn` op.

CUDA GLA kernel (`gla.cu`) untouched in upstream for years — no kernel help upstream. **Phase C is self-authored.**

## Phase A — Free config tests (~40 min, server-side only)

All against 131K + MTP n5, benchmark = 1 warmup + 3×64-token greedy (bench.py).

| # | Test | Rationale | Success criteria |
|---|---|---|---|
| A1 | `-ts 26,10` @131K+MTP | GLA is fp32-compute-bound; 3060 ≈2× Pascal fp32 → shift work to 3060 | >14.5 t/s |
| A2 | `-ts 28,8` @131K+MTP | push further toward 3060 if A1 helps | >A1 |
| A3 | `--spec-draft-n-max 3` | test if positions 4-5 are dead weight | ~=14.09 (then n5 is fine) or > (over-drafting) |
| A4 | draft head forced to GPU0 | keep MTP head off Pascal | marginal, low-risk |
| A5 | `--ubatch 512` | bigger verify batch, fewer graph splits | >14.09 |
| A6 | drop `--checkpoint-every-n-tokens` | checkpoint overhead during bench | >14.09 |
| A7 | `--main-gpu 1` sanity | KV/scratch placement | document, likely worse |

Decision gate: keep best ts; if A1/A2 trend, refine around peak.

## Phase B — Upstream cherry-picks (build-side, low-conflict additive)

1. **`a035a8887`** — server spec-decode counters (`/metrics`): `spec_decode_num_accepted_tokens_per_pos_total`. Gives acceptance-per-position → validates A3 (over-draft detection). Touches `server-context.cpp`+44, `server-task.*`+5.
2. **`9a688e51e`** — MTP layer memory fit fix (`fit.cpp`, `llama-model.cpp`). Could unlock **262K + MTP** (currently compute-buffer OOM). If fits: max ctx + MTP both.
3. **`1692f9e50`** — recurrent state rollback for `ggml_ssm_scan` (`ggml-cuda/ssm-scan.cu` etc). Correctness for DeltaNet recurrent state under `--kv-unified --cache-idle-slots` / ctx shifts. Re-derive/CUDA-specific portions only.

Process: `git cherry-pick`, resolve conflicts (our MTP edits are separate hunks), rebuild with `-DCMAKE_CUDA_ARCHITECTURES="61;86"`, `sudo vitriol setup`, retest 131K+MTP + 262K+MTP.

### Excluded (documented decisions)
- Spec auto-detect commits (1d2869c6e, f65e568fd) — for separate-draft models (`-md`), not our embedded-MTP path. Skip unless a draft model is added later.
- DSpark/DFlash/eagle3 — DeepSeek/GPT-OSS archs, not qwen35.
- Vulkan/SYCL/OpenVINO GLA work — not our CUDA path. The SYCL gated-delta-net fusion (3d9388535) is a *pattern* reference for our CUDA fusion, not a cherry-pick.

## Phase C — Kernel engineering (VITRIOL core goal)

## Phase C — CORRECTED KERNEL DIAGNOSIS (2026-08-18 20:10)

**IMPORTANT CORRECTION**: The bottleneck op is `GGML_OP_GATED_DELTA_NET`
(kernel `gated_delta_net_cuda` in `ggml/src/ggml-cuda/gated_delta_net.cu`),
NOT `gated_linear_attn` (gla.cu — that's RWKV6's op). qwen35 recurrent layers
use delta-net.

Established facts:
- qwen35 = 64 layers, every 4th is full-attention → **48 are Gated Delta Net** (recurrent).
- Graph branches on n_tokens (`delta-net-base.cpp` `build_delta_net`):
  - `n_tokens==1` (decode) → fused AR kernel (no loop)
  - `n_tokens>1` (verify/prefill) → fused keep_intermediates (`fused_gdn_ch=true` default,
    llama-context.cpp:192) → **same `gated_delta_net_cuda` with serial `for t in n_tokens`**
    (gated_delta_net.cu:57)
- A chunked graph path (`build_delta_net_chunking`, CS=64/16) exists but is bypassed
  when fused_gdn_ch=true.
- **MTP verify (n_tokens=6) → serial loop ×6 tokens ×48 layers** = the 3.4× verify wall.
  Chunking graph path gives no benefit at n_tokens=6 (1 tiny chunk + huge per-chunk
  decay_mask/transpose/mul_mat overhead).

### C1. Parallel-scan delta-net kernel for small-n_tokens (the real target)
- Replace `gated_delta_net_cuda`'s serial token loop with a **parallel scan** for the
  verify batch: the delta-net state update `S = g*S + k*delta^T` is a linear recurrence.
  Compute each of the n_tokens positions' state contribution in parallel, then compose
  transitions — cutting the 48×6 serial chain to 48×(log or 1) sequential steps.
- Alternatively: a dedicated small-batch kernel that parallelizes across the 6 verify
  positions using the chunked math but without the heavy graph decomposition.
- Payoff: verify → closer to single-forward → ~2× toward 31 t/s.
- Risk: correctness (diff vs current serial kernel); the verify is f32 on Pascal too.

### C2. fp16/bf16 delta-net kernel (secondary, deferred)
- f32-only today. Ampere 3060 fp16 ≈2×; Pascal no fast fp16 → per-device dispatch or
  ts-weighting needed. Defer until C1.

### C3. Fuse delta-net state writeback
- The kernel already writes state in the tail (gated_delta_net.cu:140). Low remaining
  headroom; do alongside C1 only if cheap.

## Phase D — Documentation
- Update `.opencode/plans/qwen38-dual-gpu-findings-2026-08-18.md`
- New `.opencode/plans/qwen38-gla-kernel-engineering-2026-08-18.md` with profiling data, kernel design, benchmarks.
- AGENTS.md: add GLA kernel section + updated MTP findings (n5 verify-bound, not draft-limited).

## Sequencing
1. Write this plan ✓
2. Phase A tests ✓ (no free lever)
3. Phase B pulls ✓ (none fit — API drift)
4. Phase C0 diagnosis ✓ (delta-net kernel, corrected)
5. Phase C1 parallel-scan delta-net kernel (in progress)
6. Phase D docs

## Risks
- Chunked GLA correctness (mitigate: reference-scan comparison, existing test vectors)
- Pascal fp32-bound GLA may cap gains even with chunking (chunking removes serialization, but per-element fp32 cost remains; 3060 carries the 26-share → expect ~2× not ~4×)
- Cherry-pick conflicts with VITRIOL MTP edits (mitigate: additive commits, separate hunks)
