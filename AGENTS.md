# VITRIOL AI Agent Guidelines

## Testing Protocol

1. **Residency rule (2026-08-24, supersedes "always DMA")**: stream/DMA offloading ONLY when weights exceed combined VRAM (e.g. 35B-class). For resident-capable quants (≤ ~20 GiB combined) the default is `VITRIOL_MODE=off` — weights fully VRAM-resident; streaming a fitting model starves GPUs on DDR3/PCIe expert fetches and is a pessimization. Streaming in an experiment record must be justified.

2. **CAP_IPC_LOCK is optional on this host**: CUDA pinned allocations do not count against RLIMIT_MEMLOCK; all 2026-08-24 certifications ran uncapped. `scripts/build-llama-server.sh` reapplies caps best-effort after builds; `sudo vitriol setup` remains for hosts where mlock fails. Never pass `--ctx-checkpoints 0` (heap corruption) or `--cache-ram 0` (no readiness).

3. **Always kill stale servers** with `killall -9 llama-server` before starting a new one.

4. **Flag provenance (mandatory)**: every launch emits a `VITRIOL-FINGERPRINT:` line (launcher, server main, and runners). Every benchmark RESULT embeds full argv. Every report/log excerpt must be traceable to its fingerprint — silent flag drift is a review blocker. Profile files under `profiles/` are canonical config sources.

5. **Window ≠ depth**: KV is allocated for the whole window at load. Context claims must state FILLED token counts (see lull-certification-report Addenda 5–6).

## Documentation

1. **Write all findings in `.md` reports** with ISO 8601 timestamps (YYYY-MM-DD HH:MM).
2. Place reports in `.opencode/plans/` for integration plans and research findings.
3. Place session logs in `SESSION_LOG_*` and experiment logs in `EXPERIMENT_LOG.md`.
4. Include exact command output, tensor shapes, and error messages in reports.
5. Update the anchored summary in each session with progress, blockers, and decisions.

## Code Conventions

- This is a fork of `ggml-org/llama.cpp` with VITRIOL modifications. The `vitriol` branch contains our changes.
- The VITRIOL predictor is in `ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`.
- The server context checkpoint logic is in `tools/server/server-context.cpp`.
- All VITRIOL env vars are prefixed with `VITRIOL_`.

## Calibration Tool (Rust)

- **`libvitriol/`** — Rust binary for `vitriol calibrate --quick`.
- Build with `cargo build --release` in `libvitriol/`.
- Source files: `gguf.rs` (GGUF v3 parser), `probe.rs` (hardware), `estimator.rs` (VRAM model), `main.rs` (CLI).
- The Rust binary is called by `scripts/vitriol` if built; falls back to Python `libvitriol/gguf_reader.py`.
- **No hardcoded model constants** — all VRAM values computed from GGUF tensor data.
- Self-computing formula: `VRAM = base_model + pin * per_layer_expert + ctx * kv_per_token + scratch + overhead`.
- Overhead heuristic: Pascal=1800, Turing=2200, Ampere=2800, Ada=3200 MiB.
- KV cache computed from model dims: `(embd_len / head_count) * head_count_kv * 2.5 / 1M`.
- Per-layer expert cost from tensor name analysis (`ffn_*_exps` patterns).

## Sweep Controller (Python)

- **`libvitriol/sweep_controller.py`** — automated benchmark sweep via HTTP POST `/completion`
- Starts `llama-server` subprocess per config, benchmarks 64-token generation (1 warmup + 3 measured rounds), reports t/s
- Server readiness: polls `/health` then `/completion` until model fully loads (~15s warmup on GTX 1070 Ti)
- Use: `python3 libvitriol/sweep_controller.py --model <path> --pin 0 8 12 16 --mtp 0 3 5 6`
- Sweep results: 25-config full sweep runs in ~20 minutes; MTP provides zero benefit on this hardware (all scores ~9.7-9.98 t/s)

## MTP (No Benefit on This Hardware)

- Full 5×5 sweep (pin 0/4/8/12/16 × mtp 0/2/4/5/6) completed
- **All configs: 9.6–9.98 t/s**, tightly clustered — MTP has zero measurable effect with Qwen3.6-35B on GTX 1070 Ti
- pin=16 + MTP regresses to 8.58 t/s (VRAM pressure from draft buffers)
- **Optimal: pin=12, mtp=0 or mtp=2, ubatch=128, ctx=65536** → 9.98 t/s
- Full report: `.opencode/plans/mtp-sweep-report-2026-05-25.md`

## Licensing and Provenance (GPL-2.0)

VITRIOL and its llama.cpp fork are licensed under the **GNU GPL v2** (see `LICENSE`).
Before incorporating any third-party code, check GPL-2.0 compatibility:

| source license | may copy into VITRIOL? |
|---|---|
| GPL-2.0 | yes (same license) |
| MIT / BSD / ISC / zlib / Unlicense / CC0 | yes, with attribution retained |
| **Apache-2.0** | **NO — incompatible with GPL-2.0** |
| academic / other restrictive | NO — re-derive only |

Apache-2.0 and GPL-2.0 are mutually incompatible (FSF and Apache both document this):
a derivative of Apache-2.0 code must carry Apache-2.0's terms, which GPL-2.0 forbids adding
to a GPL-2.0 combined work. So an Apache-2.0 implementation may be **studied and re-derived
(ideas → independent implementation), never copied**.

Every algorithm-bearing module carries a `PROVENANCE` header:
`// PROVENANCE: inspiration — <repo> (<license>), what was learned, not copied`.
`inspiration` entries must name the repo, its license, and what was learned (not what was
borrowed). See `docs/provenance/`.

## Qwen3.8-27B Dual-GPU Config (RTX 3060 + GTX 1070 Ti)

Model: `~/Downloads/Qwen3.8-27B-Q3_K_M.gguf` (unsloth, qwen35 arch, embedded MTP head).

Saved VITRIOL profiles (load with `vitriol config load <name>`):

| profile | ctx | MTP | t/s | notes |
|---|---|---|---|---|
| `qwen38-mtp-131k` | 49152 | n=1 | ~14.1 shallow-bench | **default/winner**, ts 27,9 (26,10 on merged base), q4k/q4v; meta: "131K OOMs ~45-61K tokens on this dual-GPU pair" |
| `qwen38-262k` | 262144 | off | ~11.0 | max native ctx, ts 24,12, q4k/q4v |

Depth-filled reality (2026-08-24 certification, merged base, chunked/single-shot
prefill + 3×64 decode at depth — see `.opencode/plans/lull-phase0-report-2026-08-24.md`
Addenda 5–6):
- Q3_K_M + `tq3_0` KV, ts 26,10 ub64: **54,692 tok @ 9.21 t/s** (beats the historical
  45–61k OOM zone); 43,890 tok @ 9.47 t/s.
- UD-IQ3_S + `tq3_0` KV, ts 26,10 ub64: **96,836 tok @ 11.32 t/s**.
- IQ2_S @ 64,634 tok: 11.7–12.7 t/s.
- VRAM creep ~23 KiB/token on dev0 during long prefills is the depth wall
  (independent of KV bits; NOT fixed by GGML_CUDA_NO_VMM=1). Window ≠ usable
  depth: shallow-bench numbers do not certify filled-context operation.

Recommended working config (certified): `-ngl 99 -ts 26,10 --main-gpu 0 -ub 64
--cache-type-k tq3_0 --cache-type-v tq3_0` (TurboQuant KV = 3.5 bpw, −22% vs q4_0;
per-device overrides via `VITRIOL_KV_QUANT[_K|_V]_GPU<d>`). Add
`--spec-type mtp --spec-draft-n-max 1` for the 49k-window profile.
Master deep-context profile: `vitriol config load qwen38-master`
(IQ3_S, c131072 tq3_0, sparse+probe; export VITRIOL_KV_SCORE=probe and
VITRIOL_POOL_RESET=1 first — certified 96,836 tok @ 11.32 t/s).

Required flags (all wired into `scripts/vitriol` config now):
`-ngl 99 -ts 26,10 --main-gpu 0 -ub 64 --cache-type-k tq3_0 --cache-type-v tq3_0 --spec-type mtp --spec-draft-n-max 1`

MTP draft depth: n_max must be **1** (re-confirmed 2026-08-24: n=2 → 12.71 vs n=1 → 14.05
t/s shallow-bench). Zero measurable end-to-end benefit on this hardware (2026-05-25 sweep +
2026-08-24 recheck) — omit unless A/B-ing. Depth>=2 regresses because chained MTP-head drafts
drift (acceptance decays) and each costs ~8 ms.
Sweep: `.opencode/plans/qwen38-phase-d-bottleneck-2026-08-19.md`.
Deep-context certification: `.opencode/plans/lull-certification-report-2026-08-24.md`.

Key facts:
- Build must target BOTH archs: `cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86"` (native-only default misses sm_86 → "no kernel image" crash).
- KV quantization: `--cache-type-k/v` accepts `tq3_0`/`tq3_1s`/`tq3_4s` (TurboQuant,
  2026-08-24 certified) in addition to f16/q8_0/q4_0/q4_1/iq4_nl. Per-device
  asymmetric overrides exist via `VITRIOL_KV_QUANT[_K|_V]_GPU<d>` (llama-kv-cache.cpp).
- MTP only works if `qwen35_mtp` arch string exists in `src/llama-arch.cpp` (fixed 2026-08-18). Symptom when broken: `unknown override architecture: 'qwen35_mtp'` + `speculative=none`.
- 262K + MTP does NOT fit (Pascal compute buffers). Drop MTP for 262K.
- [STALE?] opencode provider entries `qwen38-mtp`/`qwen38-262k` in `~/.config/opencode/opencode.jsonc` predate the 2026-08-24 certifications — verify before relying on them. Hermes-agent uses `~/.hermes/config.yaml` ("Lapis Occultus" → qwen38-master endpoint).
