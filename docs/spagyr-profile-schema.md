# Spagyr Profile Schema (`[spagyr]` section)

Date: 2026-08-06. Schema version 1.

Profiles live at `~/.vitriol/profiles/<name>/config` (TOML-ish INI). Existing sections:
`[gpu]`, `[model]`, `[vitriol]`, `[server]`, `[engine]`, `[kv]`, `[spec]`, `[memory]`,
`[lookup]`, `[chimera]`. Spagyr adds `[spagyr]`, written by `llama-server --spagyr-tune`
and read by the launcher.

> Profile portability warning: the current `mellum2` profile was generated on a 64 GB
> DDR4 box (header says "64GB DDR4"); this repo is developed on a 15 GB box. Profiles
> are **box-specific** — Spagyr's `fingerprint` field makes cross-box staleness
> detectable (launcher warns if fingerprint != current box).

## `[gpu]`

```toml
[gpu]
device = 0            # CUDA device index
exclude_secondary = true
```

## `[model]`

```toml
[model]
path = /abs/path/to/model.gguf
context = 32768
threads = 4
ngl = 24              # n_gpu_layers
expert_count = 0
```

## `[vitriol]`

Maps 1:1 to env vars read by `vitriol_init()`.

```toml
[vitriol]
mode = stream                     # VITRIOL_MODE
lru_mb = 2048                     # VITRIOL_LRU_MB
max_locked_mb = 0                 # VITRIOL_MAX_LOCKED_MB (0 = auto from fingerprint)
predictive_prefetch = on          # VITRIOL_PREDICTIVE_PREFETCH
pin_first_n_layers = 0            # VITRIOL_PIN_FIRST_N_LAYERS
prune_experts = 0                 # VITRIOL_PRUNE_EXPERTS
disk_offload = off                # VITRIOL_DISK_OFFLOAD
verbose = true
output_cache = off
reasoning = off
```

## `[server]`

```toml
[server]
host = 0.0.0.0
port = 8080
parallel = 4              # --parallel (decode slot count; a Spagyr knob)
```

## `[engine]` (existing; carries the decode knobs Spagyr tunes)

```toml
[engine]
mode = vitriol-dma
ubatch_size = 128         # --ubatch-size (Spagyr knee knob)
```

## `[spagyr]` (new)

```toml
[spagyr]
schema = 1
fingerprint = "~/.vitriol/fingerprint.json"   # resolved by the launcher
knee_ubatch = 16        # decode ubatch-size at the measured knee
knee_parallel = 4       # parallel slots at the knee
knee_batch = 16         # batch-size at the knee
knee_threads = 4        # threads at the knee
ngl = 24                # n_gpu_layers winner
tuned_at = "2026-08-06T00:00:00Z"
refuted_transforms = ["r2_fold", "iq_lut_pascal", "activation_delta_e1", "input_prefold"]
```

`refuted_transforms` is launch-read-only: only `--spagyr-tune` may rewrite it. The
launcher uses it to skip known-dead representations without re-measuring.

## Launcher contract

Given a profile, the launcher:
1. reads `[gpu]`, `[model]`, `[spagyr]` → builds server flags
   (`--n-gpu-layers`, `--threads`, `--batch-size`, `--ubatch-size`, `--parallel`);
2. reads `[vitriol]` → exports `VITRIOL_*` env vars;
3. reads `[server]` → host/port;
4. logs the active `[spagyr]` config at startup for auditability.
