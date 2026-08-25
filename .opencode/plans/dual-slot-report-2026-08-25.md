# Dual-Slot Deployment Report — Lapis Occultus (OOM-safe) + Ontic Reserved Slot

> **Date:** 2026-08-25 (implementation completed same day as plan)
> **Plan:** `.opencode/plans/dual-slot-plan-2026-08-25.md`
> **Commits:** submodule `20129c2cc`, outer `a6c712b`
> **Status:** SHIPPED & VERIFIED — production on master; ontic profile tested end-to-end

## 1. Trigger

Kernel OOM kill of production llama-server at 17:52:01 (`anon-rss: 8.4 GiB`,
`oom_score_adj: 200`), preceded by ~8 min of hermes-agent memory-pressure
warnings. c=131072 + q4_0 KV (~2.37 GiB) + weights + compute buffers exceeded
16 GiB DDR3 headroom under real agent load. Separately, the user identified
that `ontic solve` shares :8279 and wanted its KV footprint fenced off.

## 2. What shipped

### 2.1 `--slot-context` port (submodule `20129c2cc`)
Upstream PR #23340 (MIT → GPL-2.0 fork, attribution comment in-tree):
- `common/common.h`: `std::map<int,int32_t> slot_ctx_sizes`
- `common/arg.cpp`: `--slot-context "0=90112,1=8192"` parser with validation
- `tools/server/server-context.cpp`: per-slot vector applied at slot init;
  rejects out-of-range ids / over-budget sums; warns when a size exceeds the
  equal-split physical KV limit without `--kv-unified`
Build: `CMAKE_CUDA_ARCHITECTURES="61;86"`, clean link.

### 2.2 Profiles
| profile | context | slots | purpose |
|---|---|---|---|
| `qwen38-master` | **98304** (was 131072) | 1 | hermes daily driver, OOM-safe |
| `qwen38-ontic` | 98304 unified pool | `0=90112, 1=8192` | dual-slot day |

Both keep alias "Lapis Occultus" — hermes needs zero reconfig when switching.
Synced to `~/.vitriol/profiles/`.

### 2.3 Launcher wiring
`server.slot_context` profile key → `SLOT_CTX_ARGS=(--slot-context …)` in all
four launch paths; fingerprint line gains `slots=` field when set.

### 2.4 Ontic client pin (`forge.rs`)
`"id_slot": ONTIC_SLOT` (=1) hardcoded into the llama-backend request body,
with rationale comment. `cargo check` clean.

### 2.5 Hermes config
VITRIOL provider `context_length: 131072 → 98304`; gateway restarted.

## 3. Verification evidence

| test | result |
|---|---|
| slot topology at load | `n_slots = 2`; `slot 0 n_ctx = 90112`; `slot 1 n_ctx = 8192` |
| default chat routing | lands slot 0, replies correctly |
| pinned `/completion` + GBNF grammar, `id_slot:1` | generates on slot 1 |
| **cap enforcement** | 9001-token prompt pinned to slot 1 → `HTTP 400 exceed_context_size_error, n_ctx:8192` — hard rejection at admission |
| production relaunch | fingerprint `c=98304 kv=q4_0/q4_0 mode=off score=probe pool_reset=1`; smoke READY.; heartbeat flowing |
| reuse audit | baseline HEALTHY |

## 4. Routing semantics (why the client-side pin matters)

Without `id_slot`, the server picks the first idle slot with room — an
unpinned Ontic could land on slot 0 and silently consume hermes budget.
The pin makes contamination structurally impossible; the 8k cap is the
second fence (a misrouted >8k request is rejected outright).

## 5. Known caveats

1. Upstream #27148: RAM prompt-cache can leak content across unrelated
   conversations under some configs. Mitigation if ever observed:
   `--no-cache-idle-slots` (never `--cache-ram 0` — readiness rule).
2. Slot cells persist after release (upstream behavior); acceptable here
   because slot 1 is capped at 8k and usage is sequential (hermes blocks on
   the ontic subprocess).
3. Containerized blueprint for this exact stack documented separately:
   `~/Desktop/Projects/ontic/docs/reports/hermes-trismegistus-architecture.md`.

## 6. Addendum — OOM #2 postmortem & watchdog stack (2026-08-25 evening)

Second kernel OOM kill at 20:18:12 (anon-rss 4.9 GiB, dual-slot c=98304).
Host-level cause: opencode ~5.2 GiB across 6 processes on 16 GiB DDR3; the
server (oom_score_adj 200) is merely the preferred victim. VRAM-side answer
to "shouldn't it degrade gracefully?": **no** — CUDA_CHECK is [[noreturn]];
a foreign VRAM steal aborts mid-graph-build, and no NVML monitoring exists.

Deployed stack (commit `48fc479`, plan: watchdog-persistence-plan):
1. `vitriol-server.service` — Restart=always/5s around the launcher.
2. `vitriol-autosave.service` — `lull_slot_persist.py`: PID-generation
   detection (~5 s) → restore slot{N}.bin into fresh instances; periodic
   disk autosave (300 s) via `--slot-save-path`; warm GDN slot ≈150 MiB.
3. oom-shield — raises big consumers' oom_score_adj to 300 (user-manager
   clamps negative adjust; same-uid can only raise others).
4. Context budget cut c98304→c81920 (split 0=73728,1=8192); hermes follows.

Verified drill: kill -9 → resurrect 10 s → healthy ~40 s → context replayed
306 ms. Data-loss bound = one autosave interval for the conversation tail.

## 7. Addendum — capacity-aware dual-slot routing + branch unification (2026-08-25 night)

**Routing bug**: after checkpoint restores, every unpinned prompt landed on
slot 1 (8192) — restored slots keep `t_last_used == -1` and the LRU tie-break
(`<=`) let the last slot win ties. Hermes' 39k session died with
`exceed_context_size_error` against the 8k window while slot 0 (73728) idled.
Fix in `get_available_slot()` (submodule `441ccd871`): capacity-fit skip in
both selection passes + strict-`<` tie-break. Verified: 12k/39k unpinned →
slot 0; explicit `id_slot=1` (ontic) unaffected.

**Branch unification** (user directive: one branch, all best ideas, no
swapping between models): `vitriol` fast-forwarded `4dfd95ed4 → 441ccd871`
(strict superset — lull-kv, fewer-experts, slot-context, tq3_0 KV, Mellum all
merged); pushed to origin + randozart. `vitriol-mellum2` retained as frozen
alias at same hash. No branches deleted per user preference; `master` stays
upstream-sync base. Safety net first: outer main (78 commits) and submodule
vitriol-mellum2 (11) pushed before any ref surgery.

## 8. Addendum — hang wedge + self-healing stack v2 (2026-08-25 late night)

Incident ~23:00: server did NOT crash — it hung (health deaf, /slots timed
out) under swap pressure: 176 MiB free, server on 7.8 GiB swap, session grown
to 48016 tokens making the slot0 checkpoint 1 GiB. Restart=always only fires
on exit, so a hung process stayed hung. Manual restart recovered; sidecar
replayed 48k tokens in ~2 s.

Hardening (`e5eaabc`), all verified live:
1. Hang watchdog in the sidecar: same-PID health-deaf ~60 s → forced
   `systemctl --user restart vitriol-server.service`.
2. Autosave churn guard: metrics-counter signature skip — frozen counters
   mean no rewrites (1 GiB ticks were thrash fuel).
3. Thrash sentinel: slow /slots aborts save tick.
4. `vitriol stop` now sticky: systemctl-based stop (INVOCATION_ID guard for
   ExecStartPre context); verified no resurrection past RestartSec window.
5. Unit cascade: server Wants=autosave, autosave PartOf=server.
