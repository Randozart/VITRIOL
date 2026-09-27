# Optane-Resident KV Tier — eliminate re-prefill — 2026-09-27

**Status**: COMPLETE 2026-09-27 (Stages 0-4 done). Inner commit `15686347f`,
outer config `cache_ram = 12288`, fingerprint diff `cram=2048 -> cram=12288`
(all 18 other keys identical).

## Result (certified on live engine)

SIGKILL -> unit restart -> same prompt: **cached 745/755 tokens, 0.5s reply**
(vs 14s cold prefill; vs ~7 minutes for Hermes' 190K-token session at
~450 t/s — the exact workload that motivated this plan). Boot reload of
~9.8 GB of spilled states: "disk load: restored 2 prompt states (3 skipped)".

## What was done

- Stage 0: mega-prefills identified as Hermes/opencode sessions retrying
  full context after restarts; X-Session-Id only sent by officina/Hermes;
  plain-path LRU swaps had zero persistence; RAM cache rejected all
  mega-states (2048 MiB pool < one 2.5-7 GiB state, 9 skip events logged).
- Stage 1: `cache_ram 2048 -> 12288` (deviation from approved 8192:
  measured states up to 6967 MiB; header comment documents 262K slot KV
  ~= 5 GiB). Fingerprint law honored.
- Stage 2: inner fork `server_prompt_cache` gains disk mirror
  (`cache-<fnv>.bin` under slot_save_path, tmp+rename, detached writer via
  shared_ptr state ownership — dispatch never blocks, never copies; boot
  `disk_load` newest-first with containment dedupe + pool-limit stop +
  damaged-file removal; size-trim to RAM pool limit; mtmd skipped).
- Stage 3: cert as above. Async spill verified: B answered in 0.7s while a
  4.5 GB writer ran in background.
- Stage 4: OFFICINA.md KV-tier section (incl. known issues).

## Known issues (recorded, open)

1. Fork seq-state serialization under `--kv-unified` bloats states
   ~60-230x vs theoretical (19.1 KiB/token); spill mirrors bloat
   symmetrically (function correct, footprint fat). Future: compact
   per-seq cells at serialization.
2. Explicit `id_slot` requests bypass the prompt cache (upstream
   dispatch semantics; cert had to use the LCP/LRU path).
3. `llamacpp:kv_cache_spill_total` reads 0 across processes despite
   successful spills — wiring unproven, observability gap only.
4. Mega-spills are multi-GB disk writes; boot reload of 9.8 GB adds
   ~40s to startup. Acceptable; watch /scratch headroom (164G free).
5. Old `cache_ram=2048` cap accidentally shielded dispatch from
   multi-GB get_data serializations; the async-writer design replaces
   that shield properly.

## Problem statement

Sessions re-prefill from zero after engine restarts and slot swaps despite
`--slot-save-path /scratch/kvtier` (Optane). Today: 87.7K + 118K + ~170K-token
full prefills (minutes each) with `n_prompt_tokens_cache=0`. Owner expectation:
KV persists on Optane; next turn = delta prefill, seconds not minutes.

## Root-cause chain (researched, file:line in inner fork, main branch)

Three subsystems, two broken in live config:

| subsystem | flag | medium | survives restart | live status |
|---|---|---|---|---|
| prompt cache | `--cache-ram` | host RAM | no | **dead**: total pool 2048 MiB < one slot state (3567.576 MiB) — every admission rejected (`server-task.cpp:1735-1738`) |
| checkpoint ring | `--ctx-checkpoints` | host RAM | no | irrelevant: dense non-SWA never checkpoints (`server-context.cpp:3452-3465`) |
| slot state files | `--slot-save-path` | disk | **yes** | works mechanically; saves only on session-LRU eviction (`route_session_save`, `server-context.cpp:5278-5296`) or client POST `/slots/{id}?action=save` |

Breaks:
1. RAM pool can never admit a real mega-session (per config, not per-slot:
   `server-task.h:618-624`).
2. Disk saves require `X-Session-Id` session routing (`route_session`,
   `server-context.cpp:5355-5443`, needs `slot_save_path` + `n_parallel>=2`,
   both set). Plain clients bypass → silent `prompt_clear()` on eviction.
   Idle-clear path (`:2428-2444`, `cache_idle_slots`+`kv_unified`) also wipes
   — but live fingerprint `cis=0` → inactive today.
3. No server-side idle autosave; officina's 300s autosave is client-side only.
4. Restart-restore already exists LAZY (`:5389-5408` — first request with a
   matching session id restores `sess-<key>.bin`) and restored slots
   prefix-match (`get_common_prefix`, `:3209-3211`) → delta-only prefill. The
   mechanism works (officina 3/3 SIGKILL cycles, ~300ms). What's missing is
   REAL saves landing for the big sessions.
5. Today's proximate trigger: 15:02 restart (headless testing, protocol-
   compliant) wiped live KV; no real saves existed; 1200-byte stubs only
   (short officina-test saves).

Current mitigating context (why cache-ram raise is OOM-safe now): engine
`oom_score_adj` −500 (SYSTEM unit, last OOM victim), 16G /swapfile NOCOW +
zram, VmSwap low, MemAvailable ~13.8G. The 2026-09-04 trim (2048) predates
the hardening that makes a raise safe. Flag provenance: fingerprint carries
`cram=` — config write MUST be followed by fingerprint diff.

## Stage 0 — Diagnose (read-only)

- Correlate 87.7K/118K/170K tasks with clients: log lines + ss/lsof; who
  sends `X-Session-Id` (officina does; Hermes?; bare curl does not).
- Why eviction of the 87K session (22:04 log: LRU pick) produced no
  mega `sess-*.bin` (no session id? persist failed? different path?).
- Deliverable: per-session wipe chain appended to this doc.

## Stage 1 — `--cache-ram 8192` (config-only, immediate relief)

- `~/.vitriol/config` cache-ram key 2048 → 8192 (+ profile copy).
- `systemctl restart vitriol-server.service` (unit only, never bare serve).
- Fingerprint diff: expect `cram=2048` → `cram=8192`; embed both in the
  final report.
- Effect: eviction RAM-saves admit mega-states; prompt-cache reuse across
  slot swaps; LRU evict-to-fit works.

## Stage 2 — Engine patch (scope per Stage 0)

- Disk-save-before-`prompt_clear()` on wipe points lacking it (no-session-id
  eviction; idle-clear if Stage 0 shows it matters with `cis=1` later).
- Save task is busy-guarded + deferred (`:2545-2550`) — reuse; do not save
  mid-generation.
- Observability: `kv_disk_saves_total` counter + prometheus gauge (pattern:
  `n_kv_ejected`, `server-task.cpp:1590-1593`).
- Inner-fork commit on main, outer submodule bump (repo convention).
- Optional guard: only save when slot tokens ≥ threshold (e.g. 4096) so
  tiny sessions don't churn Optane.

## Stage 3 — Certify

- SIGKILL server → unit restart → repeat original request (same
  X-Session-Id, full history) → assert `n_prompt_tokens_cache ≈ full`,
  prefill delta only, first-token latency seconds.
- A/B: cold re-prefill vs restore+delta wall-clock (same prompt).
- Report in this doc + `.opencode/plans/` embed, fingerprints on every
  result line.

## Stage 4 — Docs

- OFFICINA.md: KV residency tier (VRAM → RAM 8G → Optane).
- AGENTS.md testing protocol: restart cost note + cache-ram law.
- Profile: qwen38 dual-3060 profile updated.

## Success criterion

Restart + same-session next turn = seconds (delta prefill), not minutes.
