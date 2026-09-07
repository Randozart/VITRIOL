# Upstream Merge + DSV4_HC dual-implementation toggle (2026-09-07)

## Background

`git fetch upstream` on the inner llama.cpp fork surfaced 22 new ggml-org commits
since our last merge (`da13a30aa`). The load-bearing one for VITRIOL: upstream
shipped their own DeepSeek-V4 hyper-connection fused ops
(`7a333e724` — Vulkan DSV4_HC_COMB/PRE/POST), which collides with our earlier
Vulkan port of the same ops.

## Why upstream's DSV4_HC matters

- Runs the full 20-iteration Sinkhorn **in registers** via subgroup shuffles
  (16 lanes/token, 4 tokens per 64-lane subgroup); one dispatch replaces ~137
  strictly-ordered node executions per site.
- Upstream measured the unfused chain at ~32% of decode op time on Strix Halo.
- B390 (Arc PTL) exposes subgroup 16-32 with shuffle ops, so the kernel runs
  locally (verified via vulkaninfo).

## Decision

Adopt upstream's DSV4_HC as the **default** (canonical, will be maintained by
ggml-org). Keep our legacy flat-1D kernels as a selectable alternate so both can
be A/B'd on any hardware where a divergence is suspected — the "Arch Linux of
inference" principle.

## Merge mechanics (inner repo, commit `9585dd299`)

### Conflicts flagged by git

1. `ggml-cuda/mmq-config-{ampere,cdna,pascal-dp4a,pascal-older,rdna2,rdna3,rdna3-5,rdna4}.cuh`
   — our side added TQ3_0/TQ3_1S/TQ3_4S CASE blocks (TurboQuant);
   upstream changed the `ggml_cuda_mmq_config` constructor signature
   (`use_typical_moe_ncols`).
   **Resolution**: keep our TQ3 CASE blocks + upstream's signature (additive).

2. `vulkan-shaders/dsv4_hc_{pre,comb,post}.comp` (add/add) — both sides added
   same-path shaders.
   **Resolution**: upstream's .comp is canonical; our shaders kept as
   `dsv4_hc_{pre,comb,post}_vitriol.comp`.

### Silent collision NOT flagged by merge-tree

`ggml-vulkan.cpp` auto-merged but contained duplicate symbols both sides
defined: `pipeline_dsv4_hc_*_f32` members, `vk_op_dsv4_hc_*_push_constants`
structs, overloaded `ggml_vk_dsv4_hc_*` dispatch fns, duplicate switch cases in
`compute` and `supports_op`. merge-tree only flags textual conflicts, not
redefinition errors.

**Resolution**: keep upstream's implementation byte-identical; re-home ours:
- pipelines `pipeline_dsv4_hc_{pre,comb,post}_vitriol_f32`
- structs `vk_op_dsv4_hc_{pre,comb,post}_vitriol_push_constants`
- dispatch `ggml_vk_dsv4_hc_{pre,comb,post}_vitriol(...)`
- shader-gen: `dsv4_hc_{pre,comb,post}_vitriol_f32` -> `*_vitriol.comp`
- removed duplicate compute + supports_op case blocks; gated upstream's compute
  cases on `ctx->device->use_vitriol_hc`

### Toggle

`VITRIOL_VK_HC_IMPL` env var, read once in `ggml_vk_init`:
- unset / anything-but-`vitriol` → upstream implementation (default)
- `vitriol` → legacy flat kernels

Both pipeline sets are created unconditionally; `ggml_vk_op_get_pipeline` and
the compute switch branch on the flag.

### Bug found during verification

`ggml_vk_dispatch_pipeline` divides `elements[]` by `pipeline->wg_denoms[]`.
Our legacy dispatch calls passed `{ CEIL_DIV(nr, 256u), 1, 1 }` while the
pipeline's wg_denoms are `{256,1,1}` — a double division. Single-workgroup
cases (nr <= 256) passed by accident; any larger size silently dropped threads
(latent since the shaders were never validated — Vulkan OOM'd before ever
reaching the kernels on B390).
**Fix**: pass raw element counts (`{ nr, 1, 1 }` / `{ n_tokens, 1, 1 }`).

## Verification

- `build-vulkan` + `build-sycl` compile clean.
- `test-backend-ops -o "DSV4_HC_COMB,DSV4_HC_PRE,DSV4_HC_POST"`: **19/19 pass**
  with default (upstream) AND with `VITRIOL_VK_HC_IMPL=vitriol`.
- `test-backend-ops -o MUL_MAT`: 1119/1119 (no regression in merged dispatch).
- B390 subgroup 16-32 → upstream kernel functional on target hardware.

## Files changed (inner)

- `ggml/src/ggml-cuda/mmq-config-*.cuh` (8, conflict resolution)
- `ggml/src/ggml-vulkan/vulkan-shaders/dsv4_hc_{pre,comb,post}.comp` (upstream)
- `ggml/src/ggml-vulkan/vulkan-shaders/dsv4_hc_{pre,comb,post}_vitriol.comp` (new)
- `ggml/src/ggml-vulkan/vulkan-shaders/vulkan-shaders-gen.cpp` (register _vitriol)
- `ggml/src/ggml-vulkan/ggml-vulkan.cpp` (toggle + re-home + dispatch fix)
- plus the 22 upstream commits themselves

## Follow-ups

- A/B `VITRIOL_VK_HC_IMPL` on B390 with a real qwen4exp/Flash-Next Vulkan load
  once the Vulkan backend can host the model (currently blocked on Vulkan
  compute-buffer OOM for 74GB models — VITRIOL streaming is the unlock).
- Watch upstream for further DSV4_HC tuning (subgroup size heuristics).