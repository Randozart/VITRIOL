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
