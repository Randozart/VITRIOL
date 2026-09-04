# Panther Lake Intel Integration - SYCL/Vulkan Benchmark Report

**Date:** 2026-09-04
**Host:** Overlord-x8664 - Intel Core Ultra X7 358H (Panther Lake, 16c/16t, AVX2+AVX-VNNI, no AVX-512, no AMX exposed), Intel Arc B390 iGPU (Xe3, Mesa 26.2.1 ANV), 64 GB LPDDR5X shared, CachyOS, Linux 7.2.2
**Model:** `~/Downloads/Qwen3.8-27B-Q4_K_M.gguf` (unsloth, qwen35 hybrid arch, 27.32B params, 15.32 GiB, embedded MTP head blk.64)
**Builds:** upstream HEAD `0ef4d560e` (fork was 100 commits behind; hard-reset to upstream, no VITRIOL patches existed to preserve)

---

## 1. What was done

1. **Fork reset to upstream HEAD.** The `missing tensor blk.64.ssm_conv1d.weight` failure was an MTP-head handling bug fixed upstream (block 64 is the MTP head; `(i+1) % 4` recurrence marking wrongly flagged it recurrent). No VITRIOL-specific changes existed in the fork (no `llama.cpp-patches/`, no vitriol CUDA files), so `git checkout -B main FETCH_HEAD` was clean.
2. **System probes (read-only):** Mesa 26.2.1, `cooperativeMatrix=true`, `VK_KHR_shader_bfloat16` present, `minSubgroupSize=16`, `maxComputeWorkgroupSubgroups=32`, CPU governor was `powersave` (user set to `performance`), NPU driver 1.35.0 + compiler 2026.28 installed, oneAPI absent.
3. **Xe3 detection concern resolved:** ggml-vulkan keys warptiles off `vendor==INTEL && coopmat_support` (ggml-vulkan.cpp:4385), applies to Xe2 AND Xe3; the `maxComputeWorkgroupSubgroups==80` heuristic from upstream PR discussion is NOT in the detection path. No patch needed.
4. **Vulkan rebuilt** with `-march=native` (AVX2+AVX-VNNI for CPU fallback paths).
5. **oneAPI installed** (user): `intel-oneapi-dpcpp-cpp 2026.0.0_947`, `intel-oneapi-mkl 2026.0.0_908`, `intel-oneapi-mkl-sycl` (CachyOS package names; NOT the `*-runtime`/`*-devel` names used elsewhere).
6. **SYCL built** in `build-sycl/`: `icx`/`icpx`, `GGML_SYCL_F16=ON`, `-march=native`.
7. **Benchmarks** via `llama-bench -r 2` (shallow, p512/n64 unless noted).
8. **SYCL server smoke test PASSED:** health ok, correct generation ("Paris"), 12.67 pp / 5.39 tg server-mode at c4096.

## 2. Results (llama-bench, t/s)

| # | backend | config | pp512 | tg64 |
|---|---|---|---|---|
| 1 | Vulkan | ub512 (default) | 49.05 | 1.74 |
| 2 | Vulkan | ub2048 fa=auto | **135.85** | **4.26** |
| 3 | Vulkan | ub2048 fa=off | 100.17 | 3.43 |
| 4 | Vulkan | ub2048 fa=auto t16 | 114.96 | 3.29 |
| 5 | Vulkan | ub1024 fa=auto | 115.74 | 3.78 |
| 6 | Vulkan | ub2048 fa=auto mask 0xFF | 114.45 | 4.19 |
| 7 | Vulkan | ub2048 pp2048 | 91.46 | 3.24 |
| 8 | Vulkan | ub2048 pp4096 | 86.16 | 4.74 |
| 9 | Vulkan | ngl0 (CPU only) | 51.13 | 2.32 |
| 10 | **SYCL** | **ub2048 fa=auto f16 KV** | **233.58** | 5.02 |
| 11 | SYCL | ub2048 fa=auto pp2048 | 254.45 | 4.56 |
| 12 | SYCL | ub2048 fa=auto pp4096 | 202.02 | 4.50 |
| 13 | SYCL | ub2048 fa=off | 207.18 | 4.60 |
| 14 | SYCL | ub2048 fa=auto t16 | 189.93 | 4.49 |
| 15 | SYCL | ub2048 q8_0 KV | 233.07 | **5.23** |
| 16 | SYCL | ub4096 | 199.31 | 4.07 |

**Winner: SYCL, ub2048, fa=auto, q8_0 KV, t8 - pp512 233 t/s, tg64 5.23 t/s.**
vs Vulkan best: 1.72x pp, 1.23x tg. vs CPU-only: 4.6x pp, 2.3x tg.

## 3. Findings

- **SYCL > Vulkan on this host**, contradicting the Battlemage-dGPU community result (Vulkan after Mesa 26.1). Likely cause: SYCL's XMX/oneDNN/oneMKL paths vs ANV KHR-coopmat-only on Xe3 iGPU.
- **ubatch=2048 is the single biggest lever** (4.8x pp on Vulkan vs default 512). ub=4096 regresses.
- **16 threads regresses on both backends** (E-core interference on shared-memory bandwidth). t=8 optimal.
- **fa=off is WORSE here** (both backends), contradicting the Xe2-dGPU finding - the hybrid GatedDeltaNet layers behave differently.
- **q4_0 KV fails on qwen35 hybrid** on both backends (context creation error). q8_0 KV works on SYCL only, and is fastest (5.23 vs 5.02 tg).
- **P-core mask 0xFF barely helps** - Panther Lake X7 358H is 16c/16t; which cores are E-cores is unclear from cpuid; do not rely on masks.
- **CPU governor `powersave` -> `performance`** (user action via cpupower) done before all benchmarks.
- **xe driver exposes GPU frequency only via perf events** (`/sys/devices/xe_0000_00_02.0/events/gt-actual-frequency`); no manual freq control sysfs - firmware-managed, nothing to tune.
- **AMX not usable:** no `amx*` cpuinfo flags on this SKU and ggml AMX path requires `__AVX512VNNI__` anyway (Panther Lake has no AVX-512). Would need AVX2-width AMX kernel port (Phase 5, low priority given SYCL wins).
- **MTP head (blk.64) tensors are ignored at load** ("unused tensor" warnings) - expected; speculative decoding not wired on this build.
- SYCL fit warning: `failed to fit params to free device memory: n_gpu_layers already set by user to 99, abort` - benign on UMA (43 GiB free > 15.3 GiB model), `-fit` machinery is VRAM-oriented.

## 4. Deliverables

- `profiles/qwen38-intel-sycl/` - WINNER profile (canonical daily driver)
- `profiles/qwen38-intel-vulkan/` - retuned fallback (ub2048, f16 KV)
- `scripts/build-llama-server.sh` - SYCL section fixed: icx/icpx, GGML_SYCL_F16=ON, -march=native, correct pacman package names
- Builds: `build/` (Vulkan), `build-sycl/` (SYCL) both produce working `llama-server` + `llama-bench`

## 5. Launch command (certified working)

```bash
source /opt/intel/oneapi/setvars.sh
~/Projects/VITRIOL/llama.cpp/build-sycl/bin/llama-server \
  -m ~/Downloads/Qwen3.8-27B-Q4_K_M.gguf \
  -ngl 99 -c 4096 -ub 2048 -fa auto -ctk q8_0 -ctv q8_0 -t 8 \
  --host 127.0.0.1 --port 8080
```

## 6. Limitations / next steps

- Certified **shallow only** (p512/n64 + one server smoke test). No depth certification (cf. residency rule: window != depth).
- Context 32768 in profiles is memory-math-derived (17 attn layers x ~35 KiB/token q8_0 ~= 1.1 GiB at 32K), not depth-tested.
- NPU untouched (driver installed; no llama.cpp path - OpenVINO backend prototype is the only bridge, 22 ops).
- Next: depth test at 32K/128K, MTP/speculative check on SYCL, server-mode pp/tg at production context, consider NPU draft-model experiment.
