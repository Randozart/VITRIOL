# Session Log — 2026-08-30 — OurobourOS cross-pollination: the 2x anomaly, bisection, and the kernel transplant

Provenance: joint session with the OurobourOS project (sister project, same
owner). OurobourOS's plan of record for this work is `PLAN.md` §16.3c/d over
there; this log is the VITRIOL-side record of the same measurements.

## 1. The anomaly

OurobourOS M2 bridge benchmarks measured the same 9B Q6_K model at
**22.9 t/s tg** on the VITRIOL llama.cpp CUDA build and **42.4 t/s** on the
microsoft/BitNet fork build — identical card (RTX 3060), near-identical
CMake flags. A 2x gap with no explanation was unacceptable to either
project, so we bisected it.

## 2. Forensics and bisection

1. **Toolchain**: VITRIOL's `build/` was configured Aug 18 against
   `/usr/bin/nvcc`, which no longer exists (toolchain moved to
   `/opt/cuda`, CUDA 13.3.1). Rebuilding the same source with CUDA 13.3
   (`build-cu13`) recovered **+17% tg** (22.9 → 26.9). Real, not the story.
2. **VITRIOL's CUDA patches exonerated**: the Vulkan backend never runs
   `ggml-cuda.cu` hooks, yet VITRIOL's Vulkan build measured 27.8 vs the
   fork's 43.9 — the gap is backend-independent.
3. **Verdict: ggml base age.** VITRIOL forked llama.cpp around PR #21370
   (Feb 2025). Upstream's decode vector kernels (mmvq) and the new
   mmvq+GLU fusion path landed since. The 60% tg gap matches the mmvq
   improvement profile: pp (compute-bound) only 12-20% behind, tg
   (bandwidth-bound vector path) 60% behind.

Build notes encountered on the way (all fixed):
- CUDA 13.3 removed `compute_61` (Pascal evicted) — arch lists must drop 61.
- Old trees needed the upstream CUDA-13 compat shim
  (`#include <cuda/iterator>` in `argsort.cu` / `top-k.cu`).
- GCC 16 broke the shared-lib link of `cpp-httplib`; bench builds use
  `BUILD_SHARED_LIBS=OFF`.
- `vitriol-server.service` auto-respawns — stop the unit, never just the PID.
- Bench provenance recorded per row (nvcc path/version, commit, flags) —
  adopted from OurobourOS §15 discipline; recommended for all VITRIOL
  benchmarks going forward.

## 3. The transplant (`vitriol-ku` branch)

Upstream history has no common ancestor with ours (squash-only merges), so
instead of a merge we **transplanted**: fresh branch from
`upstream/master` (9723942ad, 1572 commits ahead), then re-applied
VITRIOL's ggml-layer integration by hand:

- hook files copied verbatim: `vitriol-cuda-integration.*`,
  `vitriol-buffer.*`, `vitriol_copy_engine.*`
- `ggml-cuda.cu`: includes, perf-diagnostics block (`GGML_CUDA_GDN_PROFILE`),
  graph_compute split into wrapper + `_inner` with the LULL instrumentation
  and pool-reset hook, buffer-type acceptance, backend init call
- `common.cuh`: `ggml_cuda_pool::reset()` + context `vitriol_reset_pools()`
- `ggml/include/ggml-cuda.h`: perf snapshot public API
- `CMakeLists.txt`: restored `*.cpp` in the CUDA source GLOB

Not yet ported (deliberately): the expert-LRU / pin / prefetch mul_mat_vec
hook sites (env-gated, dense-model-neutral), the TQ3/TurboQuant kernel
stack, and the server feature commits. Those are Phase 2.

## 4. Results (9B, pp512/tg128, -fa 0, -ngl 99, RTX 3060, driver 580.178.04)

| build | backend | Q6_K pp | Q6_K tg | Q8_0 pp | Q8_0 tg |
|---|---|---|---|---|---|
| VITRIOL `build/` (Aug 18, pre-13.3 nvcc) | CUDA | 1249 | 22.9 | 1301 | 20.7 |
| VITRIOL `build-cu13/` (same source, CUDA 13.3) | CUDA | 1149 | 26.89 | 1350 | 22.41 |
| **VITRIOL `build-ku/` (upstream base)** | **CUDA** | **1509** | **42.08** | **1734** | **36.36** |
| **VITRIOL `build-ku/` (upstream base)** | **Vulkan** | **1376** | **42.85** | — | — |
| BitNet fork `build-m2/` (reference) | CUDA | 1535 | 42.4 | — | 42.4* |
| BitNet fork `build-m2/` (reference) | Vulkan | 1375.8 | 43.87 | 987.8 | 35.91 |

*fork Q8_0 row from a different session; treat as approximate.

**VITRIOL is back at parity: +84% tg on CUDA (22.9 → 42.1), Vulkan 42.9.**

Differential gate: fixed greedy prompt (`The capital of France is`, 48
tokens) produces **byte-identical output** on build-ku and the fork build —
the transplant is behaviorally clean.

## 5. What OurobourOS shared with us

- The bisection method (one variable per build, provenance per row) —
  already our own §15 instinct, applied to toolchains.
- The observation that **Vulkan ≥ CUDA on our cards for decode** (43.9 vs
  42.4 same-binary) — VITRIOL should keep serving with `-fa on` Vulkan
  builds as a first-class path, not a fallback. The 1070 Ti (now live on
  nvidia-580xx) is **Vulkan-only** for CUDA 13 era toolchains.
- The finding that the tg gap was base-age, not our patches — which is
  what made the transplant the obvious fix instead of endless tuning.

## 6. Pending (Phase 2)

1. ~~Port the server feature commits onto the upstream server~~ — DONE 2026-08-31:
   llama-server on build-ku serves port 8279, `--checkpoint-min-step`, `-fa on`,
   `exclude_secondary = true` (single-GPU CUDA 13), build symlink.
2. ~~Port the TQ3/TurboQuant stack onto the new kernels~~ — DONE 2026-08-31:
   commit c84097e10. block defs, tq3-native/prefill, turbo-wht, vecdotq/mmq/
   mmvq/convert/getrows/set-rows dispatch, ggml-quants WHT+Lloyd-Max, model
   loader. Tests: test-tq3-cuda 11/11, test-tq3-load-tiles 4/4,
   test-tq3-prefill PASS. Build sm_86 only (CUDA 13 dropped Pascal).
3. Port the expert-LRU/pin/prefetch hook sites if MoE serving resumes.
4. ~~Commit the `vitriol-ku` branch~~ — DONE: c84097e10.
