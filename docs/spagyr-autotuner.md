# Spagyr — VITRIOL Hardware Autotuner: Design + Measured Boundaries

Date: 2026-08-06.

## 1. What Spagyr is

A VITRIOL feature (`--spagyr-tune`) that tunes the runtime to the specific hardware:
probe the box, sweep the real tunable knobs with measured benchmarks, freeze the winner
into the profile system. It is a *tuner/optimizer harness*, not a new runtime and not a
new math representation. Rationale: enterprise AOT engines (TRT-LLM) do per-GPU engine
compilation + tactic autotuning; Spagyr scales that idea down to consumer hardware
(GTX 1070 Ti class) and records which representations are measured-good or
measured-dead per hardware class.

## 2. Hardware fingerprint (S1)

| key | source |
| --- | --- |
| compute capability (CC) | cudaGetDeviceProperties |
| SM count | cudaGetDeviceProperties |
| VRAM total | cudaGetDeviceProperties |
| L2 cache size | cudaGetDeviceProperties |
| PCIe gen/width | lspci / nvidia-smi -q |
| DRAM total | /proc/meminfo |
| logical cores | nproc |
| RLIMIT_MEMLOCK / CAP_IPC_LOCK | ulimit -l; getcap |
| disk free (offload tier) | df |

Fingerprint cached per box (e.g. `~/.vitriol/fingerprint.json`); re-probe only on
`--spagyr-tune` re-invocation or hardware delta.

## 3. Measured boundaries (bitshaper-ai, 2026-08-06) — the blacklist seed

Source: `bitshaper-ai/.opencode/plans/2026-08-06-spagyr-shader-test.md` §14 and
`.../2026-08-06-amortization-batching.md` §6. These are MEASURED on GTX 1070 Ti
(sm_61, 2 MB L2, no Tensor Cores, DP4A-capable), not assumed.

| boundary | measured | rule |
| --- | --- | --- |
| weights-as-code (R2-FOLD) | bit-exact but **92.8× packed bytes** (346 MB/tensor vs 3.73 MB); whole model ~73 GB; un-cacheable on 2 MB L2 | refuted; blacklist for CC 6.1, L2 < ~8 MB |
| IQ-LUT execution on Pascal | infeasible: codebook tables exceed 48 KB SMEM | refuted; blacklist for CC 6.1 |
| activation-delta execution (E-I1) | 14.5× worse; deltas unstable (median |Δ|/rms 0.52–0.72) | refuted |
| input prefolding | exact input prediction impossible; structural prefolding contradicted | refuted |
| dead-lane skip | 39–47% zero lanes measured, but dead-skip is a compute lever on a memory-bound box | recorded; NOT an autotune knob |
| **batch amortization** | **3.6× per-token at batch R=16** (0.258 → 0.072 ms), parity bit-exact; knee ~R=16, flat after (compute-bound) | **the autotune target**: maps to ubatch-size/parallel |

Rule of thumb for the tuner: decode batch/ubatch is the highest-leverage knob on this
class of hardware because the weight fetch is the bottleneck and amortizes across the
batch; instruction/LUT representations never beat packed bytes on small-L2 parts.

## 4. Profile schema extension (S0)

Existing profile: `~/.vitriol/profiles/<name>/config` (TOML-ish, sections
`[gpu] [model] [vitriol] [server]`). Add `[spagyr]`:

```toml
[spagyr]
schema = 1
fingerprint = "/path/to/fingerprint.json"
knee_ubatch = 16
knee_parallel = 4
knee_threads = 4
tuned_at = "2026-08-06T00:00:00Z"
refuted_transforms = ["r2_fold", "iq_lut_pascal", "activation_delta_e1", "input_prefold"]
```

The launcher reads `[spagyr]` + `[vitriol]` and builds server flags + VITRIOL env from
them. `refuted_transforms` is read-only at launch (only Spagyr rewrites it) so a future
hardware fingerprint can skip known-dead representations without re-measuring.

## 5. Knob map

| knob | flag / env | swept in |
| --- | --- | --- |
| decode ubatch | `--ubatch-size` | S2 (primary, maps to measured knee) |
| decode batch | `--batch-size` | S2 |
| parallel slots | `--parallel` | S2 |
| threads | `--threads` | S2 |
| gpu layers | `--n-gpu-layers` | S4 |
| LRU VRAM | `VITRIOL_LRU_MB` | S4 |
| locked working set | `VITRIOL_MAX_LOCKED_MB` | S4 |
| predictive prefetch | `VITRIOL_PREDICTIVE_PREFETCH` | S4 |
| pin first layers | `VITRIOL_PIN_FIRST_N_LAYERS` | S4 |
| prune experts | `VITRIOL_PRUNE_EXPERTS` | S4 |
| disk offload | `VITRIOL_DISK_OFFLOAD` | S4 (deferred) |

Deferred: `--cont-batching`, `--flash-attn`, `--tensor-split`.

## 6. End-to-end validation (S3, S5)

1. Correctness gate BEFORE timing: fixed prompt ("Write a Python function for merge
   sort."), 64-token generation, verify coherent non-empty text (DeepSeek/Mellum had a
   known correctness concern — gated here).
2. Interleaved timing, tuned vs stock defaults, same box, clean state.
3. Full-speedup report: gen t/s, eval t/s, before/after table.

## 7. Provenance

VITRIOL is the user's own repo (AGENTS.md §2.2: freely borrowable). No third-party
code is copied. The measured boundaries come from bitshaper-ai's own experiments;
TQ1_0 reference machinery there is parity-verified against ggml's CPU path. This is a
feature of the user's own runtime.
