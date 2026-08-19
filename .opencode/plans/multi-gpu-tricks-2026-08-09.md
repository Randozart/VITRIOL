# Multi-GPU Follow-up Tricks Backlog

Date: 2026-08-09
Status: **A, B, C-lite implemented 2026-08-09** (verified on single 1070 Ti).
D-e/F remain backlog (need 3060 + MoE model → real tuning).
Location: `.opencode/plans/multi-gpu-tricks-2026-08-09.md`
Depends on: per-device VITRIOL plumbing landed in
`.opencode/plans/multi-gpu-2026-08-09.md`

Everything else below is enabled by the now-per-device DMA/LRU/pin machinery.
Ranked by value ÷ effort. Modes: **quick win**, **follow-up**, **rejected (with reason)**.

---

## A. Per-GPU resource budgets — IMPLEMENTED

- Env knobs now live:
  - `VITRIOL_LRU_MB_<d>` — per-device LRU pool override (wins over `VITRIOL_LRU_MB`).
  - `VITRIOL_PIN_FIRST_N_LAYERS_GPU<d>` — per-device pin range (wins over global).
  - `vitriol_pin_enabled()` now considers per-device overrides, so a GPU-specific
    range can enable pinning even when the global is 0; alloc-failure is flagged
    per device (no global toggle-off anymore).
- Files: `ggml/src/ggml-cuda/vitriol-cuda-integration.{cpp,h}`, `ggml-cuda.cu` gate.
- Verify when 3060 lands: `VITRIOL_LRU_MB_0=1536 VITRIOL_LRU_MB_1=512`,
  `VITRIOL_PIN_FIRST_N_LAYERS_GPU0=14 VITRIOL_PIN_FIRST_N_LAYERS_GPU1=6`.

## B. Per-device KV quantization — IMPLEMENTED

- Env knobs:
  - `VITRIOL_KV_QUANT_GPU<d>` — K+V quant for CUDA device d (e.g. `q8_0`).
  - `VITRIOL_KV_QUANT_K_GPU<d>`, `VITRIOL_KV_QUANT_V_GPU<d>` — per-channel wins.
- Applied in `llama_kv_cache` per layer, resolved through the layer's buft device
  (CPU layers → gpu idx -1 → no override). Verified: `VITRIOL_KV_QUANT_GPU0=q8_0`
  flipped all 0.6B layers to q8_0 on the 1070 Ti.
- Files: `src/llama-kv-cache.cpp`.

## C. Compute-aware `--tensor-split` — C-lite IMPLEMENTED

- `vitriol calibrate` now emits a compute-aware split hint when ≥2 GPUs present:
  `--tensor-split <perf-weight per device>` (Pascal 10 / Turing 16 / Ampere 26 /
  Ada 36), e.g. 1070Ti+3060 → `--tensor-split 10,26`.
- Heuristic only — the measured per-card layer-timing autotune stays on the
  3060-arrival roadmap.
- Files: `libvitriol/src/estimator.rs`, `main.rs`.

## D. Dual-context server (backlog — needs 3060)

- run `--parallel 2` with ctx split; per-device streams already make stream mode
  survive it. Re-measure on hardware (old trap was one global LRU stream).

## E. Boundary-layer expert pre-replication (follow-up)

- **Problem:** the layer-group boundary pays a host-staged handoff each token;
  on an MoE model the boundary layer's expert weights also get ping-ponged.
- **Change:** prefetch the boundary layer's top experts into **both** LRU pools
  to hide the handoff behind compute.
- **Where:** predictor (`vitriol_predictor_prefetch`) — add a dual-pool prefetch
  when `layer_idx` crosses a device boundary.

## F. Hot-expert pinning from prefill stats (follow-up)

- **Problem:** `VITRIOL_PIN_FIRST_N_LAYERS` pins an arbitrary prefix; actual
  expert heat varies per layer.
- **Change:** during a prefill warmup, record expert fire counts (predictor
  already tracks `g_cur_exp`/temporal data) and pin the hottest expert tensors
  onto the fast card instead of first-N-by-layer.
- **Value:** better cache ROI per MB of pinned VRAM.

---

## Rejected

- **G. P2P direct DtoD boundary copy.** Skips the host bounce but saves
  ~1-2 µs/token ≈ 0% of a 100 ms token. `cuDeviceCanAccessPeer` also not
  guaranteed between Pascal and Ampere. Not worth it.
- **H. Prefill-on-3060 / decode-on-1070Ti bucketing.** KV cache would live on
  the wrong card → KV migration every token. Invasive surgery, net loss.

---

## Suggested order

1. **A + B + C-lite** — DONE 2026-08-09 (verified single-GPU, no regression:
   server boot + per-device KV quant env, sweep 93.29 t/s).
2. **D** — biggest aggregate capability (2× server throughput), needs 3060.
3. **C-full** — measured per-card layer timing when the 3060 lands.
4. **E + F** — only if stream-mode sweeps show boundary/expert-cache stalls.

## Implementation status (2026-08-09)

| trick | status | verified |
|---|---|---|
| A. per-GPU LRU/pin budgets | implemented | compile + boot (dense model, expert path latent) |
| B. per-device KV quant | implemented | `VITRIOL_KV_QUANT_GPU0=q8_0` → all layers q8_0 on GPU0 |
| C-lite. compute-aware split hint | implemented | single-GPU print suppressed; multi-GPU path built |
| D. dual-context server | backlog | needs 3060 + `--parallel 2` re-measure |
| E. boundary expert pre-replication | backlog | needs MoE model |
| F. hot-expert pinning | backlog | needs MoE model |
