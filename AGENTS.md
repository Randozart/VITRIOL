# VITRIOL AI Agent Guidelines

## Code Style

**`docs/cpp26-conventions.md` is the binding style guide** — read it before
writing/editing VITRIOL C++ host code. Core rules: C++26 per-file on vitriol
`.cpp` (g++ 16.2), C++20 ceiling on nvcc-parsed headers; contracts
(`pre`/`post`/`contract_assert`) on public host functions with
`-fcontract-evaluation-semantic=ignore` (zero runtime cost — never ship
`enforce` on hot paths); typed pointers/`std::span`/`std::mdspan`; no
pointer-as-integer keys, no C-casts, no raw alloc w/o RAII; `std::expected`
for host errors; `[[assume]]` only after a contract proves the invariant
(state once via the guide's `VITRIOL_ASSUME_FROM_CONTRACT(cond)` helper — an
ignored contract is NOT an optimizer assumption); hot inner-loop micro-APIs
return scalars, not `std::expected` (SYSV ABI: keep `sizeof(expected) <= 16`);
`std::hash<const void*>` does not mix bits — bit-mix for power-of-2 flat maps.
`main()`-bearing harnesses (`vitriol_ce_d2b.cpp`, `vitriol_ce_spike.cpp`)
must stay OUT of the ggml-cuda source GLOB.

## Testing Protocol

0. **KV residency tiers (2026-09-27, inner commit `15686347f`)**: engine KV flows VRAM -> host RAM prompt cache (`cache_ram = 12288` — raised from 2048; measured slot states 2.5-7 GiB) -> Optane disk mirror under `--slot-save-path` (async spill on swap, boot reload). Restarts no longer re-prefill from zero: same-context next turn = delta-only (certified 745/755 cached, 0.5s). Config writes to these keys MUST fingerprint-diff (`cram=` rides the fingerprint). Known: state serialization bloat under `--kv-unified` (~60-230x theoretical, pre-existing); explicit `id_slot` requests bypass the prompt cache.

1. **Residency rule (2026-08-24, supersedes "always DMA")**: stream/DMA offloading ONLY when weights exceed combined VRAM (e.g. 35B-class). For resident-capable quants (≤ ~20 GiB combined) the default is `VITRIOL_MODE=off` — weights fully VRAM-resident; streaming a fitting model starves GPUs on DDR3/PCIe expert fetches and is a pessimization. Streaming in an experiment record must be justified.

2. **CAP_IPC_LOCK is optional on this host**: CUDA pinned allocations do not count against RLIMIT_MEMLOCK; all 2026-08-24 certifications ran uncapped. `scripts/build-llama-server.sh` reapplies caps best-effort after builds; `sudo vitriol setup` remains for hosts where mlock fails. Never pass `--ctx-checkpoints 0` (heap corruption) or `--cache-ram 0` (no readiness).

3. **Always kill stale servers** with `killall -9 llama-server` before starting a new one.

   **OOM discipline (2026-09-04)**: the engine's host-memory reservoir
   made it the system's OOM-kill kingpin (5 kills in one day) — a
   zram-only box (no disk swap) + swappiness 150 swaps the engine's cold
   host pages; swapped pages dominate OOM badness. Trimmed: `--ctx-checkpoints
   4` (RAM ring of in-slot KV copies — 32 default is multi-GB; 0 FORBIDDEN),
   `--cache-ram 256`, `--cache-idle-slots` off (host KV backup). VmSwap 1.5GB -> 51MB. `ckpts=` rides the fingerprint. Hardened
   2026-09-04: 16G /swapfile (btrfs NOCOW — fallocate holes make swapon
   fail) + SYSTEM-scope units — the engine's `oom_score_adj` is now a real
   -500 (last OOM victim), verified. Report + root script:
   `.opencode/plans/oom-reservoir-trim-2026-09-04.md`,
   `scripts/vitriol-oom-hardening.sh` (re-run idempotently as root).

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

- This is a fork of `ggml-org/llama.cpp` with VITRIOL modifications. **`main` is the canonical daily-driver branch in BOTH repos** (consolidated 2026-09-01 — see `.opencode/plans/branch-consolidation-2026-09-01.md`). Outer repo: all feature work lives on `main` (was `officina`). Inner fork (`llama.cpp/`): `main` is the live line ported onto ggml-org upstream (was `vitriol-ku`); `vitriol` is the frozen pre-port archive, `master` tracks the fork's published state. **lull-kv (probe-scored sparse KV) was ported from the `VITRIOL-lull` worktree to `main` 2026-09-22** (inner commits 6f9316128 + d86cdbb59 + fa7a75f4e) — `VITRIOL-lull` is now historical; do not branch feature work there.
- The VITRIOL predictor is in `ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`.
- The server context checkpoint logic is in `tools/server/server-context.cpp`.
- All VITRIOL env vars are prefixed with `VITRIOL_`.
- **Sidebar-refresh contract (2026-09-04)**: any extension whose TOOL mutates sidebar-visible state (task list, scratchpad — anything surfaced by session-panel's `registerSidebarSection`) MUST call `requestSidebarUpdate()` (from `_shared/sidebar.ts`) after persisting. The `emitHarnessEvent` records are a passive audit log — they never signal the sidebar. Regression test: `.pi/extensions/task-state/sidebar-refresh.test.ts`.
- **Cross-extension state contract (2026-09-06) — jiti isolation**: pi loads EVERY `-e` extension through a **separate jiti instance** with `moduleCache: false` (pi `loader.js:325-332`). Module-level state does **not** cross extension boundaries — three silent bugs came from exactly this (stale sidebar listener Set; task/scratchpad session-file paths updated in one instance while session-panel's imported getters read another; three parallel engine pollers). Rule: any module-level mutable that crosses an extension boundary MUST be a `globalThis`-backed singleton (pattern: `__officina*State` — see `_shared/sidebar.ts`, `_shared/engine.ts`, task-state, scratchpad, skill-inject, knowledge-inject). Guard: `.pi/extensions/_shared/jiti-isolation.test.ts` loads two jiti instances and asserts the contracts — new cross-extension state gets a test there.

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

## Cooperative-VRAM Governor (Rust, 2026-10-10)

- **`vitriol-governor/`** — control-plane daemon: NVML 1 s poll; escalation
  ladder Full -> Relieved -> Resized -> Yielded under sustained foreign-tenant
  VRAM pressure; walks back on sustained recovery. RELIEVE = `POST /kv-policy`
  (runtime KV eager-sweep floor + sweep; engine atomics replace boot-only
  `VITRIOL_KV_FLOOR`), RESIZE = `POST /ctx/resize` (live KV window swap,
  weights resident, slots spooled -> warm restore; opt-in via
  `[server] ctx_resize = 1` -> `VITRIOL_CTX_RESIZE=1`, fingerprint `cxr=`),
  YIELD = `scripts/vitriol config load qwen38-buffedmod-208k` + unit restart
  (warm KV restore) + engine `oom_score_adj` -500 -> 0 while yielded.
- Foreign-tenant detection needs a 768 MiB per-process floor: NVML compute
  lists are full of desktop residents (kwin/electron/Discord) that must not
  trigger yields. Engine PID set = unit MainPID + `/proc` comm scan.
- Engine endpoints: `POST /kv-policy {floor,protect,sweep}`,
  `POST /ctx/resize {n_ctx}` (gated by `VITRIOL_CTX_RESIZE=1`); task-queue
  serialized (defer while decoding). `n_ctx` is NOT in the prompt-cache
  build_key (position-addressed states fit any window; per-state fit gate at
  load) so mirror states survive resize.
- LIVE since 2026-10-11 (restart-day canary passed,
  `.opencode/plans/restart-day-report-2026-10-11.md`): governor enabled,
  sidecar unit active (restores /scratch/kvtier/slot0.bin at boot),
  `VITRIOL_KV_DISK_PRUNE=0` pinned via drop-in. Config
  `~/.vitriol/governor.toml` (absolute paths - the unit has no HOME).
  Behavior contract: crash-resume warms when the client continues the
  saved conversation; pure prompt growth appends delta-only; only state
  OVERSHOOT (restored state past the match) is rollback-bounded by
  n_rs_seq (fixed 2026-10-12, inner `f5fbc4e44` - the old growth-based
  check full-re-prefilled every main<->subagent switch, 127-164k tokens
  at a time; see `.opencode/plans/restored-state-overshoot-fix-2026-10-12.md`). /metrics accumulated counters: FIXED 2026-10-12
  (inner `953784096`) — the cached copy was only refreshed on the
  sleeping transition (never fires on always-on servers); a 1 Hz
  post-update_slots hook refreshes it now. All counters verified live.
- `vitriol-gpu-rs/` — cuda-oxide spike crate (toolchain proven, no engine
  kernels; see `.opencode/plans/cuda-oxide-spike-verdict-2026-10-10.md`).

## Qwen3.8-27B Dual-GPU Config (RTX 3060 + GTX 1070 Ti)

**LIVE CONFIG (single-session, blessed 2026-09-28, ts 27,21)**:
`~/.vitriol/config` runs the 2026-09-28 single-session retarget — model
`~/Models/Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf` (tooltd; ISTA
RCO IQ3_S + `output.weight` Q4_K->Q6_K: -27.2% PPL-error, -17.5% median KLD),
**c 262144 (native)**, `parallel=1`, MTP `draft_n_max=3`, q4_0/q4_0 KV, sparse
probe, f16 draft KV. Report: `.opencode/plans/single-buffedmod-2026-09-28.md`.
The duo (2 slots / 393K) is SUPERSEDED/abandoned (`slot-scaling-2026-09-28.md`).
Historical ISTA `Qwen3.8-27B-GSQ-RCO-IQ3_S-mtp.gguf` (BLESSED 2026-09-22,
`gsq-rco-vendor-qual-2026-09-22.md`) stays on disk for rollback; lines below
describing MTP n=1 / ts 27,9 / 24,12 are historical.

**PRECONDITION (2026-09-28)**: native 262K + MTP n=3 only fits because
`lares-vision.service` (~1.85 GiB dev0) is **stopped and disabled** (it is
being moved off-box), which makes the asymmetric `tensor_split = 27,21` viable
and unloads dev1 (the engine lands ~3 GiB more there: MTP draft ctx + KV). If
any external GPU tenant returns, load `qwen38-buffedmod-208k` (ctx 212992,
ts 24,24) instead.

MTP `draft_n_max` note (2026-09-28): **n=3** (+17% at 42.7k vs n=1; n=2 +12%).
n=4 now also loads (ts 27,21, dev1 440 MiB) but is not faster (25.3 vs 25.7
t/s) so it is not used. The old "n_max MUST be 1" rule was Q3_K_M/Pascal-era.
**VRAM is NOT KV-bound**: KV at q4_0 is only ~19.1 KiB/token (hybrid arch, ~17
of 65 layers attend) = ~3.9 GiB at 208K; pressure is the ~11.5 GiB weights +
~2.3 GiB compute/misc + external tenants. Draft KV must stay **f16**: q4_0
draft KV breaks flash-attn for the MTP layer (pp buffers ~1.2 GiB -> load OOM).

Saved VITRIOL profiles (load with `vitriol config load <name>`):

| profile | ctx | ts | MTP | model | notes |
|---|---|---|---|---|---|
| `qwen38-buffedmod-262k` | 262144 | 27,21 | n=3 | BuffedMod Q6K | **live daily driver**; requires lares off; dev1 ~500 MiB free |
| `qwen38-buffedmod-208k` | 212992 | 24,24 | n=3 | BuffedMod Q6K | fallback when external GPU tenants are resident; dev1 ~230 MiB |
| `qwen38-mtp-131k` | 49152 | 27,9 | n=1 | ISTA IQ3_S | SUPERSEDED; (26,10 on merged base), q4k/q4v |
| `qwen38-262k` | 262144 | 24,12 | off | ISTA IQ3_S | max native ctx, q4k/q4v |

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

Recommended working config (certified): `-ngl 99 -ts 24,12 --main-gpu 0 -ub 64
--cache-type-k tq3_0 --cache-type-v tq3_0` (TurboQuant KV = 3.5 bpw, −22% vs q4_0;
per-device overrides via `VITRIOL_KV_QUANT[_K|_V]_GPU<d>`). Add
`--spec-type mtp --spec-draft-n-max 1` for the 49k-window profile.
Master deep-context profile: `vitriol config load qwen38-master`
(IQ3_S, c131072 tq3_0, sparse+probe; export VITRIOL_KV_SCORE=probe and
VITRIOL_POOL_RESET=1 first — certified 96,836 tok @ 11.32 t/s).

Required flags (all wired into `scripts/vitriol` config now):
`-ngl 99 -ts 24,12 --main-gpu 0 -ub 64 --cache-type-k q4_0 --cache-type-v q4_0 --spec-type mtp --spec-draft-n-max 1` (the blessed live point; depth-certified 2026-09-04 under ts 22,14 — see the MTP CORRECTION below)

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

**Identical-resend hazard (fixed 2026-10-12, inner `953784096` + `9b11ccb27`)**:
a client retry after a slow cold-start response sends an IDENTICAL prompt —
full prefix-cache hit. The `[TAG_PROMPT_LOGITS]` guard then rewinds one
token; on hybrid GDN memories a rewind deeper than the `n_rs_seq` ring (3)
used to SIGABRT (`common_context_seq_rm`) and wedge the server into a
watchdog bounce loop. Now: removal refusal falls back to a full-sequence
reset (honest re-prefill), and restored-state divergence tries the
checkpoint ring before forcing a re-process. Regression signature to
remember: health-deaf + SIGABRT ~60 s after a cache-hit request, gdb
frames from `ggml_print_backtrace` in `vitriol_gen.log`. See
`.opencode/plans/dashboard-o1o2o3-results-2026-10-12.md`.

- **Restart the engine via the unit, never bare `serve`**:
  `systemctl restart vitriol-server.service` (**SYSTEM scope since
  2026-09-04** — the engine runs under `/etc/systemd/system/` with a REAL
  `OOMScoreAdjust=-500`, effective `oom_score_adj` -500, i.e. the last OOM
  victim; the legacy user units are retired, files kept as
  `*.service.disabled`). The launcher REFUSES bare `vitriol serve` while
  either unit file is present and the service is active. A polkit rule
  lets randozart manage exactly the two vitriol units.
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
- **Stage 6 RESOLVED 2026-09-22**: lull-kv ported to main (inner commit
  `6f9316128` — vitriol-kv-probe + evict_sparse + eager floor + counter +
  prometheus gauge; verified live `VITRIOL_KV_SCORE: probe active`). Slot save/restore RESOLVED 2026-09-04: the
  endpoints exist in lull (server-context.cpp post_slots) — they were
  gated by the multimodal engine (`--mmproj` → 501 "not supported by
  multimodal"), which also silenced the entire persistence chain (startup
  restore, 300s autosave, proactive bounce). mmproj dropped from the live
  config; warm-resume verified (3/3 SIGKILL cycles restored the slot
  checkpoint in ~300ms). Re-enable vision anytime via `model.mmproj` —
  persistence shuts back off while it's loaded.
