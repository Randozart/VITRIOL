# MoE Hook Revival + README Rewrite — 2026-09-08

## Motivation

VITRIOL's MoE optimization layer — LRU VRAM cache, expert pinning, predictive
prefetch, approximate output cache, expert pruning — is fully implemented in
`ggml-cuda/vitriol-cuda-integration.cpp` but has **zero callers** in the CUDA
dispatch path. The SYCL backend (`ggml-sycl.cpp`) wires the equivalent hooks
and is the reference implementation. The RAM Shot buffer type itself is alive
and is the daily-driver allocation mechanism; only the acceleration layers are
disconnected.

Second goal: the README is ~70% War I content (35B MoE streaming, Chimera,
single GPU). It must reflect the actual mission — *optimal inference on
constrained hardware* — while preserving the human-authored record (name lore,
Officina, Ars Priori).

## Part 1 — CUDA hook revival

### Reference: SYCL hook points (already live in `ggml-sycl.cpp`)

- Fused-path bypass (L5026): when VITRIOL streaming active, skip fused MMVQ so
  all experts route through the per-expert LRU loop.
- Per-expert LRU ensure + sync + prefetch (L5082-5098, L5162-5178).
- E34 profiler record per MUL_MAT_ID (L5042).
- Op-support gate (L6218): non-MUL_MAT_ID ops reject VITRIOL bufts.
- Buffer-type acceptance (L6647); registry init (L7071).

### CUDA insertion points (`ggml_cuda_mul_mat_id`, ~L2025-2180)

1. Function entry: `vitriol_predictor_prefetch()` fires async DMA for predicted
   experts before path selection.
2. Fused-path guard: when VITRIOL streaming buffer active, disable MMVQ fused
   fast paths (route through per-expert path where LRU lives) — mirror SYCL.
3. Per-expert loop (fallback path): `vitriol_lru_ensure()` per expert; remap
   src0 slice data pointer to VRAM on hit; `vitriol_ensure_expert_locked()`
   when lazy lock active; `vitriol_output_cache_lookup()` skip-on-hit.
4. Per-expert loop epilogue: `vitriol_lru_mark_compute_done()`,
   `vitriol_output_cache_store()`.
5. Function exit: `vitriol_predictor_update()` records actual expert ids.

Also wire the `vitriol_*_ensure`/prefetch calls into the MMVQ/MMQ/MMF fast
paths where `src0->data` is read directly (pin/LRU remap before kernel).

### Gating — zero overhead when off

- Guard all hooks: buffer is a VITRIOL buft AND `vitriol_mode() != DISABLED`.
- Per-subsystem: `vitriol_predictive_enabled()`, `vitriol_pin_active()`,
  `vitriol_output_cache_active()`, `vitriol_lazy_lock_active()`.
- `VITRIOL_MODE=off` (resident default) → no hooks fire, no branch cost beyond
  an inline predicate check.

### Build/verify

- `cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86" -DGGML_CUDA=ON ...`
- Smoke: `VITRIOL_MODE=off` — no behavioral change.
- A/B on MoE model (Mellum2-12B) when available: stream mode + hooks vs
  passive host-DMA baseline.

## Part 2 — README rewrite

### Preserve verbatim
- Why VITRIOL (human, L9-13)
- Officina section (L15-60)
- Ars Priori & Acknowledgements (L608-732) — historical record of inspiration;
  add citations where gaps exist.

### Distill
- Three Wars → concise factual "History" record (eras + verdicts), link to
  VERDICTS.md.

### Remove
- Emulated Memory Architecture (L522-554) → one-liner: superseded by Officina's
  built-in memory.

### Rewrite (technical, reflect current state)
- What Is It → mission framing: optimal inference on constrained hardware.
- Quick Start: CUDA_ARCHITECTURES 61;86, profiles, systemd note.
- Configuration table: n_max=1, ts-splits, TurboQuant, VITRIOL_KV_*,
  cache-ram, ctx-checkpoints.
- How It Works: residency primary + streaming optional + dual-GPU + context
  lifecycle + MTP.
- Performance: current 27B numbers, depth≠window, history labeled.
- Hardware/compat: dual-GPU, SYCL doc note, tested list.
- Architecture: dual-GPU diagram.
- Durability: systemd, OOM shield, fingerprint discipline.
- Project structure: add officina/, profiles/, libvitriol/, systemd/.
- Differentiating features list: what sets VITRIOL apart (add).

## Status

2026-09-08 12:25 — plan written. Commence Part 1.
2026-09-08 — Part 1 DONE: CUDA hooks wired, built (sm_61;86), committed
  (llama.cpp 23d951b04, outer 7bd2846). Part 2 DONE: README rewritten,
  human sections verified verbatim (Why VITRIOL, Officina, Ars Priori),
  memory mode reduced to superseded one-liner, History distilled from
  three wars. Runtime A/B on MoE model deferred (no MoE model present);
  VITRIOL_MODE=off path verified inert by construction (src0_eff == src0,
  vitriol_route_lru == false).
