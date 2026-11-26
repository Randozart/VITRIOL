# VITRIOL AI Agent Guidelines

## Kintsugi 224K GPU1 qualification — 2026-11-26 20:59

The reference MTP build on GPU1 recalled the marker at 202,000 filled
tokens and decoded a 256-token Rust sample at 20.66 tok/s. A warm edit
reused 201,562 tokens and decoded at 20.45 tok/s. Both generated functions
passed independent Rust tests. Dual-GPU DFlash2 failed to become ready at
this window. The 65,536-token production profile remains selected, with
its system services still stopped.
[Plan](.opencode/plans/2026-11-26-192713-kintsugi-200k-daily.md).

## Lapis local admission — 2026-11-26 19:20

New Lapis turns use a freshly idle local slot when the estimated context
fits. The router records the actual slot window and fallback reason in
`/route-why`. The live 65,536-token Kintsugi slot served a short Pi RPC
smoke. [Plan](.opencode/plans/2026-11-26-191539-lapis-local-first.md).

## GPU0 decision serving and Kintsugi slots — 2026-11-26 17:47

The local 0.5B decision endpoint runs on GPU0. The 0.5B and 1B candidates
scored 2/8 and 4/8 on synthetic routing cases, so live Caduceus stays
deterministic. Router reports distinguish proposed, reordered and served
lanes. Two no-draft Kintsugi slots on GPU0 decoded 128 tokens each at 13.05
tok/s per slot. The all-GPU DFlash2 variant failed draft allocation.
[Plan and experiment notes](.opencode/plans/2026-11-26-173848-gpu0-decision-kintsugi-slots.md).

## README theme screenshots — 2026-11-26 16:39

The README shows the VITRIOL dashboard in Aethership, the Caduceus pane
in Vitriolum, and Officina in Monokai and Aethership. The release binaries
rendered fixture telemetry and a demo workspace. Officina's images also
resolve from its standalone README source.
[Plan](.opencode/plans/2026-11-26-163533-readme-theme-screenshots.md).

The README now leads with the normal Vitriolum dashboard from a fresh
release launch. [Plan](.opencode/plans/2026-11-26-164642-readme-default-vitriolum.md).

## Caduceus catalog — 2026-11-26 16:03

The Dashboard pane merges the shared Caduceus catalog with live router
status. `/` searches model IDs and `r` runs the canonical catalog sync.
The release displayed all 416 cached IDs and found a model beyond the
initial viewport. [Plan](.opencode/plans/2026-11-26-155827-caduceus-full-catalog.md).

## Caduceus dashboard — 2026-11-26 15:49

`vitriol tui` shows the last Caduceus router request and cooldowns on Dashboard.
Open with `c`, select a route, use `l` for its router events or `u` to unpark.
Officina shows model-specific routing failures inline. Both release frontends
passed terminal fixtures, and the reloaded loopback router serves the new
health response. [Plan](.opencode/plans/2026-11-26-153957-caduceus-dashboard.md).

## Officina model discovery — 2026-11-26 15:09

`/models` and `/providers` use the live Pi catalog and local recent IDs.
Named Caduceus choices use the direct gateway, and Auto-route keeps the
advertised model ID fixed across eligible providers. Pi RPC fixture passed.
[Plan](.opencode/plans/2026-11-26-145644-officina-model-catalog.md).

## Slot log viewer — 2026-11-26 13:47

Dashboard can open the same log viewer as the Logs tab. Slots indexes
engine-tagged output and Events retains polling-derived transitions.
The release passed narrow and wide Aether terminal checks.
[Plan](.opencode/plans/2026-11-26-134128-tui-slot-log-sidebar.md).

## Operations TUI — 2026-11-26 13:30

`vitriol tui` now has Dashboard and Logs. Service actions go through the
system-scope CLI and the profile picker saves through config load. Live
model, GPU placement, and slot fill come from server and process telemetry.
Aether terminal captures verified dark gauge surfaces.
[Plan and evidence](.opencode/plans/2026-11-26-131628-vitriol-tui-two-tabs.md).

## Selectable Kintsugi daily driver — 2026-11-26 12:40

`kintsugi-gpu1` loads through `vitriol config` and uses the existing
systemd entry point. The local config selects it and preserves a rollback
profile. The detached profile passed GPU1 chat and isolated autosave.
Systemd denied the production restart without administrator authentication.
[Plan and evidence](.opencode/plans/2026-11-26-123509-selectable-kintsugi-gpu1.md).

## Provenance inventory — 2026-11-26 11:32

The README credits the main upstreams and links to the technique inventory.
The registry records Kintsugi and EXL3 reuse and the verified Pi example
origin. The little-coder intermediary attribution remains open.
[Plan](.opencode/plans/2026-11-26-112738-provenance-inventory.md).

## README cleanup — 2026-11-26 11:25

The author confirmed the opening and “Why VITRIOL?” as their own wording.
The README keeps that text verbatim and links to the detailed docs. The CUDA
example passed a launcher dry run. [Plan](.opencode/plans/2026-11-26-112416-readme-cleanup.md).

## Kintsugi system launch — 2026-11-26 11:08

GPU1 daily-driver overrides are prepared for `vitriol-server.service` and
its autosave sidecar, with isolated state under `/var/lib/vitriol-kintsugi`.
The launcher passed GPU1 placement, chat, metrics and slot save/restore
on a temporary port. Administrator authentication controls activation.
[Install, verification and rollback](.opencode/plans/2026-11-26-110624-kintsugi-gpu1-daily-driver.md).

## Z440 Kintsugi and Inkling experiments — 2026-11-26 11:02

Fixed n7 led the local Kintsugi coding sweep at 39.45 tok/s median. Private
full draft-cache retention recovered a 33112-token checkpoint follow-up from
8.99 to 31.28 tok/s at a 332 MiB checkpoint. Separate GPU0 drafting worked with
a local projection and trailed the single-card control. GPU0 served a separate
small model alongside Kintsugi. Current Inkling IQ4_NL runs on both cards with
split 1,3, ctx 32768 and ubatch 32, including 2335 filled-token recall.
[Experiments and Inkling launch command](.opencode/plans/2026-11-26-104921-z440-kintsugi-inkling-experiments.md).

## Kintsugi readiness update — 2026-11-26 10:33

Run Kintsugi on the quiet RTX 3060 with `scripts/serve-kintsugi.sh`. The 65536-token
window uses DFlash2 n7, resident recurrent state, a 32768-cell GPU KV tier and
full draft cache retention. Measured 68.5 tok/s repeat, 42.3 code and 46.1 after a
fresh 33116-token prefill. A suffix-replacing follow-up restored a checkpoint
and fell back to 9.0 tok/s with target prefix reuse. Replay stays gated off after
a post-rejection checkpoint save hit a bounds failure. [Plan and placement evidence](.opencode/plans/2026-11-26-101857-kintsugi-replay-z440.md).

## CPU buffer research update — 2026-11-19 09:26

Private no-host/none selects CPU+CPU_REPACK weights,12.43GBanonymous/Swap0 survives
warmup. All5STTtexts/WAVs exact/all5playback gates pass, but CPU browser6.83–6.86s
vsreversehost6.12s: no promotion. Native repack23.6–23.9t/s vs20.7–21.5host;
no-repack20.95 isolates kernel benefit. Currentvoice T3tokens1.309s/HiFT.583s/
flow.406s(estimator.196 within),4profilePCMexact; nextsplitT3prefill/decode, no
newtraining. Originalv9/voice/20Bcommand/pins restored, generator intentionally
restarted/newPID, thread0–15/Performance/extraction/clientACKready verified09:22.
No exactPID/cachewarmth claim or runtime/homeprofile change. Avatar owns code/
evidence; .opencode/plans/ptl-cpu-buffers-2026-11-19.md.0autoemployeehours.

## Memory research update — 2026-11-18 16:30

Private late-prime placement passes matched speech/browser gates; first audio
5.34→4.48s, majorfaults14441→1233 in one paired observation. GPU-host allocation
must be separated from file cache/RSS; anonymous resident attempt unqualified,
no physical-residency claim. Publicv9/currentvoice/20B restored within the approved
window; no runtime/profile/home-PC changes. See .opencode/plans/ptl-memory-final-
prime-2026-11-18.md; Avatar owns tested private code/evidence.0auto employeehours.

## Anchored summary — 2026-11-18 14:32

Optional generic client-prefix primer qualified against the current PTL20B and
private full-avatar playback. Native cache loss/reprime/resend/history/cancel and
fresh-generator restart pass; candidate server3/3 and both browser delivery gates
pass,40msmaxdrift/0skips. Three exported replies/WAVs exactly match controls.
No warm-browser speedup; memory pressure/residency remains a latency limit. Exact
livev9/original20B restored; no home-PC/default-profile change. Invariants and
configuration in .opencode/plans/client-prefix-primer-2026-11-18.md, CLI usage in
docs/client-prefix-primer.md. Nine primer tests/type/lint, cache-store15/native
ownership checks pass. Owned-cache atime fix carried, original dirty vendor
license preserved. Opt-in code integrated with current upstream main; no new
Officina work or binary install. Intel/WBSO evidence in Avatar research,0auto hours.

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

3. **Always kill stale servers** with `killall -9 vitriol-server` (legacy name `llama-server` accepted during the monorepo transition) before starting a new one.

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
2. **Officina rework ⇒ updated binary**: any change to the Rust TUI (`officina/tui/`) that ships new operator-visible behaviour MUST be followed by a rebuild + reinstall before it is considered done — `cargo build --release` in `officina/tui/`, then `cp target/release/officina ~/.local/bin/officina`. The operator runs `~/.local/bin/officina`, not a dev build; a commit that is never installed is invisible to them (2026-11-11: the ctx-bar row was committed but not visible because the installed binary predated it). Gate: `cargo test` + `cargo fmt --check` green, then rebuild + copy, then confirm the row/feature actually appears in the running TUI.

## Code Conventions

- **`main` is the canonical branch of this monorepo.** The engine is no
  longer a submodule: since 2026-10-23 the llama.cpp fork lives in-tree at
  `engine/` (imported at fork commit `30b128504`, upstream sync FROZEN -
  see `docs/ENGINE_PROVENANCE.md`). The old fork repo
  (`Randozart/llama.cpp`, branches `main`/`vitriol`/`master`) is an
  archived read-only record; do not commit there. Engine work happens on
  outer branches touching `engine/`. The VITRIOL predictor is in
  `engine/ggml/src/ggml-cuda/vitriol-cuda-integration.cpp`; the server
  context checkpoint logic is in `engine/tools/server/server-context.cpp`;
  the server binary ships as `vitriol-server` (CMake target
  `llama-server`, compat symlink in `engine/build/bin/`).
- All VITRIOL env vars are prefixed with `VITRIOL_`.
- **Invariants-in-plans (2026-11-11)**: every plan in `.opencode/plans/` that ships new behaviour carries an `## Invariants` section: the numbered properties the change MUST preserve (e.g. `I1: 0 peers ⇒ decisions byte-identical to today`), each tied to the test that pins it. Rules: (1) an invariant is a *property*, not a test name — it must survive refactors of the code that implements it; (2) the load-bearing one is the *parity* invariant — old behaviour with no new inputs must be unchanged, pinned by a golden matrix; (3) a plan that cannot name its invariants is not ready for implementation; (4) when code refactors silently change an invariant, the plan's invariant list is the review checklist. Precedent: `multi-session-routing-2026-11-10.md` (I1–I6, pinned by `overflow-peer.test.ts` + `peer-policy.test.ts`).
- **Sidebar-refresh contract (2026-09-04)**: any extension whose TOOL mutates sidebar-visible state (task list, scratchpad — anything surfaced by session-panel's `registerSidebarSection`) MUST call `requestSidebarUpdate()` (from `_shared/sidebar.ts`) after persisting. The `emitHarnessEvent` records are a passive audit log — they never signal the sidebar. Regression test: `.pi/extensions/task-state/sidebar-refresh.test.ts`.
- **Cross-extension state contract (2026-09-06) — jiti isolation**: pi loads EVERY `-e` extension through a **separate jiti instance** with `moduleCache: false` (pi `loader.js:325-332`). Module-level state does **not** cross extension boundaries — three silent bugs came from exactly this (stale sidebar listener Set; task/scratchpad session-file paths updated in one instance while session-panel's imported getters read another; three parallel engine pollers). Rule: any module-level mutable that crosses an extension boundary MUST be a `globalThis`-backed singleton (pattern: `__officina*State` — see `_shared/sidebar.ts`, `_shared/engine.ts`, task-state, scratchpad, skill-inject, knowledge-inject). Guard: `.pi/extensions/_shared/jiti-isolation.test.ts` loads two jiti instances and asserts the contracts — new cross-extension state gets a test there.

- **Caduceus gateway + Lapis default (2026-11-10)**: Officina's local
  gateway integration is named **Caduceus**; the CLI is `vitriol caduceus`
  (provider id `caduceus`, model ids `caduceus/...`). The resilient default
  model is the virtual `officina/lapis` ("Lapis Occultus"): local-first,
  free/cloud overflow, and cloud failover when VITRIOL is down. Vendor
  internals keep their upstream names (`~/.vitriol/vendor/omniroute`,
  `~/.omniroute`, `OMNIROUTE_HEADLESS`, `X-OmniRoute-*`) and OmniRoute MIT
  attribution lives in `NOTICE` + `officina/THIRD_PARTY_NOTICES.md`.

- **Officina frontends (2026-11-10) — Rust is canonical**: the operator runs
  the Rust TUI binary `officina` (installed at `~/.local/bin/officina`, source
  `officina/tui/`). The JS frontend (`officina/officina.mjs` +
  `officina/runtime/patched/*` + the pi bundle hooks) is **DEPRECATED legacy**
  — do not add new feature work there unless the operator explicitly asks.
  The Rust TUI spawns `pi --mode rpc`, so the shared `.pi/extensions/*`
  surface is **not** deprecated: sidebar, tools, scratchpad/tasks, engine
  telemetry, bash titles, etc. still live there and are load-bearing for both.
  /3rd-party pi TUI patches are legacy-only.

- **Officina publishing (2026-11-11) — snapshots, not a split**: the
  monorepo is CANONICAL; github.com/Randozart/officina receives exact
  snapshot tags for the "Officina without VITRIOL" audience. Release
  ONLY via `scripts/publish-officina.sh` (gates + secrets canary +
  tracked-tree snapshot; tag = `officina-v<officina/package.json
  version>`, bump both package.json and tui/Cargo.toml in lockstep).
  NEVER hand-edit or force-push the public repo outside the script; the
  snapshot tree is `HEAD:officina` — anything committed under `officina/`
  WILL ship publicly (the secrets canary is the last line, not the
  first). Subtree `split` is retired: the path spans two eras and the
  split cache resurrects old-era files. Officina-specific docs live in
  `officina/docs/`; assets the TUI compiles in live in `officina/assets/`
  (nothing under `officina/` may reference paths above the subtree).

## Prior Art Rule (2026-11-25 — read the reference before measuring your own build)

When a reference implementation of the same thing exists on disk or upstream, **diff it before you
profile your own build**. Measured numbers are not a substitute for reading the other
implementation, and profiling cannot tell you that you are comparing two different generations of a
component.

Incident: for three sessions we tried to explain why `vitriol-server` ran Kintsugi-27B at 8.80 t/s
where `kintsugi-engine` ran the same model on the same host at 14.55 (and 72.8 vs 17.5 with the
DFlash2 drafter). The reference engine's full source sat in `~/kintsugi/engine-src/`. Instead of
diffing it, we instrumented our own binary: a per-node GPU op histogram, a speculative-phase timer,
draft-depth sweeps, per-row cost attribution, a graph dump. Every number was true; four hypotheses
were refuted in a row; the cause was never found. One `diff` of the file inventories plus one diff
of the hot file found it in a single pass:

- their fork carries `gated-delta-net-mma.cu`, `head2.cu`, `mirai-s.cu`, `wddm.cu`;
- `gated_delta_net.cu` differs by **1,425 lines**: their kernel is templated
  `<S_v, KDA, keep_rs_t, RAW, G_PRECOMPUTED>` where ours stops at `keep_rs_t`, with three
  precompute kernels (`gdn_precompute_exp`, `gdn_gather_state`, `gdn_activate_raw_gates`) that hoist
  per-token gate work out of the sequential scan.

Ours was not a buggy copy of theirs; it was **an older generation**, which no amount of profiling of
our own binary could reveal.

Mandatory order when explaining a difference from a reference implementation:

1. **Inventory diff** — which files exist only there, and which shared files diverge most?
2. **Hot-file diff** — read the diverged file in the path the difference lives in.
3. **Measure** — instrument only what the diff cannot explain.

Corollary: if prior art is right there, adopting it beats re-deriving it. Our own re-derivation cost
three sessions and produced four refuted hypotheses; the port is bounded and provable.

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

## Sweep Controller (Python) — placement only

- **`libvitriol/sweep_controller.py`** — automated benchmark sweep via HTTP POST `/completion`
- **Scope (2026-11-19):** this is the *placement* sweeper (pin/MTP/ubatch/ts grid -> CSV) and the
  `vitriol-tui` dashboard shells out to it. Token-density comparisons (grammar vs prose,
  speculative-decoding variants, recall guards, JSON artifacts) live in
  `density-bench sweep --engine <vitriol-server> --variant "label=flags" ...`, which also runs one
  engine per arm but measures prompt shapes and records a fingerprint per run. Do not delete either
  to "consolidate" without moving the TUI path first.
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

VITRIOL (monorepo, including the in-tree `engine/` fork) is dual-licensed
under the **Apache License 2.0 OR the MIT License** (SPDX:
`Apache-2.0 OR MIT`; see `LICENSE` and `LICENSE-MIT`; upstream MIT text
retained at `engine/LICENSE`, attribution in `NOTICE`). Downstream
recipients choose either term.
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

MTP `draft_n_max` note: **n=2 is the live daily driver** (A/B 2026-11-11 at
~70k filled depth: mtp2 acceptance 0.953 vs mtp3 0.735 — the 3rd draft token
is net negative on the BuffedMod-Q6K build; see
`.opencode/plans/mtp2-vs-mtp3-ab-2026-11-11.md`). Historical 2026-09-28 note:
n=3 (+17% at 42.7k vs n=1; n=2 +12%) — depth/build-sensitive, re-certify
after any model or MTP-engine change.
n=4 now also loads (ts 27,21, dev1 440 MiB) but is not faster (25.3 vs 25.7
t/s) so it is not used. The old "n_max MUST be 1" rule was Q3_K_M/Pascal-era.
**VRAM is NOT KV-bound**: KV at q4_0 is only ~19.1 KiB/token (hybrid arch, ~17
of 65 layers attend) = ~3.9 GiB at 208K; pressure is the ~11.5 GiB weights +
~2.3 GiB compute/misc + external tenants. Draft KV must stay **f16**: q4_0
draft KV breaks flash-attn for the MTP layer (pp buffers ~1.2 GiB -> load OOM).

Saved VITRIOL profiles (load with `vitriol config load <name>`):

| profile | ctx | ts | MTP | model | notes |
|---|---|---|---|---|---|
| `qwen38-buffedmod-262k-cache8-mtp2` | 262144 | 27,21 | n=2 | BuffedMod Q6K | **live daily driver** (2026-11-11 A/B); requires lares off |
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
