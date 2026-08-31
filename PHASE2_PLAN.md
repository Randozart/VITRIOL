# Phase 2 Plan — VITRIOL `vitriol-ku` branch completion

**Date:** 2026-08-31
**Status:** COMPLETE — TQ3 stack ported, committed c84097e10

## Current state

- `vitriol-ku` branch: upstream/master 9723942ad + VITRIOL ggml hooks + TQ3 stack
- llama-server running on port 8279, all unit flags adapted
- CUDA 41.17 tg (post-TQ3, matches pre-TQ3 42.08 within noise), Vulkan 42.85 tg
- `build → build-ku` symlink, `exclude_secondary = true`, `--checkpoint-min-step`
- TQ3 tests: test-tq3-cuda (11 pass), test-tq3-load-tiles (4 pass), test-tq3-prefill (pass)

## Remaining work

### Step 1: Verify server responds (2 min)
- `curl http://127.0.0.1:8279/health` — confirm 200
- `curl` a chat completion — confirm token generation

### Step 2: Fix GPU display bug (5 min)
- `scripts/vitriol` lines 1841, 2081: hardcoded "GTX 1070 Ti" → dynamic via nvidia-smi
- Cosmetic only, no functional impact

### Step 3: Port TQ3 stack (main work, ~2h)

Port from `vitriol` branch to `vitriol-ku`. 2100 lines CUDA + 800 lines CPU quant.

**Phase A — New kernel files (copy verbatim):**
1. `tq3-native.cuh` (104 lines) — block_tq3_0 device helpers, vec_dot
2. `tq3-native.cu` (32 lines) — native dot kernel + activation rotation
3. `tq3-prefill.cuh` (113 lines) — tiled prefill kernel
4. `turbo-wht.cu` (75 lines) — dynamic WHT op
5. `turbo-wht.cuh` (3 lines) — header

**Phase B — Block definitions:**
6. `ggml-common.h` — add `block_tq3_0`, `block_tq3_1s`, `block_tq3_4s`, `block_tq3_1s_shift` structs + `QK_TQ3_0 = 32`

**Phase C — Type/op IDs:**
7. `ggml/include/ggml.h` — `GGML_TYPE_TQ3_1S = 44`, `GGML_TYPE_TQ3_4S = 46`, `GGML_TYPE_TQ3_0 = 200`, `GGML_OP_TURBO_WHT`
8. `include/llama.h` — `LLAMA_FTYPE_MOSTLY_TQ3_0 = 200`, `LLAMA_FTYPE_MOSTLY_TQ3_1S = 43`, `LLAMA_FTYPE_MOSTLY_TQ3_4S = 45`

**Phase D — CUDA dispatch (highest risk, most divergence):**
9. `vecdotq.cuh` — add 7 vec_dot functions + VDR constants
10. `mmq.cuh` — add 3 `load_tiles_tq3_*` + mmq_type_traits specializations
11. `mmq.cu` — add switch cases + activation rotation
12. `mmvq.cu` — add switch cases + activation rotation
13. `common.cuh` — add type_traits for TQ3 types
14. `convert.cu` — add dequantize kernels + dispatch
15. `getrows.cu` — add get_rows kernels
16. `set-rows.cu` — add set_rows dispatch

**Phase E — ggml-cuda.cu integration:**
17. `ggml-cuda.cu` — add includes, TURBO_WHT dispatch, TQ3 mul_mat support, prefill debug

**Phase F — CPU quantization:**
18. `ggml-quants.h` — declare TQ3 quantize/dequantize functions
19. `ggml-quants.c` — implement ~800 lines (WHT, codebook, centroid lookup, scale search)

**Phase G — Model loading:**
20. `llama-quant.cpp` — FTYPE mapping + attention-V promotion
21. `llama-model-loader.cpp` — TQ3 AP bitmap handling
22. `tools/quantize/quantize.cpp` — register TQ3_4S ftype

### Step 4: Build and test (30 min)
- `cmake -B build-ku -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES=86 && cmake --build build-ku -j$(nproc)`
- Run `build-ku/bin/llama-bench` with TQ3 model if available
- Run `tests/test-tq3-cuda` if compiled

### Step 5: Commit vitriol-ku (5 min)
- Stage all changes, write commit message

## Risk assessment

- **Highest risk:** Phase D (CUDA dispatch) — upstream mmq.cuh/mmvq.cu diverged 4800+/485 lines. TQ3 additions are insertions at known switch-case points, but conflict resolution requires care.
- **Medium risk:** Phase F (CPU quant) — self-contained, no upstream divergence, but 800 lines of math.
- **Low risk:** Phase A-C, E-G — new files or well-isolated additions.

## Verification gates

1. Build succeeds with no errors
2. `llama-bench -m <tq3-model> -ngl 99` produces valid output
3. `llama-server` serves TQ3 model without crash
4. Byte-identical output vs vitriol branch build (differential gate)
