# Scratch-clear / KV-cache audit — 2026-11-19 (PLAN mode findings)

## Question
"Scratch cleared because of rampant KV cache. Didn't we limit that to 40 GB max?"

## Answer: the 40 GB cap IS live, and the engine is far under it. The scratch
clear was pi-side compaction, not engine cache pressure. Two separate systems
got conflated; both measured:

### 1. Engine KV prompt cache (cache_max_gb = 40) — HEALTHY
- `~/.vitriol/config:92` → `cache_max_gb = 40`; live fingerprint carries `cmax=40`,
  `cram=16384` (16 GiB host prompt-cache reservoir).
- Disk mirror: `/scratch/kvtier/slot0.bin` = **4.4 GB** (single slot) — 11% of the cap.
- Engine RSS right now: **0.9 GiB**.
- BUT: `vitriol-server` has **16,421 MB in swap** (VmSwap). The ~16 GiB
  `cache_ram` reservoir is resident-but-cold; swappiness 150 paged nearly all of
  it out. The cap limits *how many cache entries exist*, not whether their cold
  pages get swapped. This is the known 2026-09-04 OOM trade-off in reverse:
  the reservoir survives, but a warm hit that lands in swapped pages costs a
  swap-in before it is fast. Not a config bug — a residency tax.

### 2. pi session transcripts (what "scratch cleared" actually was) — 356 MB
- `~/.pi/agent/sessions/` = 356 MB total; briev-lang project alone 204 MB.
- **29 `.patched-*` files, 220 MB** — every compaction/repair leaves the old
  transcript in place and appends a new copy. This churn, not the KV cache, is
  what wiped the conversation context.
- `.officina/` (scratchpad/tasks external truth) = only 236 KB — survived fine.

## System memory right now
39 GiB RAM: 25 used, 1 free, 15 buff/cache; swap 20/103 GiB used. The engine is
the dominant swap tenant (16.4 of 20 GiB) exactly as the -500 oom_score design
intended (last killed, first swapped).

## Options (for a future code session — NOT executed)
- **A (do nothing)**: cap is respected; swap-in latency on cold cache hits is
  the price. Cheapest, matches the OOM-hardening doctrine.
- **B (residency probe)**: re-visit the 2026-11-18 "anonymous resident attempt"
  idea — mlock or `mlockall`-lite on the hot portion of the reservoir so the
  FIRST 2–4 GiB of hot entries stay physical. Needs the unqualified-residency
  caveat resolved before claiming anything.
- **C (session churn)**: pi-side, `.patched-*` GC — retire transcripts older
  than N days except the newest per project. ~150–200 MB recoverable, zero
  engine risk. Low priority (356 MB is not pressure).
- **D (cap the reservoir lower)**: `cache_ram 16384 → 8192` halves the swap
  tenant but loses warm-restart depth. NOT recommended without measuring
  cold-hit frequency first.

## Recommendation
A now + C at leisure. B only if cold-cache swap-in stalls are actually felt
(watch `vitriol:` cache-hit latency vs the swapped state). Keep the cap at 40.
