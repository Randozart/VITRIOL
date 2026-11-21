# Research: Officina↔VITRIOL feature exchange + dual-slot trial (2026-11-21)

## Part 1 — Feature exchange

### Officina → VITRIOL (features that belong in the engine)

| Officina feature | Where | VITRIOL fit |
|---|---|---|
| **Band content hold** (`tui/band_hold.rs`) | geometry debounce had no content debounce → blank rows during the 900 ms settle window | The engine's slot telemetry (`/slots`) has the same shape: poll cadence vs state flapping. A **hold-last-good + TTL** wrapper for slot/busy state would stop `/slots` from reporting transient empty between requests — exactly the stale-snapshot class the slot-contention fix fought from the other side. |
| **Config Resolution Oven** (`_shared/resolve-config.ts`) | precedence folklore across 24 files caused the router bypass | The launcher (`scripts/vitriol`) + profile loader + unit EnvironmentFile is the same shape: three sources, undocumented precedence. A single resolved-snapshot with `why(key)` would have caught the 8-31 flag-drift incident (`[spec]` silently dropped, 40% decode loss for weeks). |
| **Seam tripwires** (CI greps for known bug shapes) | filename contracts, parity testers | The engine ↔ Officina filename contract (`<stem>-turn-N.bin`) is exactly the class the KV cap breach exploited. A tripwire asserting the C++ scanner's patterns match the TS writer's would be pure win. |
| **Path-keyed caches with (path, mtime) keys** | mode colour cache served file A's data for file B | `provider-catalog.json` / `gateway-health.json` caches in Officina already do this; the engine's model catalog cache should adopt the same shape if it doesn't. |

### VITRIOL → Officina (features that belong in the harness)

| VITRIOL feature | Where | Officina fit |
|---|---|---|
| **Fingerprint discipline** (`VITRIOL-FINGERPRINT:` line, argv embedded in every result) | every launch | Route decisions already log `reason`; a **config fingerprint per routing decision** (which precedence rule won, which env file, which unit state) would make incidents like the bypass diagnosable from one line instead of a grep session. M4's doctor is close; embedding the fingerprint *in the ledger row* completes it. |
| **Slot save/restore + warm resume** | `server-context.cpp` post_slots | Officina's crash-recovery marker already exists; engine-side warm restore is the engine's job — no port needed. Skip. |
| **Admission control honesty** (`server-context.cpp:3658` rejects over-window requests with an explicit 400) | server | Officina's lanes could adopt the same "reject with named reason" instead of generic errors — the lane picker would then distinguish context-overflow from rate-limit without parsing bodies. |

### Not worth porting (examined, rejected)
- Fire/watermark/glimmer: TUI-only, zero engine value.
- Adaptive-router P3 peer arbitration: the engine's slot table IS the authority;
  routing above it would duplicate admission control.
- Phase-model: Officina-specific lifecycle.

## Part 2 — Dual-slot trial on the current config

### History (must-read before touching anything)
- **2026-08-25 dual-slot report**: two kernel OOM kills on 16 GiB DDR3;
  root cause was **DRAM prompt-cache entitlement** (`cache_ram_mib` default
  8192 MiB) + zram, not VRAM. P1/P2/P3 deployed (cache_ram 1024, proactive
  bounce, retries).
- **2026-11-14 protect-single-slot-background**: single-slot engines never
  launch local background reviews (they displace the foreground KV ring).
  The gate already supports multi-slot — it activates the moment ≥2 slots
  exist. **So Officina is dual-slot-ready; the engine config is the only
  blocker.**
- **E6 verdict**: sparse-KV eviction is unreachable at `parallel=1` — with
  2 slots it becomes reachable for the first time (needs re-certification).

### Current hardware budget (measured today)
- VRAM: 10.6 + 10.9 GiB used of 12+12 (≈2.8 GiB free total) — tight.
- DRAM: 39 GiB total, 10 GiB available (the 8-25 OOM context is gone — this
  box has 39 GiB now, not 16).
- KV cost at q4_0: **19.1 KiB/token**.
  - Two slots × 131072 tokens = 2.39 GiB VRAM/slot (vs one 262144 slot at
    4.78 GiB) → **VRAM-neutral**, actually frees ~2.4 GiB.

### Existing profile
`~/.vitriol/profiles/lapis-dualslot/` already exists (2026-09-01): Q3_K_M,
ts 22,14, c 81920, parallel 2, kv-unified, sparse. **Stale** — it targets the
old Q3_K_M model and 580xx driver era. Not usable as-is; a fresh profile
derived from the CURRENT live config is the right move.

### Proposed trial config (derived from live, minimal delta)
```
parallel = 2            # was 1
context = 262144        # unchanged — engine splits the window across slots
# everything else identical: ts 27,21, kv q4_0/q4_0, MTP n2, cram 16384
```
VRAM: KV total stays 4.78 GiB (2 × 2.39); per-slot compute buffers grow
slightly (second context set). The risk is DRAM-side (per-slot checkpoint
ring, cache_ram pressure) — which the 39 GiB host absorbs far better than
the 16 GiB box that produced the 8-25 OOMs.

### Open risks to measure, not assume
1. **Per-slot window vs single 262144 window**: if the engine splits
   `context` per slot (131072 each), deep-context sessions lose half their
   window. If it multiplexes, KV doubles. **Needs one restart + /slots +
   /props read to answer.** This is the decisive unknown.
2. **MTP n2 with parallel=2**: draft model sharing across slots was never
   certified; acceptance rate may drop.
3. **Sparse-KV eviction becomes reachable** (E6 re-test opportunity).
4. **Cache_ram**: 16384 MiB with two slots → confirm the prompt cache
   doesn't double-entitle.

### Trial protocol (if approved)
1. Derive `lapis-dualslot-2026` profile from live config, `parallel = 2`.
2. `vitriol config load` + unit restart, fingerprint diff (mandatory).
3. `/slots` + `/props`: record per-slot window (decisive unknown #1).
4. Shallow bench 3×64 both slots, depth probe at 60k filled, MTP acceptance.
5. Restart-day-style report; keep the single-slot profile untouched for
   rollback (`vitriol config load qwen38-buffedmod-262k-cache8-mtp2`).

## Verdict
- Feature exchange: 4 items → VITRIOL, 1 item → Officina, rest rejected with
  reasons.
- Dual-slot: **plausible and reversible**, with one decisive unknown
  (per-slot window semantics) that only a live restart answers. The 39 GiB
  host removes the historical OOM class; VRAM is neutral-to-positive.
