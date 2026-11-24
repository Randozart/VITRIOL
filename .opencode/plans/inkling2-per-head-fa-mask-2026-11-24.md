# Per-head FA mask: letting Inkling 2 use flash attention at all

2026-11-24. Supersedes the C1 half of `inkling2-per-head-fa-bias-2026-11-23.md`
(C1 was refuted by measurement — see that file's Results section).

## Problem

`llama-graph.cpp:2670`:

```cpp
const bool use_flash_attn = cparams.flash_attn && kq_b == nullptr;
```

`inkling2.cpp:422` passes `kq_b` for **every** layer (SWA and full alike), so
`-fa on` is a no-op for the whole model: all 24 layers run the dense
O(ctx) path. Two measured consequences:

1. the depth wall — 5.5 t/s decode at 3.9k filled (vs 16.2 t/s shallow);
2. the pp-buffer ceiling — 3031.32 MiB at ub 64 / c 200k, 6058.22 MiB at
   ub 128, because the 4 full-attention layers materialise dense scores
   (`24 heads x 204800 kv x ub x 4 B` = 1258 MB at ub 64, plus softmax).

## Why this is a mask change and not a bias channel

Three facts from the fork:

- The MMA kernel adds the mask straight into the scores —
  `KQ_C[...] += slope * __half2float(tile_mask[...])`
  (`fattn-mma-f16.cuh:744`, half2 path `:810`). A per-head additive bias is
  expressible as mask values.
- `ne32` (`mask->ne[2]`) is already plumbed into both kernels and unused
  (`fattn.cu:1349` → `fattn-vec.cuh:41`, `fattn-mma-f16.cuh:1786`).
- The CPU already indexes the mask per head
  (`ggml-cpu/ops.cpp:8721`, `:9002`: `(iq2 % mask->ne[2]) * mask->nb[2]`).

The only refusal is CUDA's dispatch gate, `fattn.cu:588`:

```cpp
if (mask && mask->ne[2] != 1) {
    return BEST_FATTN_KERNEL_NONE;      // -> supports_op false -> op leaves CUDA
}
```

Algebra that makes the fold legitimate (`soft_max_ext` computes
`softmax(kq*scale + mask)`):

```
softmax((kq + bias)*scale + mask) == softmax(kq*scale + (mask + bias*scale))
```

So a biased layer can hand FA an F16 mask carrying `bias*scale` on the head
axis and set `kq_b = nullptr`.

## Stages

- **K1 contract.** Accept `mask->ne[2] == n_head` in
  `ggml_cuda_flash_attn_ext_supported` / `best_fattn_kernel`. Keep
  `ne[2] == 1` broadcast semantics byte-identical.
- **K2 MMA kernel** (prefill). Head slice `(head % ne32) * nb32` in
  `flash_attn_ext_f16_load_mask`; force `ncols2 == 1` when a per-head mask is
  present so one tile is one head.
- **K3 vec kernel** (batch-1 decode). Same head offset; verify the sm_86
  decode dispatch lands here.
- **K4 graph wiring.** In `build_attn_mha`: when `kq_b != nullptr`, fold
  `bias * kq_scale` into an F16 per-head mask, pass `kq_b = nullptr`, and
  allow FA. `VITRIOL_FA_BIAS=0` restores the dense path for A/B.
- **K5 tests.** `tests/test-fa-bias.cpp` — dense-with-bias vs
  FA-with-folded-mask on a fixed fixture, CPU then CUDA, `max|d| <= 1e-4`;
  `test-backend-ops -o FLASH_ATTN_EXT` gains a `ne[2] == n_head` case and
  keeps the `ne[2] == 1` case as the negative control.
- **K6 measure.** Decode at 3.9k (target: materially better than 5.5 t/s),
  prefill, ub 64/128 loadability at c 200k, GPU utilisation.

## Invariants

- **I1 (parity for everything else).** Any graph with `kq_b == nullptr`
  produces the same kernels and the same tokens as today. *Pinned by*
  `test-backend-ops -o FLASH_ATTN_EXT` (existing `ne[2] == 1` cases) and the
  27B golden md5 in `bench/fa-bias-gate.sh`.
- **I2 (correctness — the load-bearing one).** With a per-head bias, the FA
  path agrees with the dense path to `1e-4` on the fixed fixture. *Pinned by*
  `tests/test-fa-bias.cpp` (CPU reference) and its CUDA arm.
- **I3 (no silent fallback).** A per-head mask either executes on CUDA or the
  graph build fails loudly; it must never be quietly evicted to CPU. *Pinned
  by* `test-fa-bias --assert-supported` (asserts
  `ggml_backend_supports_op` is true for the `ne[2] == n_head` fixture on the
  CUDA backend).
- **I4 (the ceiling actually moves).** c 200k with ub 64 loads. *Pinned by*
  the load gate in `bench/fa-bias-gate.sh`.
- **I5 (tokens unchanged).** Temp-0 output for a fixed prompt matches the
  current dense build's md5. *Pinned by* the same gate script.
- **I6 (no regression elsewhere).** The 27B daily-driver profile decodes
  within noise of its recorded baseline. *Pinned by* the gate script's
  second arm.

## Measurements to record

| what | before | after |
|---|---|---|
| decode @3.9k | 5.5 t/s | ? |
| prefill @3.9k | 19.7 t/s | ? |
| decode shallow | 13.1–16.2 t/s | ? |
| pp alloc ub 64 / c200k | 3031.32 MiB (fails) | ? |
| pp alloc ub 128 | 6058.22 MiB (fails) | ? |

---

## Results (2026-11-24)

### Shipped: the per-head mask channel

Commit `e0ad53e`. Implemented exactly as planned, with one deviation forced by
the compiler and one bug found by measurement:

- `best_fattn_kernel` accepts `mask->ne[2] == n_head`, scoped to 128-dim
  heads because the `ncols2 == 1` tile layout is only instantiated there.
- `ncols2` pinned to 1 for a per-head mask (one head per tile).
- The mask pointer gains `nb32 * (head % ne32)` in **three** launchers:
  the MMA main loop, the MMA fixup loop, and the VEC kernel.
- `llama-graph` folds a layer's `kq_b` into an F16 per-head mask and leaves
  `kq_b` null so FA can run; `VITRIOL_FA_BIAS=0` restores the dense path.
  The fold is currently confined to windowed layers (`kq_mask->ne[0] <= 4096`),
  because an expanded per-head mask is n_head copies of the causal mask
  (~500 MiB at 200 k) and only pays where `n_swa` bounds it.

**The bug that took the longest** and is worth remembering: after the first
patch, `nb=1` passed and `nb=32` failed with NMSE 0.38. The MMA file has TWO
launchers with their own `mask_h` — the main loop and the post-stream-K fixup
loop. `nb=1` stayed in one block and never hit the fixup launcher; `nb=32`
split K across blocks and read the mask through the unpatched second launcher.
The identical-slices diagnostic case isolated it: a per-head mask whose slices
are all equal to head 0 passes even when the head offset is ignored, so a
passing identical-slice case with a failing varying-slice case means "one
slice is being read for all heads".

### Gate results

| gate | result |
|---|---|
| `test-backend-ops -o FLASH_ATTN_EXT` | **2964/2964 pass, 3/3 backends** (CUDA0, CUDA1, CPU) |
| broadcast (`ne[2] == 1`) cases | unchanged — I1 holds |
| per-head cases (varying, identical slices, `nr23=[3,1]`) | pass — I2, I3 hold |
| 27B daily driver | serving, `local-ok` — I6 holds so far |

### Not yet achieved: the 200 k load

Inkling cannot load at c 204800 in the current environment, and **the patch is
not the cause** — proven twice:

| arm | CUDA0 compute buffer requested |
|---|---|
| current graph, `VITRIOL_FA_BIAS=0` (dense everywhere) | 7734366464 |
| current graph, fold on | 7734366464 (identical) |
| **pre-patch graph** (`e0ad53e~1`) | 8513210112 |

The pre-patch graph requests *more* than the patched one, so the fold is a
(partial) improvement, not a regression. The earlier working budget was
1551 MiB at ub 32; today the very first reserve asks for 7.2–7.9 GiB, which no
arrangement of a 17.6 GB model over 2×12 GB can satisfy alongside the weights.

So I4 stays open, blocked on a memory-accounting question that is independent
of flash attention.

### The reserve is O(ctx), ub-independent, and FA-independent

Three probes narrow it decisively (c 131072, ts 1,1 unless noted):

| variable | CUDA0 compute buffer |
|---|---|
| ub 1 | 4583211648 |
| ub 8 | 4807338752 |
| ub 32 | 5587683072 |
| ub 32, `VITRIOL_FA_BIAS=0` (dense everywhere) | 5587683072 (identical) |
| ub 32, fold on **all** layers (full-attention too) | 5587683072 (identical) |
| c 32768, 65536 | no failure (fits) |
| c 204800 | 8513210112 (pre-patch graph also 8513210112) |

Reading: a **ub-independent floor of ~4.3 GiB** plus ~32 MiB per ubatch token.
The floor is ~35 KB per kv position and scales linearly with ctx; enabling or
disabling flash attention on those layers changes it by **zero bytes**.

So the ceiling was never the attention bias. The dense scores cost ~32 MiB per
ubatch token (which the fold does remove — that is the ub-dependent part), but
the dominant term is a ctx-proportional buffer that FA cannot touch. Fixing the
200 k load means finding that tensor, and it deserves its own single-objective
stage: dump the scheduler/gallocr node list at c 131072 and sort by size. The
prime suspect is the per-layer input taps added for DSpark
(`cparams.embeddings_layer_inp[il]` → `t_layer_inp[il]`, `[n_embd, n_tokens]`
F32): ctx × n_embd × 4 B = 1.2 GB at 131072 for a single tap, and several
tapped layers of that shape land squarely on the observed floor.

Note this is a *pre-existing* cost, not a regression from this work: the
2026-11-23 figure of 1551 MiB at c 204800 was measured in a different
invocation than today's, and today the pre-patch grammar asks for the same
8513210112 as the patched one.
