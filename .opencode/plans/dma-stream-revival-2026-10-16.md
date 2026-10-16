# DMA streaming revival: wiring fixes, mmap integration, C++/Rust A/B

**Created**: 2026-10-16 18:05
**Status**: approved (user, 2026-10-16)
**Goal**: make the three-tier expert streaming architecture (VRAM LRU / pinned RAM / Optane+XFS)
actually engage for Nemotron 3.5 Super VL 120B, prove engagement with counters, and A/B the
C++ and Rust hot paths. Gate: architecture-proven bar (per user) - correctness oracle first,
then t/s scaling with hot-tier residency; legacy >=2 t/s secondary.

## Evidence base (this session + September campaign)

- Sept 25 Heisenbug campaign (`.opencode/plans/vitriol-stream-rust-hybrid-2026-09-25.md`):
  streaming corrupted outputs at async graph depth ~1272 unsynced nodes; transport byte-verified
  clean (1794/1794 slices); Rust port (`vitriol-stream/`, 1223 lines) REPRODUCED the corruption
  exactly -> both implementations exonerated, ggml async graph execution implicated.
  Campaign shelved at the 27B retarget. This is "broke for unknown reasons".
- Today: model-load fixes landed (inner `f4518576a` rms-eps fallback, `90cb995a1` MTP latent MoE);
  CPU-MoE baseline 0.70-0.83 t/s; gate fail. DMA store tier wired (launcher keys
  `store/store_path/store_source/max_locked_mb`, 38 GiB store mkfs'd, profile
  `nemotron35-super-vl-store`), decode 0.73-0.82 t/s == CPU-MoE (streaming tiers starved).
- User-instigated deep research found two wiring defects (below) that fully explain
  "silently broke" and "nothing happening".

## Root causes

- **RC1 (launcher)**: five blank lines inside env-chain blocks
  (scripts/vitriol 2290, 2668, 2746, 2818, 2891) from profile-key insertion surgery.
  `env` executes alone (journal env-dump), `$ENV_PREDICTIVE_PREFETCH` runs as a command
  ("line 2747: VITRIOL_PREDICTIVE_PREFETCH=1: command not found"), shell exits 0,
  bogus PID banner, systemd restart loop (52+ cycles). bash -n cannot catch it.
- **RC2 (engine)**: the designed zero-copy mmap integration is dead code.
  llama-model.cpp:1800-1834 queries proc addresses `vitriol_buft_supports_host_ptr` /
  `vitriol_buffer_from_host_ptr`; no backend registers them. With `mmap=on` the wrap path
  never engages, experts fall into the plain mmap host buffer, `vitriol_is_vitriol_buffer_type`
  is false at compute, and the entire architecture disarms silently. Any mmap=on config
  (including our profiles) was a silent off-switch.

## Stages

### Stage 1 - launcher repair + anti-silent-break guard (outer commit)
1. `systemctl stop vitriol-server.service` (crash-looping since ~16:30).
2. Delete the 5 blank lines; extended scanner walks ALL env-chain openers
   (`env \`, `exec env \`, `nohup ... \` forms) asserting zero mid-chain blanks and
   no missing continuations; `bash -n`.
3. Engagement guard: when config requests `vitriol.mode=stream`, after health the launcher
   checks GEN_LOG for the stream/lazy/store init line and prints a loud WARNING if absent.
4. Acceptance: scanner clean, unit reaches health, fingerprint line in GEN_LOG.

### Stage 2 - expose the mmap-wrap procs (inner llama.cpp commit)
1. `vitriol_buft_supports_host_ptr(buft)` -> true when `g_vitriol_config.mode == STREAM`.
2. `vitriol_buffer_from_host_ptr(dev, ptr, size, max_tensor_size)` -> wrap region:
   cudaHostRegister(ptr, size) + ggml_backend_buffer over the vitriol buft (same buft
   instance as extra_bufts so the compute gate matches); free = unregister, never munmap
   (we do not own the mapping). Same registration pattern as the existing disk-offload branch.
3. Register both in ggml-cuda.cu `get_proc_address`.
4. Acceptance: build green; with `mmap=on` the load logs the wrap path and post-load
   server RSS stays small (zero-copy) while the compute gate evaluates true.

### Stage 3 - verification protocol (no config churn beyond profile select)
1. Profile `nemotron35-super-vl-ramshot`: mmap=on, mode=stream, ncmoe=88 (placement flag),
   lru_mb_gpu=3400,4800, max_locked_mb=16384, predictive_prefetch=on, store off.
2. ONE restart -> health -> engagement lines in GEN_LOG.
3. Warmup request -> sample `nvidia-smi` (pools) + `/proc/<pid>/status` VmLck (pin tier,
   first run where LimitMEMLOCK=infinity is live).
4. Correctness oracle BEFORE perf: fixed factual prompts (17*23=391 class); any incoherent
   or wrong output = corruption signal -> immediately test `VITRIOL_MATMUL_SYNC=1` arm
   (September heal protocol) before any perf claim.
5. 3x256-token bench (lighthouse prompt, temp 0.7) -> clean `systemctl stop` ->
   atexit report card from GEN_LOG: LRU pools/slots, Hits/Misses/HitRate/Evictions,
   Pinned Memory, Predictor.
6. Rust A/B: same protocol with `VITRIOL_RUST=1` (static libvitriol_stream.a link).
   Closes the September campaign with a live comparison.
7. Fallback arm if wrap path misbehaves: profile with mmap=off (real-buffer variant,
   36 GiB host copy, zram-backed cold tier), same protocol.

### Stage 4 - docs (outer commit)
Addendum B in `nemotron35-super-vl-bringup-2026-10-16.md`: both root causes with file:line,
causal chain, report card numbers, budget math, verdict. Update AGENTS.md DMA section if
engagement confirmed (with the anti-silent-break guard noted).

## Verdict matrix

| outcome | meaning | next |
|---|---|---|
| oracle fail | Heisenbug resurfaced via engaged hooks | MATMUL_SYNC arm; ggml scheduler workstream |
| hits > 0, oracle pass | architecture live | tune budgets; judge t/s vs residency scaling |
| hits == 0, oracle pass | hook still bypassed | debug route gate with counters as evidence |
| wrap path broken | Stage 2 defect | fix before anything else |

## Risks

- Heisenbug resurfacing is a REAL possibility (that is what Stage 3.4 is for); the
  27B daily driver is untouched by this work (separate profile).
- Host RAM: 16 GiB pin + page cache + engine ~6 GiB in 31 GiB; zram absorbs; engine is
  oom_score_adj -500 (last victim). pin 16384 chosen by user.
- dev0 carries desktop tenants (~1.5 GiB); pools budgeted around it (3400/4800).
- sterling-4b server (port 8124) is another project's tenant - do not kill.
- Toolchain: Rust arm needs only stable Rust + vendored cuda-rust driver bindings
  (no rustc-codegen-cuda; no kernels). cuda-oxide kernel verdict unaffected.

## Provenance

No new third-party code. All work on fork-owned files (Apache-2.0 OR MIT).

## Progress log

- **2026-10-16 19:40 - Stage 1 DONE** (outer `758ce99`): 5 blank lines deleted,
  scanner clean across 9 env-chain blocks, engagement guard installed. The
  52-cycle crash loop stopped.
- **2026-10-16 19:50 - Stage 2 DONE** (inner `3d055419a` + follow-up `bb8c17943`):
  wrap procs registered and working - `VITRIOL: wrapped mmap region (40573 MiB)`
  binds the expert pages into the vitriol buft. Follow-up fixes found by the
  first engaged runs:
  1. eager wrap cudaHostRegister pins the whole mapping -> removed (lazy pin
     tier registers slices on demand; note cudaHostRegister EINVALs on mmap
     pages, so the mmap arm's pin tier is mlock-only - residency without
     DMA registration; H2D stages through the driver at reduced rate)
  2. THREE ungated `g_store_source` checks (source_device(), pool-init
     bounce ring, mul_mat_id fill) routed non-store runs into store reads
     against no store - now all require `store_backed`
  3. fit pass executes graphs through 0-size dummy vitriol buffers -> route
     gate now requires buffer size > 0
- **2026-10-16 19:55 - ENGAGEMENT PROVEN, launch defect isolated.** With the
  wrap live: route gate fires on real decode, LRU pool allocates (3400 MB,
  6631 slots, GPU 0), per-expert H2D fills and hit-remaps execute. First
  remapped decode aborts in ggml's launch path (`quantize_row_q8_1_cuda`
  EINVAL inside `mul_mat_vec_q`), deterministic, NOT fixed by
  VITRIOL_LRU_STRICT / VITRIOL_MATMUL_SYNC / CUDA_LAUNCH_BLOCKING. Same
  failure signature class as the September campaign (nondeterministic
  primitive failures under the streaming remap; transport byte-clean).
- **2026-10-16 20:00 - Rust arm status**: `vitriol-stream` builds clean
  (static link via -DVITRIOL_RUST_LIB). VITRIOL_RUST=1 engages the crate but
  it carries the same latent store-default assumption ("store fill but store
  not open" -> protocol violation): the crate was left mid-hunt in September
  before the non-store path was exercised. Fixing the crate default is
  mechanical; the ggml launch defect is the real blocker on both arms.
- **REMAINING (next session)**: (a) crate: default source to gguf when
  VITRIOL_STORE unset, exercise non-store path; (b) the ggml launch-path
  defect: compute-sanitizer + PDL interaction audit on the first remapped
  decode (quantize_row_q8_1 EINVAL with slots as src0) - this is the
  September "ggml scheduler workstream" resurfaced at Nemotron scale.
  Consider testing small-ne02 + minimal repro outside llama-server.
- Daily driver state: Qwen engine still down (since 2026-10-14, pre-session);
  live config = nemotron35-super-vl-ramshot; rollback = `vitriol config load
  qwen38-buffedmod-262k` + restart. Nemotron store file
  (/scratch/vitriol-store-nemo.bin, 38 GiB) kept on disk for the store arm.
- **2026-10-16 20:30 - RC4 identified: sticky CUDA error poisoning.** The
  per-slice `cudaHostRegister` in `ensure_expert_locked` fails with EINVAL on
  mmap-backed pages (registration only works on anonymous memory) and returns
  WITHOUT clearing CUDA's per-thread sticky last-error. The next
  `CUDA_CHECK(cudaGetLastError())` in ggml - after the `quantize_row_q8_1_cuda`
  launch inside the first remapped mul_mat_vec_q - reads the STALE EINVAL and
  aborts, misattributing it to that launch. Explains: determinism, immunity to
  LRU_STRICT/MATMUL_SYNC/launch-blocking, "wrong kernel" failure site. The
  September 35B runs never hit this because the launcher-era configs ran
  --no-mmap (anonymous buffer -> slice registration succeeds -> no poison).
  Why Qwen3.5 MoE worked: same anon-registered path, full pin tier functional.
- **Fix plan (approved 2026-10-16 20:30)**:
  1. clear sticky error after failed slice registration (1 line) + audit other
     VITRIOL runtime-API failure paths for uncaught last-error poisoning
     (driver cu* calls do not set runtime last-error - only runtime calls)
  2. mmap=on wrap arm: oracle -> 3x256 bench -> clean stop -> report card
  3. mmap=off anon arm (exact Qwen3.5-MoE-era config, registration works):
     same protocol; A/B decides ship arm; also falsifies/confirms RC4
  4. Rust arm after crate store-default fix; same protocol
  5. ship best arm as the working profile; Addendum C; AGENTS note
  Success = oracle-clean decode + t/s scaling with tier residency.
