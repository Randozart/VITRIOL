# LAPIS REPORT — Qwen3.8-27B on Dual-GPU VITRIOL: Investigation, Certification & Production

> **Dates:** 2026-08-24 → 2026-08-25
> **Branches:** outer `main` @ `6b67686`, submodule `vitriol-mellum2` @ merge `054fae712` + `e013c26d0`/`83619cf09`
> **Hardware:** RTX 3060 12 GiB (sm_86) + GTX 1070 Ti 8 GiB (sm_61), i7-3770 (AVX, no AVX2), dual-channel DDR3
> **Status:** PRODUCTION DEPLOYED — "Lapis Occultus" serving hermes-agent from the certified master config

---

## 1. Executive summary

Started as an exploration of GPU-lull utilization for KV-cache management (LULL),
ended as a full-stack audit that **corrected three long-standing misconceptions**,
**shipped a working attention-scoring substrate**, **certified real (depth-filled)
operating envelopes**, and **deployed a production endpoint** the maintainer confirms
works with their agent stack.

Final production state: **Qwen3.8-27B UD-IQ3_S, q4_0 KV, ts 24,12, ub64, 131072
window, weights fully VRAM-resident (`VITRIOL_MODE=off`)** — certified to
**92,642 filled tokens**, decode ~9–12.4 t/s, serving hermes-agent as
"Lapis Occultus".

## 2. Chronology (compressed)

| When | What |
|---|---|
| Aug 24 AM | LULL plan written; worktree isolation (`lull-kv`); Phase-0 split profiler shipped |
| Aug 24 midday | Heap-corruption hunt (many dead ends; see §4.1); profiler hardened through real failures |
| Aug 24 PM | Corruption root-caused (**our flags**, not base); deep-context unblocked; Phases 1–2 implemented (probe scoring + score-driven eviction) |
| Aug 24 eve | Merged to `main`; certification attempts begin; quant/KV matrix explored |
| Aug 25 | Golden-era reconciliation; speed-investigation reversal (tq3_0 verdict); master config finalized; production deployed |

## 3. Discoveries & root causes (each independently verified)

### 3.1 The "131k myth"
Historical "~14 t/s @ 131k" conflated **allocated window** with **filled depth**.
Sweep benches used a 13-token prompt; the saved profile's own meta documented
*"131K OOMs ~45-61K tokens on this dual-GPU pair"* with actual window ctx=49152.
Reproduction: **14.05 t/s** shallow-bench (ts 26,10, MTP n1) — matches history;
no regression ever existed. Rule adopted: **window ≠ depth** (protocol §5).

### 3.2 Heap corruption was our own flag
`--ctx-checkpoints 0` corrupts the heap (checkpoint code mishandles disabled=0);
`--cache-ram 0` separately blocks server readiness. Both were LULL-bench
hardening flags, which is why crashes shadowed every commit/model combination
investigated. Q3_K_M's embedded-MTP sibling exonerated as primary cause.
**Never pass zeros** — bounded values (`--ctx-checkpoints 4
--checkpoint-every-n-tokens 8192`) are mandatory.

### 3.3 TurboQuant KV exists — but is not free
Fork supports `tq3_0` KV (3.5 bpw, −22% static bytes vs q4_0) plus per-device
overrides (`VITRIOL_KV_QUANT[_K|_V]_GPU<d>`). However **tq3_0 lacks MMQ kernels**:
in resident (non-stream) mode its attention path costs **−40% decode**
(6.86 vs 11.91 t/s, controlled A/B). Verdict: q4_0 KV for daily driving;
tq3_0 only where static bytes matter more than speed (max-depth + stream).

### 3.4 Streaming exonerated; residency rule adopted
The "always DMA" protocol dated from the 35B non-fitting era. For 27B quants
that fit in 20 GiB, streaming neither helps nor hurts decode materially at
moderate depth — but **resident mode removes DDR3 expert-fetch latency entirely**
and was the maintainer's preferred operating point. Protocol §1 rewritten:
stream only when weights exceed combined VRAM.

### 3.5 Decode plateau is architectural
Every sane configuration converges to **~11.7–12.9 t/s** decode regardless of
split, mode, quant, or fill depth (CUDA graphs confirmed engaged; util curves
flat). The Aug-21 "~20 t/s" record predates verifiable methodology and could
not be reproduced under any flag combination; treated as unexplained historical
outlier until re-measured with today's provenance tooling.

### 3.6 Depth wall mechanism (partially solved)
dev0 VRAM creeps **~23 KiB/token** during prefill, linear and independent of
KV bits. Two components identified:
1. VMM-pool LIFO drift across growing n_kv shapes → fixed by
   **`VITRIOL_POOL_RESET=1`** (recovers ~20% depth);
2. residual growth from per-rebuild input buffers allocated *outside* the
   compute pool (kq masks et al.) — OPEN, next surgical target.
Pascal-side kernel launch failures at n_kv ≥ ~100k are secondary symptoms.

### 3.7 Context reuse for GDN-hybrid models
Recurrent layers cannot mid-history restore (upstream PR #13194). The fork's
checkpoint system mitigates within sessions. `frozen_prompt` is a **shim-era**
feature — no-op without the memory shim; hermes-direct deployments rely on
native prompt-cache + checkpoints. Reuse auditor shipped; baseline HEALTHY;
real-traffic verdict accumulates in `/tmp/opencode/vitriol_gen.log`.

### 3.8 Misc confirmed
- MTP: depth must be 1; zero end-to-end benefit; separate-head MTP on IQ3_S
  *degrades* throughput (8.45 and falling).
- CAP_IPC_LOCK optional here (CUDA pinned allocs bypass RLIMIT_MEMLOCK);
  checks downgraded to warnings; build script reapplies best-effort.
- TUI TPS gauge parses `vitriol_gen.log` heartbeats — manual terminal launches
  starve it; launcher-detached relaunch restores.

## 4. False leads & negative results (institutional memory)

| Lead | Outcome |
|---|---|
| "Always DMA" protocol | Wrong for fitting models — rescinded |
| Streaming caused slowness | Exonerated by controlled matrix |
| Tensor split tax on decode | Neutral (single-GPU ≈ split within noise) |
| git-bisect for corruption | Invalid: "good" anchor assumed clean-tree builds that were never committed |
| GGML_CUDA_NO_VMM=1 | Negative (59k, launch-fail mode instead of OOM) |
| Compute-pool reset | Partial positive (+20% depth); residual grower elsewhere |
| ub32 | cublas invalid-parameter crash ~53k — avoid |
| MTP via separate head on IQ3_S | Degrades (8.45, decaying rounds) |

## 5. Certified envelope (filled-token discipline)

All: single-shot mega-prefill, 3×64 greedy decode, bounded checkpoints,
fingerprints recorded.

| # | model | KV | ts | window | FILLED | decode t/s |
|---|---|---|---|---|---|---|
| 1 | UD-IQ2_S | q4_0 | none | 32768 | 30,190 | 11.74 |
| 2 | UD-IQ3_S | q4_0 | 24,12 | 98304 | **92,642** | 7.8–12.4 |
| 3 | UD-IQ3_S | tq3_0 | 26,10 | 131072 | **96,836** | 11.32 (stream) |
| 4 | Q3_K_M | tq3_0 | 26,10 | 65536 | 54,692 | 9.21 |
| 5 | Q3_K_M | q4_0 | 24,12 | 98304 | **92,642** | **9.22** |
| 6 | Q3_K_M shallow | q4_0 | 26,10 | 49152 | — | 14.05 (MTP n1) |

Rows 2 and 5 are the production-relevant certifications.

## 6. Final master configuration (deployed)

```bash
export VITRIOL_KV_SCORE=probe     # LULL attention scoring
export VITRIOL_POOL_RESET=1       # compute-pool rewind
vitriol config load qwen38-master
vitriol serve --detach            # persistent logging -> vitriol_gen.log
```

Profile (`profiles/qwen38-master`, synced to `~/.vitriol/profiles/`):
IQ3_S · `ts 24,12` · `c 131072` · `ub 64` · `quant_mode=q4_0/q4_0` ·
`mode=off` · `checkpoint_every_n_tokens=8192` · `kv.mode=sparse` ·
`engine.mode=vitriol-dma` (dormant when MODE=off) · server alias
**Lapis Occultus**.

Hermes-agent (`~/.hermes/config.yaml`): default provider VITRIOL →
`127.0.0.1:8279/v1`, ctx 131072, timeouts 3600s/1800s. Gateway runs as
systemd user service; restart applies config.

Alternative documented variant: tq3_0 + stream for >92k single-session needs
(§5 row 3).

## 7. Infrastructure shipped

| Artifact | Purpose |
|---|---|
| `VITRIOL-FINGERPRINT` (launcher + server-main + runners) | Every log excerpt self-describes its flags; drift = review blocker |
| `scripts/depth_cert.py` | Single-shot depth certification runner (mode/fa/spec/ts/ub/kv/util-sampling) |
| `scripts/lull_fill.py` / `lull_bench.py` | Chunked-fill harness / short-bench driver |
| `scripts/lull_reuse_audit.py` | Per-turn cache-hit vs forced-miss accounting |
| `scripts/probe_corrupt.py`, `bisect_corrupt.sh` | Repro/bisect tooling from the corruption hunt |
| `scripts/lull_report.py` | Busy/idle percentile tables from split profiler |
| `VITRIOL_POOL_RESET=1` | Compute-pool rewind (submodule `83619cf09`) |
| Attention-probe scoring | `VITRIOL_KV_SCORE=probe`: tail-appended softmax(q·K) scoring feeding cell importance (submodule `46893d105`) |
| `model.alias` profile key + array-safe expansion | Named endpoints ("Lapis Occultus") survive word-splitting |
| Best-effort cap reapply in build script | Rebuilds no longer silently drop caps; sudo optional |

## 8. Process changes (AGENTS.md, commit `ca3cdb5`)

- Protocol §1 rescinded → **Residency Rule** (§3.4 above).
- **Flag Provenance** made mandatory (§4 of protocol).
- **Window ≠ depth** rule added.
- Staleness sweep: VITRIOL_KV_QUANT note corrected (per-device overrides
  verified), MTP guidance consolidated (n=1, zero benefit), opencode entries
  marked `[STALE?]`.
- Standing practice: commit after every step; reports carry ISO 8601 stamps.

## 9. Open threads

1. **Reuse audit accumulation** — forced-full rate under real hermes traffic;
   auditor verdict logic ready (`lull_reuse_audit.py`).
2. **Eviction runtime trigger** — HTTP pre-check cancels before init_batch;
   exhaustion must route through prepare() (dual-slot pressure mapped,
   Addendum 4).
3. **Probe-quality gate** — ppl/divergence comparison once eviction fires live.
4. **Residual creep grower** — input buffers outside compute pool; watermark
   instrumentation ready to lead the hunt.
5. **Phases 3–4 (LULL)** — lull scheduler + tiered cold-KV spill: converts the
   depth wall from a ceiling into a managed resource; also the structural fix
   for §3.6.
6. **131k-filled on Q3_K_M-class weights** — blocked pending items 2/5.

## 10. Artifact index

- Reports: this file; `lull-certification-report-2026-08-24.md` (+Addenda 1–7);
  `lull-phase0-report-2026-08-24.md`; `lull-plan-2026-08-24.md`
- Profiles: `profiles/qwen38-master` (production), `qwen38-mtp-131k` (fixed ts),
  maintainer historicals preserved in `~/.vitriol/profiles/`
  (`qwen38-iq2s-100k`, `qwen38-iq3s-131k`)
- Key commits: outer `393730a` plan · `a2042fb` fingerprints · `ca3cdb5` AGENTS
  rewrite · `aa359bc`/`45b0aab`/`ef6918b`/`dc1cb4e`/`a8a7666` reports ·
  `60499ac` profile fix · `1a3b68c`+`690d421` master/alias · `24f159f` optional
  caps · `955940e`/`66e073c` depth_cert · `0eba9d3` auditor/final · `6b67686`
  alias-array fix · submodule `5968dc0f5`/`7dfb21ea3` profiler · `46893d105`
  probe · `83619cf09` pool-reset · `e013c26d0` server fingerprint
- Logs: `/tmp/opencode/dc-*.log`, `/tmp/opencode/lullfill-*.log`,
  `/tmp/opencode/vitriol_gen.log` (live production)
