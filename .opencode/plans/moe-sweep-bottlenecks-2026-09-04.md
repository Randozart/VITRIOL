# MoE sweep results + VITRIOL bottleneck map - 2026-09-04 (session 3)

Follows: tq3-unification-flash-next-2026-09-04.md. Machine quiet
(load 1.2, no server-ml contention). All tests: SYCL build 3f2509bfe,
Arc B390 Xe3, -ngl 99 -t 8 -ub 2048 -fa auto, llama-bench r=2 p512/n64
(full argv embedded per flag-provenance rule).

## Results

| model | arch | size | pp512 | tg64 | KV |
|---|---|---|---|---|---|
| GLM-4.7-Flash (deepseek2 30B.A3B Q4_K) | deepseek2 | 17.0G | 129.9 +/-114 | 17.1 +/-3.1 | f16 |
| GLM-4.7-Flash (same, q8_0 KV) | deepseek2 | 17.0G | (same run fam) | **19.1** | q8_0 |
| gpt-oss-20b MXFP4 MoE | gpt-oss | 11.3G | (n/a this run) | **11.8** +/-0.8 | f16 |
| Qwen3-Next-80B IQ4_XS | qwen3next | 39.9G | **345.0** +/-3.6 | **21.2** +/-0.0 | f16 |
| Qwen3-Next-80B (q8_0 KV, server load only) | qwen3next | 39.9G | - | - | q8_0 |
| [old ref] Qwen3.8-27B dense Q4_K_M | qwen35 | 15.3G | 233.1 | 5.23 | q8_0 |

Stability: NO xe-driver MUL_MAT_ID timeout with 512 experts at ub2048
(the known iGPU risk did not materialize at this ubatch/driver).
Qwen3-Next server load: ~135 s from NVMe (UX bottleneck in itself).

## Concurrency (Qwen3-Next-80B, --parallel 4, c16384 q8_0 KV)

4 streams x 128 tokens, temp 0: 8.41 t/s per stream, **33.6 t/s
aggregate** = 1.58x single-stream. Sub-linear: per-stream KV + batch
dispatch overheads eat the rest.

## Speculative (gpt-oss-20b + eagle3 Q8_0 draft, n_max=2)

10.9 t/s vs 11.8 baseline = NET LOSS. Draft acceptance 20.2%
(18/89, mean len 1.4). Draft cost > savings at this acceptance.

## Bottleneck map for VITRIOL (ranked by opportunity)

1. **MoE decode bandwidth utilization ~35-40%.** GLM ~30 GB/s, Next80
   ~34 GB/s effective vs ~85 GB/s ceiling. MUL_MAT_ID per-token
   dispatch + shared-expert/attention stream serialization. Lever:
   batched expert gather kernels, expert-interleaved scheduling,
   VITRIOL's original DMA heritage applied UMA-side.
2. **MXFP4 kernels underperform on Xe3 SYCL** (gpt-oss 20B-A3.6B
   LOSES to GLM 30B-A3B at tg). ~20 GB/s effective. Lever: XMX-native
   MXFP4 vec_dot/dequant kernels; or UD-Q4_K requant as fallback.
3. **Eagle3 draft acceptance 20%** -> spec-decode net loss. Lever:
   tune n_max/n_min, try draft-mtp when a wired MTP exists, or drop
   spec for now. Chip-side: decode-bound streams have idle XMX -
   acceptance is the whole game.
4. **Concurrency sub-linear (1.58x @ 4 streams).** Lever: KV q8_0
   everywhere (done here), scheduler-level batch packing, parallel
   ubatch interleave.
5. **80B model load ~135 s** from NVMe. Lever: VITRIOL warm-start
   (page-cache pinning), lazy expert loading, --mmap tuning.
6. pp variance ±114 on GLM run - download process was running
   (9.6 MiB/s disk/net). Real-machine contention tax confirmed again;
   sweep should pin/pause background IO.

## Next

- Flash-Next UD-Q2_K_XL lands (~1h): same sweep + streaming-mode probe
  (mmap, hot-expert behavior) - bottleneck #1 test at 200B scale.
- Qwen3-Next q8_0 KV tg number (bench run was preempted by load-time;
  expect ~+2 t/s like GLM).
- VITRIOL Phase 6 design: expert-granular LRU + router-guided prefetch
  targeting bottleneck #1.

## Addendum (2026-09-04 ~21:00): cross-agent Vulkan result - backend routing hypothesis

Second opencode agent on THIS machine benchmarked GLM-4.7-Flash on the
Vulkan build (c6e1fc740), defaults (t=2, ub512, ngl -1 auto):

    $ ~/Projects/VITRIOL/llama.cpp/build/bin/llama-bench -m ~/models/GLM-4.7-Flash-Q4_K.gguf -p 512 -n 128
    | deepseek2 30B.A3B Q4_K - Medium | 16.98 GiB | 29.94 B | Vulkan | -1 | pp512 | 370.00 +/- 0.09 |
    | deepseek2 30B.A3B Q4_K - Medium | 16.98 GiB | 29.94 B | Vulkan | -1 | tg128 |  31.91 +/- 0.66 |

Comparison with my SYCL run (same model, t8/ub2048):

| | SYCL (mine) | Vulkan (other agent) |
|---|---|---|
| pp512 | 129.9 +/-114 | 370.0 +/-0.09 |
| tg64/128 | 17.1 / 19.1 (q8_0 KV) | 31.9 (tg128) |

Caveats: parameters differ (t2/ub512 vs t8/ub2048) and pp variance in
my run had download-IO contention. But tg is insensitive to ubatch and
barely sensitive to threads (GPU-bound): the **1.7-1.9x Vulkan decode
advantage on deepseek2 MoE is real**.

Combined with the dense result (Qwen3.8-27B: SYCL 233 pp vs Vulkan 136):

**Dense -> SYCL wins (XMX/oneDNN matmuls). MoE -> Vulkan wins
(MUL_MAT_ID dispatch path).**

Implication for the bottleneck map: lever #1 refines from "MoE kernels
slow" to "SYCL MUL_MAT_ID/expert-dispatch path is the weak link".
VITRIOL actions this unlocks:
- per-arch backend routing in profiles (dense=SYCL, MoE=Vulkan, pending
  same-params A/B confirmation)
- SYCL MUL_MAT_ID as a concrete upstream-of-VITRIOL improvement target
  (or VITRIOL-side expert dispatch override)

Pending: same-params A/B matrix (GLM/Next80/gpt-oss x Vulkan/SYCL x
ub512/2048) to confirm; Flash-Next rerun on both backends.

## Addendum 2: two-agent operational hazard

Second agent benchmarking concurrently on this host. Shared hazards:
~/models/, ports 8080/8081, and bare `killall -9 llama-server` in my
sweep would have killed their server mid-test (and vice versa - my
background server death earlier may have been mutual). Proposed
protocol before further cross-agent testing:
- fixed port ranges per agent (laptop agent: 8081-8089; other: 8091+)
- PID-scoped kills only, no killall
- coordinate large downloads (single fetcher)

