# Spagyr - VITRIOL Hardware Autotuner (--spagyr-tune)

Date: 2026-08-06.

Mirror of the master record in
`bitshaper-ai/.opencode/plans/2026-08-06-spagyr-vitriol-autotuner.md` (that file is
canonical, including all measured facts and the cross-repo migration map). Design and
measured boundaries: `docs/spagyr-autotuner.md`. Profile schema:
`docs/spagyr-profile-schema.md`.

## Deliverable

`llama-server --spagyr-tune [--spagyr-model MODEL]` probes the specific hardware,
sweeps the real tunable knobs with measured benchmarks, finds the knee, verifies
end-to-end, and freezes a profile under `~/.vitriol/profiles/<model>/config` with a new
`[spagyr]` section. Normal launches consume the profile (Spagyr is the first real
profile consumer; none exist today).

## Flow

1. Probe + fingerprint (CC, SMs, VRAM, L2, PCIe gen/width, DRAM, cores,
   RLIMIT_MEMLOCK/CAP_IPC_LOCK, disk). Cached per box.
2. Decode-knob micro sweep: ubatch-size x batch-size x parallel x threads on synthetic
   TQ1_0/q8 GEMV; parity before timing; find the knee (expect ~R=16 on GTX 1070 Ti).
3. Decode-knob end-to-end validation on the real model: correctness gate (legible
   output) then t/s.
4. VITRIOL-knob sweep: n-gpu-layers, LRU_MB, MAX_LOCKED_MB, predictive_prefetch,
   pin_first_n_layers, prune_experts.
5. Full-speedup measurement: tuned vs stock defaults, baseline table, interleaved runs.
6. Freeze profile: [spagyr] = fingerprint + config + knee + refuted_transforms.

## Integration anchors (verified)

- Arg hook: `tools/server/server.cpp:82` `common_params_parse(..., LLAMA_EXAMPLE_SERVER)`.
- Knobs exist: --batch-size, --ubatch-size, --parallel, --threads, --n-gpu-layers,
  --tensor-split (common/arg.cpp).
- VITRIOL env read in vitriol_init() (ggml/src/ggml-cuda/vitriol-cuda-integration.cpp:
  240+): VITRIOL_MODE, LRU_MB, MAX_LOCKED_MB, PREDICTIVE_PREFETCH, PIN_FIRST_N_LAYERS,
  PRUNE_EXPERTS, DISK_OFFLOAD.
- Profile schema: ~/.vitriol/profiles/<name>/config (TOML-ish: [gpu] [model] [vitriol]
  [server]).

## Phases

- S0 - profile schema extension + fingerprint schema + spec.
- S1 - probe tool + fingerprint cache.
- S2 - micro decode-knob sweep (parity + knee).
- S3 - end-to-end decode validation on real model.
- S4 - VITRIOL-knob sweep.
- S5 - full-speedup baseline + freeze profile + launch integration.
- S6 - refuted-transform blacklist + docs + provenance.

## Scope first-cut

Decode knobs first to demonstrate the loop; VITRIOL knobs in the same pass. Deferred:
--cont-batching, --flash-attn, --tensor-split.

## Baseline table (fill on execution)

| config | gen t/s | eval t/s | correctness (legible?) |
| --- | --- | --- | --- |
| stock defaults | TBD | TBD | TBD |
| spagyr decode tune | TBD | TBD | TBD |
| spagyr decode + vitriol tune | TBD | TBD | TBD |

## Dependencies

- Runs on VRAM-fit models immediately (DeepSeek-Coder-V2-Lite, Mellum2). Ternary Qwen
  behind the mlock unblock: `sudo prlimit --pid $$ --memlock=unlimited:unlimited`.
- Spagyr is a tuner, not new math: it optimizes config and records refuted reps.
