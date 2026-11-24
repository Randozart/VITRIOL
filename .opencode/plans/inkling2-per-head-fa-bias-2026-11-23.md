# Per-head relative bias in flash attention — remove the O(ctx) bias materialisation

**Date:** 2026-11-23 · **Status:** implementing · **Predecessor:** `inkling2-serve-profile-2026-11-23.md`

## Problem, measured

Inkling 2 Mini runs at 1–18 % GPU utilisation, ~45 W on a 170 W card, 19.7 t/s prefill
and 5.5 t/s decode at 3.9k depth. Root cause, traced to source:

1. `src/models/inkling2.cpp:411-415` builds a dense per-head bias
   `kq_b = get_rows(table, rel_pos) -> [n_head, n_kv, n_tokens]` in F32.
2. `src/llama-graph.cpp:2670`: `use_flash_attn = cparams.flash_attn && kq_b == nullptr`.
   A bias present ⇒ flash attention is **off** ⇒ dense scores are materialised too.

Buffer arithmetic checks out against the failures:

| ctx | ub | per full-attn layer | measured pp alloc |
|---|---|---|---|
| 204800 | 32 | 629 MB | 1551 MB |
| 204800 | 64 | 1259 MB | 3031 MB (cudaMalloc fail) |
| 131072 | 64 |  806 MB | 2123 MB |

Only the **4 full-attention layers** pay this (the 20 SWA layers have `n_kv = 1024`), and
the bias *table* is only 1024 wide (`n_relative_extent = 1024`) — the information is
bounded, only the layout is O(ctx). That is the depth wall and the ubatch ceiling at once.

## Why it needs a kernel

`ggml-cuda/fattn.cu:127` requires `mask->ne[2] == 1` — the FA mask is broadcast over heads.
Our bias is genuinely per-head (`table[h, i, clamp(pos_i - pos_j)]`). No graph algebra
fits a per-head bias into a head-broadcast mask, which is why upstream's version of this
is a ~2000-line change.

## Stages

| stage | work | gate |
|---|---|---|
| **C1** | F16 bias in the inkling2 graph (halves the buffer, no semantics change) | loads at `ub 64`/200k; output identical to the F32 baseline at `ub 32`; decode within 2 % of 16 t/s short-context |
| **C2** | per-head bias channel in `ggml_flash_attn_ext`: optional extra tensor with `ne[2] == n_head`, **CPU reference implementation first** | CPU FA-with-bias matches the dense path (max abs diff ≤ 1e-4 on a fixed fixture) |
| **C3** | CUDA FA kernels consume the per-head bias; `use_flash_attn` no longer requires `kq_b == nullptr` when the model opts in | identical tokens to C1 output on the model golden |
| **C4** | wire `inkling2` to pass `(bias table, rel_pos)` and let FA run on all layers | `ub 128` loads at 200k; decode at 3.9k materially better than 5.5 t/s; GPU util up |

## Invariants

- **I1 — other models untouched.** With no per-head bias tensor, every kernel path is
  unchanged. *Pinned by:* the 27B golden completion (text md5) and `kq_b == nullptr`
  remaining the only FA condition for models that do not opt in.
- **I2 — C1 is semantics-preserving.** F16 bias produces the same *tokens* as F32 at
  `ub 32`. *Pinned by:* 32-token `--temp 0` output md5, before and after.
- **I3 — CPU reference is the arbiter for C2.** FA-with-per-head-bias equals the dense
  path on CPU within a stated tolerance. *Pinned by:* a new test in `tests/` comparing
  both paths on a fixed synthetic fixture (random Q/K/V + a known bias, both dtypes).
- **I4 — the ceiling moves.** `ub 128` at 200k loads without `cudaMalloc` failure.
  *Pinned by:* the load test (the exact check that failed at ub 64 before).
- **I5 — the wall moves.** Decode at ~3.9k depth improves over the 5.5 t/s baseline.
  *Pinned by:* the depth benchmark, recorded verbatim.
- **I6 — SWA layers unaffected.** `n_kv = 1024` layers keep their current path.
  *Pinned by:* per-layer timing comparison, plus I2's identical output.

## Named tests

- `tests/test-flash-attn-bias.cpp` (new): dense-path vs FA-with-head-bias, CPU, fixed
  fixture, both `f16` bias and `f32` scores; asserts max abs diff and reports it.
- Golden md5: 32-token `--temp 0` completion on the live engine (I2, I5).
- Load gate: `vitriol-server ... -c 204800 --ubatch-size 128` must not print
  `failed to allocate compute pp buffers` (I4).

---

## Results (2026-11-23): C1 refuted by measurement

**Experiment — fold the bias into the F16 softmax mask.** The softmax mask is F16 and indexes
per head (`ggml-cpu/ops.cpp:8721`), so `softmax(kq*s + mask + bias*s)` is algebraically
identical to adding the bias to `kq` first. Implemented in `build_attn_mha` (bias to F16,
repeat the causal mask over heads, add in F16), built and run:

- output identical (`local-ok`) — the fold is *correct*;
- **the pp allocation did not move**: `allocating 3031.32 MiB on device 0` at ub 64 and
  `6058.22 MiB` at ub 128, byte-for-byte the same numbers as before the change.

Why, and the arithmetic matches the observed allocation:

```
dense kq alone at ub 64 : 24 heads x 204800 kv x 64 tok x 4 B = 1258 MB
kq + softmax output     : 2517 MB            (observed 3031 MB incl. temporaries)
```

The dominant tensor is the **dense score matrix**, which exists only because
`use_flash_attn = cparams.flash_attn && kq_b == nullptr` (`llama-graph.cpp:2670`). Removing a
bias tensor cannot help while the scores themselves are materialised. Reverted — complexity
with zero measured benefit.

### Consequence for the plan

A per-head bias channel in FA is necessary but **not sufficient**: the four full-attention
layers must run through flash attention at all. That is C2/C3, and it is a kernel project —
the FA mask contract is `mask->ne[2] == 1` (head-broadcast), while this bias is genuinely
per-head.

### Environment note (not a code issue)

The host rebooted mid-session for a CachyOS update. Before the reboot CUDA was unavailable
(`ggml_cuda_init: failed to initialize CUDA: system has unsupported display driver / cuda
driver combination`, NVRM 620.70.06 vs userspace 620.73): the launcher refused to start
(exit 18, GPU probe) and a direct run fell back to CPU at 1.65 t/s decode — the tell. After
the reboot: service active, `local-ok` at 13.1 t/s. No engine regression was involved.
