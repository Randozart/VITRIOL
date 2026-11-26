# KINTSUGI → VITRIOL HANDOFF REPORT (2026-11-26 evening)

**Read this first.** You are picking up the Kintsugi-27B native-port work in `engine/`.
Everything below is measured, committed, and reproducible. The session that wrote this
burned three failed port attempts before getting here — do not repeat them; the
"Proven failures" section exists so you don't.

## 1. Mission and current scorecard

Owner objective: Kintsugi-27B running natively in `vitriol-server` at **or above** the
speed of the reference engine (`/scratch/kintsugi-ref/Kintsugi-27B/`, MIT, commit 8f12baacf).
Source tree for diffing: `~/kintsugi/engine-src/kintsugi-engine-8f12baacf/`.

| capability | their engine | ours (committed) | status |
|---|---|---|---|
| no-draft decode, c 65536-8192, ub 128-256 | 14.55 t/s | **14.0-14.15 t/s** | **PARITY** |
| DFlash2 n7 single-card repeat | 73.2 t/s (replay on) / 37.65 (4k trial) | **38.7-40.7 t/s** | functional; replay missing |
| 2-card | deadlocks on request | works (but slower: I10) | ours better |
| capacity | tiered KV | tiered KV (`kvc=`, our own port) | parity |

**What was our biggest win had nothing to do with their code**: an off-by-one in OUR fork —
`act_gpu_layers = min(n_gpu_layers, n_layer_real)` excluded the output layer, so the 1.35 GB
Q8_0 output head sat in **CPU_Mapped** and every decode paid PCIe for the largest matmul.
`n_layer_all + 1` (their formula) = +55 % decode instantly. Commit `3394a8d`.

## 2. What is committed (newest last)

| commit | content |
|---|---|
| `3394a8d` | HEAD2 two-stage output head + act_gpu_layers fix + launch-gate script |
| `38301ae` | stage 3a: GGML_CUDA_SHARED_POOL, VITRIOL_RS_VRAM_FRACTION (RS tier hook), LLAMA_RS_REPLAY sizing (stage-gated) |
| `036bc00` | stage 3b part 1: THEIR gated_delta_net.cu generation kernel (RAW template) + raw-gate graph plumbing |
| `48e0ed5` | `ggml_gated_delta_net_replay` constructor (inert) |
| `7e745e8` | fusion guard: gdn_cache_fusion skips replay ops (op_params[2] != 0) |
| `a2d478d` | replay op graph branch + pending plane + accessors — **CUDA IMA on first replay decode, gated OFF** |

Feature switches (all default-safe):
- `LLAMA_HEAD2=512` — two-stage head (required for parity numbers; frees ~600 MiB)
- `GGML_CUDA_SHARED_POOL=1` — one transient pool per device+stream (target+drafter overlap HWM)
- `VITRIOL_RS_VRAM_FRACTION=F` — tier the recurrent-state cache (F of each part in VRAM)
- `VITRIOL_KV_VRAM_CELLS=N` — tier the KV cache (our original tiered-KV knob, fingerprint `kvc=`)
- `LLAMA_RS_REPLAY=1` + `VITRIOL_RS_REPLAY_OP=1` — replay rollback layout+op (**BROKEN, see §4**)
- `GGML_GDN_RAW_GATES_DISABLE=1` — fall back to activated gates (verified byte-sane)
- `GGML_CUDA_HEAD2_CHECK=1` — two-stage vs exact compare in-process (correctness oracle)

## 3. Verified measurements (single RTX 3060 12 GB, clean card)

- HEAD2 off / on: 13.56-13.86 / 13.97-14.15 t/s (code+repeat prompts, 128 tok, 2 runs each)
- HEAD2 check mode: exact top-1 always inside stage-1 top-512, 0 differing candidate bits, 142 replays
- DFlash2 n7 (HEAD2+raw+pool+RS tier 0.05+KV tier 16384 cells, ub 128): **39.41/40.68/40.67**,
  acceptance 0.955, mean draft len 7.68; ub 256 with deeper KV tier: 38.98 (worse — host-KV reads
  cost more than the bigger ub gains)
- raw gates on vs off: identical golden text, 13.75 vs 13.71 t/s
- THEIR reference for the 73.2: ub 256, thinking off, LLAMA_RS_REPLAY=1+SHARED_POOL=1, acceptance
  1.000, mean len 7.89, 292 tok — **on this box it only works with CTX=65536/CELLS=65536/UB=256 and
  a clean card**; at CTX 8192 their own 32 GB reserve OOMs and they fall to CPU (0 % GPU). Do not
  treat a CPU-fallback reference run as valid.
- suites: GATED_DELTA_NET 40/40 3/3 backends; MUL_MAT 1288/1288; no-draft regression 14.11 post-port.

## 4. The one open defect: replay IMA

Enabling `LLAMA_RS_REPLAY=1 VITRIOL_RS_REPLAY_OP=1` gives a CUDA illegal-memory-access on the
first replay decode (~0.33 s in). Everything else in the replay chain works: constructor, graph
branch (`delta-net-base.cpp` replay path), pending plane alloc, accessors
(`get_pend_l/get_n_pend/s_pend_info/s_copy0`), fusion guard.

Debug notes for whoever picks this up:
- The fused-cache fusion guard was ONE candidate and is already fixed (`7e745e8`).
- The replay kernel (`gated_delta_net.cu`, n_pend = op_params[2] branch) reads `src_p->nb[1]` and
  asserts `== ne[0]` — our pend plane is `new_tensor_2d(F32, n_rs_seq*p_tok+1, mem_size)`,
  contiguous, should satisfy it. Check `pend_tok()` = `pend_k + pend_v + 2*pend_h` with
  `pend_k = ssm_d_state * ssm_n_group`, `pend_v = ssm_d_inner`, `pend_h = ssm_dt_rank`
  (their `llama-memory-recurrent.cpp:97-99`) matches what the KINTSUGI model actually sets
  (dump `hparams` at load and compare against their numbers for the same GGUF).
- First decode after load has an all-zero pending plane with valid count 0 — verify the kernel's
  zero-pending path (`p_info` fresh/rejected flags) doesn't index past the plane.
- Suspicion ranking: (1) `s_pinfo` values from `s_pend_info` (rs_z semantics may differ between
  forks), (2) the dst output region size (`pend_rows` computation in the replay ctor) vs how many
  rows the kernel actually writes, (3) the CUDA-graph capture interacting with the replay branch
  (their :3862 disables graphs for check-mode only — maybe replay needs the same).
- Reproduce: gate env from §2 + the `kintsugi-h2-df7` profile argv (below), crash in <1 s of decode.

## 5. Operator-facing artifacts (already in place)

- Profiles: `~/.vitriol/profiles/kintsugi-h2` (no-draft, 14.1 t/s) and `kintsugi-h2-df7`
  (DFlash2, 40.7 t/s). Required env is recorded in each `meta` file — the launcher reads
  config from the profile dir but env must be exported in the unit/shell.
- Launch gate: `scripts/engine-launch-gate.sh pre|post <log> <port>`. Preflight fails hard on any
  resident engine or non-desktop VRAM tenant; postlaunch fails on compute-buffer OOM/retry lines or
  a probe decode with <50 % GPU util (the "engine on CPU" trap cost this session two bad
  measurement windows).
- The live 27B BuffedMod driver (`qwen38-buffedmod-262k-cache8-mtp2`) was DOWN the whole session per
  owner instruction. Restoring it is `systemctl start vitriol-server.service` (profiles unchanged);
  it will NOT pick up the new engine features until a restart on the new build — that restart is
  fine now, the build is committed and regression-verified.

## 6. Proven failures — do NOT repeat

1. **Partial adoption of their kernel** (11-25): importing `gated_delta_net.cu` alone kept our old
   graph; result: correct but slower (their file also silently DELETED our chunked kernel for
   n_tokens ∈ {2,3,4,6,8} — `gdn_scan_*`, which theirs lacks entirely).
2. **Raw gates via spot-patches** (11-25): plumbing was textually identical to theirs and still
   produced garbage — because the old kernel generation didn't implement RAW correctly with our
   fused-cache interplay. Fixed by taking their real generation kernel (11-26, `036bc00`).
3. **Their launcher at CTX 8192 as oracle**: reserve OOM → CPU fallback → numbers are meaningless.
   Only the CTX 65536/UB 256/CELLS 65536 shape reproduces their published numbers here.
4. **`pkill -9` on vitriol-server** while `Restart=always`: systemd restarts it mid-window and the
   next load OOMs. `systemctl stop` first, always.

## 7. Rules that bit this session (from AGENTS.md, re-learned the hard way)

- Engine patches: python3 exact-match scripts with `count == 1` asserts (template `/tmp/d1_patch.py`).
  The edit tool rewrites whole files — 9k-line noise on a frozen fork.
- New `.cu` files need `cmake -B build` reconfigure (sources are GLOBbed) — linking fails with
  undefined references otherwise, and the error looks like a missing port, not a stale build.
- `build` symlinks to `/scratch/builds/build-ku-cu12-native`. Target `llama-server`, binary
  `build/bin/vitriol-server`.
- Their files are CRLF; git converts on commit — harmless, but expect the warnings.
- One engine lifetime = many measurements. Load once, run the whole arm matrix, kill once.
  Health "ok" is NOT "ready": require the launch gate's GPU-util probe.

## 8. What "done" means

- [x] Kintsugi native in vitriol-server, single card, coherent output
- [x] No-draft parity (14.0+ vs 14.55)
- [x] DFlash2 functional (40.7, above their 4k trial)
- [ ] DFlash2 at/above their 73.2 — blocked ONLY on the replay IMA (§4)
- [ ] Owner decision: make a Kintsugi profile the daily driver (replaces `qwen38-buffedmod-*`)
      — requires the engine restarted on the new build and Officina compatibility check
      (`alias = Lapis Occultus` is already set in both profiles)

Plans on disk (chronological): `gdn-port-plan-2026-11-25.md`, `gdn-raw-gate-research/-reconciliation-2026-11-26.md`,
`kintsugi-parity-research-2026-11-26.md`, `kintsugi-what-works-research-2026-11-26.md`,
`kintsugi-parity-plan-2026-11-26.md` (the master plan, with executed log), this file.
