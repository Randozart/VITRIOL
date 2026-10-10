# Cooperative VRAM: Rust governor + engine hooks

- **Date:** 2026-10-10 14:0x
- **Status:** EXECUTED 2026-10-10 (G1-G4 complete; live canary deferred to
  restart day). Commits: governor G1 `922b175`, G2 inner `2a9b79706`/outer
  `dd2b0a6`, G3 inner `d62b88172`/outer `797e291`, G4 `0828b55`.
- **Predecessor:** `hardening-and-cuda-oxide-2026-10-10.md` (stages 1-4 shipped)
- **Standing constraint:** live engine NOT restarted in this batch (restart-day
  checklist stays parked). Governor + engine code land, live canary deferred.

## Problem

Engine pins both cards at boot (~11.3/11.7 of 12 GiB) and never reacts to
foreign GPU tenants. Policy today is manual (AGENTS: "if any external GPU
tenant returns, load qwen38-buffedmod-208k"). Engine is also the last OOM
victim (`OOMScoreAdjust=-500`) - it WINS every VRAM fight. Demonstrated live
2026-10-10: cuda-oxide context creation failed on dev0 (DriverError 210).

## Verified facts (2026-10-10, read-only)

- KV buffer is allocated for the WHOLE window at load (AGENTS rule 5):
  cell eviction (`evict_sparse`, `vitriol_probe_finish` eager sweep,
  `llama-kv-cache.cpp:2960-3120`) frees SLOTS for reuse but not VRAM.
- `GGML_CUDA_NO_VMM=OFF` in the live build; CUDA buffer free path does
  `cuMemUnmap` + `cuMemRelease` (`ggml/src/ggml-cuda/ggml-cuda.cu:675-719`)
  -> freeing/reallocing the KV buffer RETURNS physical pages to the driver.
  Runtime context shrink is therefore real, not cosmetic.
- Eager sweep floor/protect are env-at-boot statics
  (`llama-kv-cache.cpp:3013`); sweep rides probe cadence (score_every=16) so
  an IDLE engine never sweeps.
- VITRIOL has zero custom GPU kernels; probe scoring already runs on GPU via
  ggml ops (the "GPU-ize probe" spike premise was wrong - documented here to
  prevent resurrection).
- 2x RTX 3060 12 GiB, driver 615.71, nvcc 13.4 (AGENTS "1070 Ti" text stale).

## Architecture

Control plane OUTSIDE the engine (survivorship: the actor that restarts/
resizes must outlive the engine; failure isolation; oom_score_adj is a
process credential). Data plane telemetry already in engine (/metrics).

### A. `vitriol-governor/` (Rust crate, outer repo)

- Deps: nvml-wrapper (MIT), serde+toml, ureq. License Apache-2.0 OR MIT,
  PROVENANCE header.
- 1 s NVML poll: per-GPU used/free + foreign compute PID set.
- Layered response, cheap -> expensive:
  1. RELIEVE (ms): pressure -> POST /kv-policy {floor, sweep} -> engine
     sweeps low-score cells (sub-second; direct trigger when idle).
     Frees cells only (engine headroom), not VRAM for tenants.
  2. RESIZE (seconds): sustained pressure -> POST /ctx/resize {n_ctx} ->
     spool all slots to RAM tier, free + realloc KV buffer (VMM returns
     pages), restore best states. No process restart.
  3. YIELD (last resort): `scripts/vitriol config load qwen38-buffedmod-208k`
     + `systemctl restart vitriol-server` (warm restore ~300 ms), flip
     engine oom_score_adj -500 -> 0 while yielded.
  4. Recovery: free VRAM > ceiling sustained -> walk back up.
- Hysteresis (TOML-tunable): fast-path confirm 10 s, de-escalate 5 min,
  min-dwell 10 min, jitter. Defaults: floor 1.5 GiB free, ceiling 4 GiB.
- Single-writer state file; journal-friendly logs; fingerprint diff on every
  transition (doctrine; dry-run fingerprint capture).
- Unit: `scripts/systemd/vitriol-governor.service` (system scope, root, so
  oom_score_adj writes work; polkit already lets randozart manage the two
  vitriol units, governor restarts them as root).
- Policy FSM unit-tested (`cargo test`), transitions pure functions.

### B. Engine work (inner fork)

1. `/kv-policy` (~60 LoC): std::atomic floor/protect in llama_kv_cache
   (env statics become initial values), setter task + idle-safe sweep
   trigger using the SLOT_ERASE defer-when-busy pattern
   (`server-context.cpp` SERVER_TASK_TYPE_SLOT_ERASE). kv-policy atomics
   safe during decode (allowed in the is_yielding set like METRICS).
2. `/ctx/resize` (~150 LoC): SERVER_TASK_TYPE_CTX_RESIZE, defers while any
   slot processing; prompt_save all slots -> rebuild ctx_tgt memory at
   target n_ctx (weights shared, untouched) -> restore best states.
   KNOWN RISKS (resolve during G3 before enabling):
   - server_prompt_cache identity may include context size -> mirror keys
     could orphan on resize; check `server_prompt_cache_identity` fields.
   - MTP draft ctx_dft rebuild coupling; checkpoint ring spool; probe state.
   - Config-gated `allow_ctx_resize` (default OFF until certified live).
3. `/metrics`: resident KV cells + KV buffer bytes per GPU gauges -> governor
   reads engine truth (extends `m.n_kv_ejected` plumbing at
   `server-context.cpp:875`).

### C. Tests

- Extend `scripts/test-kvtier-smoke.sh`: kv-policy sweep assert; (after G3)
  resize round-trip assert on the throwaway CPU engine.
- Governor: `cargo test` policy transitions.
- Live canary deferred to restart day.

## Execution order

1. G1 governor crate: observe + FSM + YIELD path (works day one, Phase 1).
2. G2 /kv-policy + metrics; wire governor RELIEVE.
3. G3 /ctx/resize (config-gated); wire governor RESIZE.
4. G4 smoke-test extensions, docs, commits per stage.

## Owner defaults (veto anytime)

- yield profile = qwen38-buffedmod-208k (ts 24,24); full = 262k (ts 27,21)
- thresholds: floor 1.5 GiB / ceiling 4 GiB / dwell 10 min (TOML)
- /ctx/resize config-gated OFF by default
- governor language Rust

## Parked (unchanged)

cuda-oxide spike resume (kernel artifact lookup bug, toolchain PROVEN),
GGML_SCHED_MAX_COPIES=64 experiment, restart-day checklist (systemd return,
OOMScoreAdjust verify, /metrics live check, trim-WRN watch, sidecar enable).


## Execution results (2026-10-10 18:xx)

- **G1**: governor crate (observe/FSM/YIELD), 6/6 tests, Praetor clean.
  Foreign-tenant detection needed a 768 MiB tenant floor: NVML compute list
  is full of desktop residents (kwin 157 MiB, electron, Discord) that would
  otherwise yield the engine forever.
- **G2**: /kv-policy live (set persists, echo, bad-JSON 400, buffer_bytes
  gauge 48 MiB on the smoke engine). Boot env statics -> atomics.
- **G3**: /ctx/resize verified end-to-end on the throwaway engine:
  8192 -> 4096 -> 8192, slot spooled each way, warm restore 603/607 tokens
  cached, no re-prefill, no restart. common_init_result::context_from_model
  + init_context extraction; n_ctx dropped from build_key (position-
  addressed states fit any window; per-state fit gate added at load).
- **G4**: governor ladder v2 (7/7 tests, Praetor 0 warnings), launcher
  ctx_resize -> cxr=1 fingerprint, live config + profiles opt-in
  (fingerprint diff: ONLY cxr=1 added), harness T4/T5.
- **Environment findings**: (1) the build's fit probe initializes ALL
  backends incl Vulkan - vkCreateDevice intermittently fails/aborts when
  the host is saturated; smoke engines run `-device none -fit off`.
  (2) Smoke prefill thrashes swap when MemAvailable < ~1 GiB (live engine
  owns the 12 GiB prompt tier) - harness now preflights MemAvailable >= 3 GiB.
  Earlier same-day full pass (T1-T3) proves the harness itself sound;
  T4/T5 logic is the manually-verified endpoint smoke sequence.
- **Deferred**: live governor canary (needs engine restart day),
  full-harness rerun when RAM frees, systemd unit enable/installer run
  (sudo), cuda-oxide spike resume.
