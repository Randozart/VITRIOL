# E1 — Upstream Sync Port: Results Report

Date: 2026-09-01 (runs 15:26–17:45 local)
Plan: `mining-experiment-master-plan-2026-09-01.md`
Raw bench data: `.opencode/plans/bench-e1-2026-09-01/`

---

## 1. What was merged

Inner `llama.cpp` `main`: upstream `upstream/master` merged at tip
`be789c344` (2026-09-01). Merge commit **`04a4f5f12`**, zero conflicts
(auto-merge clean; TQ3/TurboQuant + `vitriol-*` files untouched).

A-list commits verified in tree after merge:

| PR | verification |
|---|---|
| #28011 seq-scan early stop | `src/llama-kv-cells.h:336` `left > 0` loop condition |
| #27991 KV restore batching | `src/llama-kv-cache.cpp:2533-2591` contiguous-run scatter batching |
| #27621 MoE fusion→specdec/multi-token | `41ef91f7c` in merge range |
| #27978 mm_ids_helper fast path | `ggml/src/ggml-cuda/mmid.cu:31-52` templated `n_expert_used` |
| #28123 qwen4exp state rollback | in merge range |
| #28159 n_layer_nextn load order | `src/llama-model.cpp:1226` loads before `n_layer()` use |
| #25635 FA K/V XOR-swizzle | `fattn-swizzle.cuh` + `fattn-mma-f16.cuh:402,442,583` |
| #27837/#27969 `--lazy-mode -lzm` | in `llama-bench`/`llama-cli` help output |

Build: `build-ku2/`, `GGML_CUDA=ON GGML_VULKAN=ON`, arch `61;86`,
nvcc **12.9** (`~/toolkits/cuda-12.9`), host compiler forced
`-DCMAKE_CUDA_HOST_COMPILER=/usr/bin/g++-14` (system g++-15/16 headers break
nvcc 12.9: `type_traits(555): error: type name is not allowed`). ccache wired
via `*_COMPILER_LAUNCHER` (first fill 171 misses; future rebuilds cached).

NOTE (toolchain archaeology): `build-ku/` (OurobourOS-era control) was
86-only @ nvcc 13.3; the daily-driver `build/` is 61;86 @ nvcc 12.9 and is
the E1 control. CUDA 13.3 at `/opt/cuda` cannot target sm_61 (compute_61
removed) — CUDA 12.9 sidecar remains mandatory for dual-GPU builds.

## 2. Environment / fingerprint

- GPUs: dev0 RTX 3060 12 GiB (sm_86), dev1 GTX 1070 Ti 8 GiB (sm_61); driver 580.178.04
- Models: `Qwen3.8-9B-Q6_K.gguf` (7.03 GiB), `Qwen3.8-9B-Q8_0.gguf` (9.10 GiB),
  `Qwen3.8-27B-Q3_K_M.gguf` (12.86 GiB, 27.32 B params, qwen35 arch)
- Protocol: `killall -9 llama-server` before each launch; daily-driver unit
  stopped for bench windows (restarted 17:45 on old binary — swap pending user);
  CUDA device pinning via `-dev` + `CUDA_VISIBLE_DEVICES`; `-r 2` (depth
  confirmation `-r 3`).

## 3. Results

### 3.1 Shallow bench, 9B, CUDA0 pinned (H0, H1c, H1d)

pp512 / tg128, t/s, ± is run spread over 2 reps:

| config | control `590a4bb09` (`build/`) | candidate `04a4f5f12` (`build-ku2/`) | Δ |
|---|---|---|---|
| Q6_K fa=off pp | 1504.41 ± 47.0 | 1518.15 ± 53.6 | +0.9% |
| Q6_K fa=off tg | 41.89 ± 0.07 | 42.07 ± 0.04 | +0.4% |
| Q6_K fa=on pp | 1526.68 ± 20.0 | 1546.86 ± 30.0 | +1.3% |
| Q6_K fa=on tg | 42.07 ± 0.04 | 42.24 ± 0.07 | +0.4% |
| Q8_0 fa=off pp | 1721.42 ± 78.5 | 1721.66 ± 89.3 | +0.0% |
| Q8_0 fa=off tg | 36.31 ± 0.07 | 36.36 ± 0.03 | +0.1% |
| Q8_0 fa=on pp | 1763.13 ± 46.6 | 1781.64 ± 46.4 | +1.1% |
| Q8_0 fa=on tg | 36.50 ± 0.06 | 36.55 ± 0.03 | +0.1% |

Control reproduces the frozen §3 baselines (OurobourOS §16.3d) within 0.5%
(1504/41.89 vs 1509/42.08; 1721/36.31 vs 1734/36.36) — control valid.

**H0 (no regression): PASS.** Everything within noise; small consistent
positive drift (+0.4–1.3%) in fa=on rows.

First control attempt (no `-dev` pin) accidentally ran a CUDA+Vulkan mix:
Q6_K tg 34.89 vs 41.89 pinned — **18% degradation from backend mixing**.
Operational warning recorded: always pin `-dev` on this dual-backend build.

### 3.2 Depth-filled, 27B Q3_K_M, ts 22/14, q4_0 KV, ub64, fa=on (H1a)

`-d` = cache pre-filled with dummy tokens before the measured pp/tg:

| test | control | candidate | Δ |
|---|---|---|---|
| pp512 d0 | 299.06 | 299.20 | +0.05% |
| tg128 d0 | 12.88 | 12.88 | 0.0% |
| pp512 @ d43000 | 162.59 | 163.20 | +0.4% |
| tg128 @ d43000 | 7.59 | 7.57 | −0.3% |
| pp512 @ d54000 | 146.90 (147.58 n=3) | 146.54 (147.82 n=3) | +0.2% |
| tg128 @ d54000 | 6.81 (6.80 n=3) | 6.72 (6.80 n=3) | 0.0% confirmed |

**H1a (deep-ctx decode gain): NULL on this workload.** The upstream
56→74 t/s win does not reproduce in single-sequence llama-bench at 43k/54k
filled. Mechanism (from #28011 diff): the seq-scan early-stop saves work
proportional to sequences-per-cell; a single-sequence cache scans one bit
per cell either way. The win should appear in **multi-slot / multi-sequence
server conditions** (e.g. parallel ≥ 2 with distinct prompts per slot) —
not tested here; daily driver now runs `--parallel 1`, so expected real-world
impact ≈ 0 at the current operating point. Recorded as "condition-dependent;
re-test if parallel operation returns".

Depth tie at 54k confirmed with `-r 3` after a −1.3% first read (run
variance, not regression).

### 3.3 Slot save/restore wall-time, server-level (H1b)

Procedure: llama-server `-c 32768 --parallel 2 --slot-save-path …`, 10,801
-token prompt prefilled into slot 1 (~42 s pp @ ~260 t/s), `slots/1?save`,
dirty with a disjoint 2,078-token prompt (forces cell displacement), then
`slots/1?restore`, wall time via curl:

| build | save | restore (disjoint-dirty) | save file size |
|---|---|---|---|
| control `590a4bb09` | 0.232 s | **0.176 s** | 356,281,248 B |
| candidate `04a4f5f12` | 0.249 s | **0.189 s** | 356,281,248 B |

Earlier contiguous-dirty candidate restore measured 0.098 s (LCP reuse case).

**H1b (≥10× restore win): NOT REPRODUCIBLE on this lineage — pass-neutral.**
Both builds restore a 10.8k-token state in <0.25 s. Upstream's 25–63 s
pathology (1.4M copies) is not present in the vitriol-ku lineage at this
scale; either our fork's server-context/checkpoint path already avoided it
or the upstream case requires much larger/more fragmented states (55k+ ctx,
many slots, context shifts). No regression; #27991 batching rides along as
insurance. **Observation to re-visit:** an earlier candidate save under
partial LCP reuse wrote 168 MB vs 356 MB cold — save size varies with cache
reuse state; not investigated further this session.

### 3.4 FA XOR-swizzle sm_61 applicability (H1d)

Static read: swizzle lives in the fattn-**mma** path
(`fattn-mma-f16.cuh` includes `fattn-swizzle.cuh`; mma = tensor-core path).
sm_61 has no tensor-core mma → Pascal FA uses the tile kernels; the swizzle
code is compiled but not exercised on the 1070 Ti. Expected effect confined
to sm_70+ (here: 3060). Bench shows fa=on rows +0.4–1.3% on the 3060 —
within noise; no dedicated fa-only kernel benchmark was run. Verdict:
**implemented, no measurable end-to-end delta at our shapes; sm_61 null by
architecture.**

### 3.5 `--lazy-mode -lzm` (E6, bundled)

Qwen3.8-9B Q6_K, CUDA0, fa on (n=2): off 1515.91/42.12, on 1526.17/42.21
(pp/tg) — **parity**. Load-time A/B (`-st -n 1`, wall): first pair
off=15.3 s / on=4.9 s was order-biased (cold start); reversed pair
on=4.9 s / off=4.7 s → **warm-cache load parity**. Lazy mode is neutral for
fully-resident models; adopted as available tooling for future partial-
residency (streaming) experiments. Flag also present in llama-bench
(`-lzm on|auto|off`).

### 3.6 CLI flag renames worth knowing (upstream drift)

- `-fa` now takes `on|off|auto` (was `0|1`)
- `-dev` names are uppercase `CUDA0`/`CUDA1` on the new build (old build: `cuda0`)
- `-ts` separator is `/` on BOTH builds (`,` = multi-value separator; `-ts 22,14` in llama-bench silently parses as two values — always `22/14` here)
- `--no-cnv` → gone; use `-st` (`--single-turn`)
- new llama-bench: `-fitt/-fitc` fit-to-device options, `--no-warmup`, `-oe` stderr output

## 4. Hypothesis scoreboard

| hypothesis | verdict |
|---|---|
| H0 no-regression | **PASS** |
| H1a deep-ctx decode gain | **NULL** (single-seq; condition-dependent — multi-seq re-test queued) |
| H1b restore ≥10× | **NOT REPRODUCIBLE** (both <0.25 s; lineage never had the slow path) |
| H1c MoE fusion gain | inconclusive at shallow bench (9B dense-ish workload within noise); MoE-specific pp/tg test queued against Qwen3.6-35B |
| H1d FA swizzle | no end-to-end delta; sm_61 null by arch (mma-gated) |
| H1e MTP fixes | correctness-only; not exercised (MTP off in profiles) — merged as insurance |
| H1f/E6 lazy-mode | neutral; adopted |

## 5. Decision

- Merge **stands** (`04a4f5f12` = new main): zero regressions, several
  correctness fixes (MTP load order, qwen4exp pile, KV guards), future-facing
  infra (lazy mode, restore batching, MoE fusion).
- `build-ku2/` is the reference candidate build. **Daily-driver swap NOT yet
  applied** — daily server still runs old `build/` binary (restarted 17:45).
  Swap = repoint `vitriol-server.service` or rebuild `build/` from new main;
  recommend a short server A/B (prompt-cache hit behavior + slots) before
  swapping — deferred to user's call.
- H1a/H1b nulls are honest results and recorded in EXPERIMENT_LOG.

## 6. Raw data index

- `bench-e1-2026-09-01/control-590a4bb09-9b{Q6_K,Q8_0}-cudaOnly.md`
- `bench-e1-2026-09-01/candidate-04a4f5f12-9b{Q6_K,Q8_0}-cudaOnly.md`
- `bench-e1-2026-09-01/depth-{control,candidate}-*-27B-q40kv.md`
- `bench-e1-2026-09-01/e6-lazymode-9bQ6K.md`
- server logs: `/tmp/opencode/server-h1b-{old,new,new2}.log`
- slot state files: `/tmp/opencode/slots/s1-{old,new,new2}.bin`
