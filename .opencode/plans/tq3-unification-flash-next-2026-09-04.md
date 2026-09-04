# TQ3 unification + Flash-Next roster update - 2026-09-04 (session 2)

Follows: panther-lake-sycl-bench-2026-09-04.md. All work pushed; both
rigs' lines unified.

## 1. Fork reconciliation (the big one)

- Laptop fork was 100 commits stale with NO VITRIOL patches (correction:
  earlier report's "no patches existed" was wrong - they lived on
  origin/main, not in the stale local checkout).
- origin/main carried c84097e10 (vitriol-ku port) which NEVER compiled
  standalone: TQ3_1S/4S/0 enum values 44/46/200 vs GGML_TYPE_COUNT=43
  on its own base, and no trait rows. Desktop's working TQ3 was
  unpushed local state (vendor-patch-rule violation, again).
- Desktop independently hit the same clobber and pushed their restore
  (adeaecc09) plus newer upstream (be789c344) while we worked.

## 2. TQ3 completion (validated end-to-end on Arc B390)

- ggml.c trait rows for TQ3_{0,1s,4s}; GGML_TYPE_COUNT 43->201.
- CPU vec_dots ggml_vec_dot_tq3_{0,1s,4s}_q8_0: SCALAR-exact via
  dequant-then-dot. CRITICAL PAIRING: q8_0 (32-block), NOT q8_K
  (256-block) - head_dim<256 breaks q8_K superblocks. Both rigs
  converged on this independently.
- llama-kv-cache: attn_rot gate for TQ3 (the type's own randomized
  Hadamard must not stack with upstream's K/V rotation). This was the
  silent garbage-output cause, NOT the fp16 macro.
- fp16-macro "bug" was a test-harness artifact (table initialized in
  ggml_cpu_init, absent from raw dlopen probes). No real icx issue.
- Validation: unit kernel-vs-reference exact; quantizer roundtrip
  18.2% = 3-bit Lloyd-Max theory; server tq3_1s coherent, tq3_0
  degraded on 0.6B (bitrate physics, monotonic f16>tq3_1s>tq3_0);
  SYCL/Vulkan/CPU paths all run. Vulkan tq3 GPU-hosted KV aborts
  (no kernels) - route via -nkvo; clean-error hygiene pending.

## 3. Model roster (this rig)

Deleted: Qwen3.8-27B-Q4_K_M (owner call).
Downloaded+verified: GLM-4.7-Flash Q4_K (17G), gpt-oss-20b MXFP4 (12G),
eagle3-gpt-oss-20b Q8_0 draft (879M), Qwen3-Next-80B IQ4_XS (40G).
In flight: Qwen3.8-Flash-Next UD-Q2_K_XL (79G, ~200B-A5B, qwen4exp).
Dropped: GLM-5.3-Flash UD-IQ2_XXS (126G; Flash-Next wins streaming
economics - smaller cold tier, ~half active bandwidth).

Qwen3.8-Flash-Next notes: qwen4exp arch SUPPORTED in build; MTP ships
as separate adapter, NOT wired in llama.cpp; license other (Qwen).

## 4. Pushed

- inner 009915688..058ff4df9 (merge + dedupe, TQ3 unified)
- outer b8ac173..7d088fd (rebased onto desktop's latest, submodule
  bumped; fetch-hf skip-verify fix; manifest rev2)

## Next

1. Sweep when Flash-Next lands: GLM-4.7-Flash, gpt-oss-20b(+eagle3
   speculative!), Qwen3-Next-80B IQ4_XS, Flash-Next UD-Q2_K_XL.
2. Concurrency parallel 2/4 on the winner (kiosk serving case).
3. Vulkan tq3 GPU-hosted KV: clean error instead of abort.
4. Flash-Next MTP adapter investigation (separate GGUF, unwired).
