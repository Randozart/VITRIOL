# VITRIOL AI Agent Guidelines

## Testing Protocol

1. **Residency rule (2026-08-24, supersedes "always DMA")**: stream/DMA offloading ONLY when weights exceed combined VRAM (e.g. 35B-class). For resident-capable quants (≤ ~20 GiB combined) the default is `VITRIOL_MODE=off` — weights fully VRAM-resident; streaming a fitting model starves GPUs on DDR3/PCIe expert fetches and is a pessimization. Streaming in an experiment record must be justified.

2. **CAP_IPC_LOCK is optional on this host**: CUDA pinned allocations do not count against RLIMIT_MEMLOCK; all 2026-08-24 certifications ran uncapped. `scripts/build-llama-server.sh` reapplies caps best-effort after builds; `sudo vitriol setup` remains for hosts where mlock fails. Never pass `--ctx-checkpoints 0` (heap corruption) or `--cache-ram 0` (no readiness).

3. **Always kill stale servers** with `killall -9 llama-server` before starting a new one.

4. **Flag provenance (mandatory)**: every launch emits a `VITRIOL-FINGERPRINT:` line (launcher, server main, and runners). Every benchmark RESULT embeds full argv. Every report/log excerpt must be traceable to its fingerprint — silent flag drift is a review blocker. Profile files under `profiles/` are canonical config sources. **Config keys are flags too** (2026-09-03: the 8-31 retarget silently dropped `[spec]` from `~/.vitriol/config`, costing 40% decode for weeks — see `.opencode/plans/mtp-flag-drift-postmortem-2026-09-03.md`). Any config write must be followed by a fingerprint diff against the previous one; speed-bearing keys (`[spec]`, `[kv] score*`, `ts`, `ubatch`) are provenance-bearing. The fingerprint carries `spec=<type>:<n_max>` since 2026-09-03 — `spec=none:0` on a 27B launch is a red flag.

5. **Window ≠ depth**: KV is allocated for the whole window at load. Context claims must state FILLED token counts (see lull-certification-report Addenda 5–6).

6. **HF CDN stalls on this host** (2026-08-31): `hf download` transfers ~16 MiB then the connection goes silent. Use a ranged-resume downloader (Range + reconnect-on-stall, ~12 MiB/s) like `/tmp/mellum2_fetch.py`; verify against the tree-API `lfs.oid` sha256 — the resolve-HEAD `etag` is the xetHash, NOT the sha256.

## Documentation

1. **Write all findings in `.md` reports** with ISO 8601 timestamps (YYYY-MM-DD HH:MM).
2. Place reports in `.opencode/plans/` for integration plans and research findings.
3. Place session logs in `SESSION_LOG_*` and experiment logs in `EXPERIMENT_LOG.md`.
4. Include exact command output, tensor shapes, and error messages in reports.
5. Update the anchored summary in each session with progress, blockers, and decisions.

## Workflow

1. **Git commit after every stage**: each completed feature or fix gets its own commit immediately — no batching, no "commit at the end". If a stage is done and tests pass, commit it before starting the next stage.

## Code Conventions

- This is a fork of `ggml-org/llama.cpp` with VITRIOL modifications. **`main` is the canonical daily-driver branch in BOTH repos** (consolidated 2026-09-01 — see `.opencode/plans/branch-consolidation-2026-09-01.md`). Outer repo: all feature work lives on `main` (was `officina`). Inner fork (`llama.cpp/`): `main` is the live line ported onto ggml-org upstream (was `vitriol-ku`); `vitriol` is the frozen pre-port archive, `master` tracks the fork's published state. `lull-kv` lives in the `VITRIOL-lull` worktree.
- The VITRIOL predictor is in `ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`.
- The server context checkpoint logic is in `tools/server/server-context.cpp`.
- All VITRIOL env vars are prefixed with `VITRIOL_`.

## Vendor Patch Rule (2026-09-01 incident — do not repeat)

`officina/runtime/patched/*` are GENERATED or vendor-pinned copies. **A patch that exists only in a generated file is a patch waiting to be lost.**

- Every modification to `interactive-mode.officina.js` MUST be an anchored patch in `officina/runtime/build-patch.mjs` AND leave a `[officina P<n>]` marker that the build's canary assertions grep for. Running `node runtime/build-patch.mjs` regenerates the file from the pristine reference — anything hand-edited into the generated file is **silently wiped**.
- Incident (2026-09-01): P4 sidebar bottom-anchor, scrollback (now P10), P8 mouse reporting, and P9 mode tint were hand-applied to the generated file only. Regeneration during unrelated work erased all four committed features (no scroll, runaway sidebar, no mode recolor). They are now anchored patches with 8 build canaries; the build FAILS if any canary is missing.
- `session-selector.officina.js` and `markdown.officina.js` are hand-maintained (NOT regenerated) — re-base them manually on pi bumps; never assume build-patch covers them.
- After ANY build-patch change, run `node runtime/build-patch.mjs` and confirm `canaries ok` before shipping.

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

## Cloud Escalation Policy (ascensus)

Cloud tier is provider-agnostic via `~/.vitriol/secrets` (current:
`provider = zai-coding-plan`, GLM `glm-4.6`; the 2026-09-02 smoke test
proved escalate → ledger → Hermetis store-back → dedup hit end-to-end,
first escalation cost €0.00048, the repeat was free).
Escalation is euro-capped (`ASCENSUS_EUR_DAILY=1`, monthly 30) and
self-tapers: every escalation is stored to Hermetis, so a problem class pays
the cloud price **once**. Early escalation of recurring problem classes is an
investment, not an expense. Escalate when ANY of:

1. One bug has consumed **3+ failed hypotheses** locally.
2. You are about to do something irreversible or hard to undo (force-push,
   schema/data migration, deleting unbacked state) and feel <90% sure.
3. The error class is long-tail platform behavior — kernel/systemd/CUDA/
   driver internals — where small models hallucinate confidently.

Do NOT escalate for: routine coding, this repo's own conventions (they live
in docs/), anything already answered in Hermetis (dedup handles it).
When the budget gate refuses, answer locally — never retry-loop ascensus.

Auto-route (officina extension, 2026-09-02): every officina turn is
classified (complexity + privacy) and routed advisory or enforced.
`OFFICINA_ROUTE_MODE=suggest|auto|off` (default suggest — advisory only),
`OFFICINA_ROUTE_THRESHOLD=0.0–1.0` (default 0.5; higher = more local),
`OFFICINA_NO_AUTO_ROUTE=1` kill switch, `OFFICINA_ROUTE_ASCENSUS_URL`
(default `http://127.0.0.1:8283`). Cloud tier escalates through ascensusd
(single budget writer) and injects the verdict as a tail message — it never
switches models. Sensitive/confidential turns never reach the cloud.

## Licensing and Provenance (Apache-2.0 OR MIT)

VITRIOL and its llama.cpp fork are dual-licensed under the
**Apache License 2.0 OR the MIT License** (SPDX: `Apache-2.0 OR MIT`; see
`LICENSE` and `LICENSE-MIT`). Downstream recipients choose either term.
History: GPL-2.0 → Apache-2.0 on 2026-08-28; dual Apache-2.0/MIT from
2026-08-31 (both repos are owned by the same author; no external contributors
are affected). `officina/package.json` carries the same SPDX expression.
Before incorporating any third-party code, check compatibility (the table
governs INCOMING code only — our outgoing dual offer is unaffected):

| source license | may copy into VITRIOL? |
|---|---|
| Apache-2.0 | yes (same license) |
| MIT / BSD / ISC / zlib / Unlicense / CC0 | yes, with attribution retained |
| GPL-2.0 / LGPL / AGPL | NO — copyleft, keep isolated or re-derive |
| academic / other restrictive | NO — re-derive only |

Every algorithm-bearing module still carries a `PROVENANCE` header:
`// PROVENANCE: inspiration — <repo> (<license>), what was learned, not copied`.
`inspiration` entries must name the repo, its license, and what was learned (not what was
borrowed). See `docs/provenance/`. (Historical note: entries predating 2026-08-28 marked
"re-derived only" were written under the previous GPL-2.0 regime; the constraint no longer
applies, but the records are kept as-is for accuracy.)

## Qwen3.8-27B Dual-GPU Config (RTX 3060 + GTX 1070 Ti)

Model: `~/Downloads/Qwen3.8-27B-Q3_K_M.gguf` (unsloth, qwen35 arch, embedded MTP head).

**LIVE CONFIG (blessed 2026-09-04)**: `~/.vitriol/config` runs `-ts 22,14`
(2 more layers to the 1070 Ti for 3060 display headroom), ctx 81920,
`parallel = 1`, q4_0 KV, `[spec] type=mtp draft_n_max=1` — pinned via
`vitriol config bless`; launch fingerprints carry `spec=` and `par=`
since 2026-09-04. The 26,10 / 27,9 splits below are the OLDER profile
history — do not quote them as the current operating point.

Saved VITRIOL profiles (load with `vitriol config load <name>`):

| profile | ctx | MTP | t/s | notes |
|---|---|---|---|---|
| `qwen38-mtp-131k` | 49152 | n=1 | ~14.1 shallow-bench | SUPERSEDED by the live config as daily driver; ts 27,9 (26,10 on merged base), q4k/q4v; meta: "131K OOMs ~45-61K tokens on this dual-GPU pair" |
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

Recommended working config (certified): `-ngl 99 -ts 22,14 --main-gpu 0 -ub 64
--cache-type-k tq3_0 --cache-type-v tq3_0` (TurboQuant KV = 3.5 bpw, −22% vs q4_0;
per-device overrides via `VITRIOL_KV_QUANT[_K|_V]_GPU<d>`). Add
`--spec-type mtp --spec-draft-n-max 1` for the 49k-window profile.
Master deep-context profile: `vitriol config load qwen38-master`
(IQ3_S, c131072 tq3_0, sparse+probe; export VITRIOL_KV_SCORE=probe and
VITRIOL_POOL_RESET=1 first — certified 96,836 tok @ 11.32 t/s).

Required flags (all wired into `scripts/vitriol` config now):
`-ngl 99 -ts 22,14 --main-gpu 0 -ub 64 --cache-type-k q4_0 --cache-type-v q4_0 --spec-type mtp --spec-draft-n-max 1` (the blessed live point; depth-certified 2026-09-04 — see the MTP CORRECTION below)

MTP draft depth: n_max must be **1** (re-confirmed 2026-08-24: n=2 → 12.71 vs n=1 → 14.05
t/s shallow-bench). Depth>=2 regresses because chained MTP-head drafts drift (acceptance
decays) and each costs ~8 ms.

**MTP CORRECTION (2026-09-03 A/B — supersedes the "zero benefit" note)**: the
"zero measurable benefit" doctrine (2026-05-25, 35B MoE + n≥2 sweeps) does NOT hold for
the 27B dense with n=1. Controlled A/B (same binary/model, shallow 3×64):
ts 22,14 no-MTP 11.79 → +MTP n=1 **16.46 t/s (+40%)**; era ts 27,9 +MTP 14.69. The
8-31 config retarget silently dropped `[spec]` from `~/.vitriol/config` (the exact
flag-drift the fingerprint doctrine warns about) and cost 40% for weeks. Restored;
live unit benches **16.49 t/s** — new operating point. `[spec] type=mtp draft_n_max=1`
is now load-bearing in the live config. **Depth re-certified 2026-09-04**
(see `.opencode/plans/depth-recert-report-2026-09-04.md`): MTP on, q4_0
KV, ts 22,14 — 12.66 / 11.16 / 10.45 t/s at 13K / 26K / 36K filled;
MTP-off arm at 26K = 8.54 → **+31% at depth**. Prefill 179→161 t/s
across the ladder. Wall not re-probed (Aug-24 wall 54,692 on tq3_0/26,10
remains the reference); one transient bare-serve SIGKILL during the cert
reinforces restart-via-unit.
Sweep: `.opencode/plans/qwen38-phase-d-bottleneck-2026-08-19.md`.
Deep-context certification: `.opencode/plans/lull-certification-report-2026-08-24.md`.

Key facts:
- Build must target BOTH archs: `cmake -B build -DCMAKE_CUDA_ARCHITECTURES="61;86"` (native-only default misses sm_86 → "no kernel image" crash).
- KV quantization: `--cache-type-k/v` accepts `tq3_0`/`tq3_1s`/`tq3_4s` (TurboQuant,
  2026-08-24 certified) in addition to f16/q8_0/q4_0/q4_1/iq4_nl. Per-device
  asymmetric overrides exist via `VITRIOL_KV_QUANT[_K|_V]_GPU<d>` (llama-kv-cache.cpp).
- MTP only works if `qwen35_mtp` arch string exists in `src/llama-arch.cpp` (fixed 2026-08-18). Symptom when broken: `unknown override architecture: 'qwen35_mtp'` + `speculative=none`.
- 262K + MTP does NOT fit (Pascal compute buffers). Drop MTP for 262K.
- opencode provider entries `qwen38-mtp`/`qwen38-262k` in `~/.config/opencode/opencode.jsonc`: VERIFIED 2026-09-04 as functional-but-stale — their description strings cite the pre-retarget config (ts 26,10, ctx 49152), but the model string is cosmetic (llama-server serves its loaded model regardless), so the entries work against the live engine. Refresh descriptions at leisure. Hermes-agent uses `~/.hermes/config.yaml` ("Lapis Occultus" → qwen38-master endpoint).

## Context lifecycle (sparse KV + preservation) — 2026-09-03

The daily-driver engine runs **sparse KV eviction with attention-probe
scoring** (lull-kv feature; see
`.opencode/plans/officina-context-lifecycle-2026-09-03.md` — includes the
Aug-31 silent-no-op regression post-mortem).

- **Restart the engine via the unit, never bare `serve`**:
  `systemctl --user restart vitriol-server.service`. The unit is protected
  (oom-shield raises big consumers' oom_score_adj; Restart=always). Bare
  `vitriol serve` from a shell spawns an oom-elevated, kill-first server —
  the launcher now REFUSES this when the unit is active.
- Probe keys (profile `[kv]`, launcher-enforced): `score = probe`,
  `score_every = 16`. Cadence 4 is REJECTED (owner-tested: ejected too
  aggressively). `VITRIOL_KV_FLOOR` (eager sweep) exists but defaults OFF.
- **`vis=on` in the fingerprint means cold restarts**: multimodal
  (`--mmproj`) gates slot save/restore (501); persistence requires a
  text-only engine. The live config dropped mmproj 2026-09-04 for this
  reason — the sidecar probes capability once and stays silent when off.
- **Checkpoint flag name is engine-dependent**: the lull engine wants
  `--checkpoint-every-n-tokens N`; main-tree builds used
  `--checkpoint-min-step N`. The launcher currently emits the lull name.
- **Counter**: `llamacpp:kv_ejected_total` (/metrics) — surfaced as
  `⤓ Nk` in Officina's ctx row. Preservation layer: scratchpad +
  task-state tails re-inject every turn and are carried abroad by the
  TUI bridge; task tail states the eviction contract.
- **Stage 6 standing**: port lull-kv → main (llama-kv-cache divergence +
  `vitriol-kv-probe.*`). Slot save/restore RESOLVED 2026-09-04: the
  endpoints exist in lull (server-context.cpp post_slots) — they were
  gated by the multimodal engine (`--mmproj` → 501 "not supported by
  multimodal"), which also silenced the entire persistence chain (startup
  restore, 300s autosave, proactive bounce). mmproj dropped from the live
  config; warm-resume verified (3/3 SIGKILL cycles restored the slot
  checkpoint in ~300ms). Re-enable vision anytime via `model.mmproj` —
  persistence shuts back off while it's loaded.
