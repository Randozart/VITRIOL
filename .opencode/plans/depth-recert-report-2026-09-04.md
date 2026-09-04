# Depth-with-MTP re-certification — 2026-09-04

Executes Part 1 of depth-recert-roadmap-2026-09-04.md. First certification
of the live operating point WITH MTP n=1 at filled depth (the 2026-08-24
cert predates the MTP restoration). Also the first measured sparse-KV +
MTP-draft coexistence.

## Config under test (= blessed fingerprint)

Q3_K_M, build/bin (lull engine), ts 22,14, q4_0/q4_0 KV, ub 64, -fa on,
--no-mmap, --kv-unified, --cache-idle-slots, c 81920, --spec-type mtp
--spec-draft-n-max 1 --spec-draft-ngl all, resident. Port 8291, unit
stopped. Methodology: exact-fill via /tokenize, prefill measured from
server timings, decode 3×64 at depth (rounds 2-3 cache_hit — sparse
eviction ACTIVE throughout, as in production).

## Results

| filled depth | prefill | decode 3×64 median (MTP on) |
|---|---|---|
| 13,025 tok | 179 t/s | **12.66 t/s** |
| 26,049 tok | 168 t/s | **11.16 t/s** |
| 36,257 tok | 159 t/s | **10.45 t/s** |

Isolation arm (same launch flags, `--spec-type` omitted):

| filled depth | decode median |
|---|---|
| 26,049 tok | **8.54 t/s** |

**MTP attribution at depth: +31%** (11.16 / 8.54). The shallow +40%
carries to depth almost fully.

VRAM across the ladder: dev0 9,269 → 9,359 MiB — no creep explosion
through 36K filled.

## Comparison with the Aug-24 cert

| | Aug-24 cert | this cert |
|---|---|---|
| config | tq3_0 KV, ts 26,10, MTP **off** | q4_0 KV, ts 22,14, MTP **on** |
| 26K-ish depth | ~9.5 t/s (interp. 9.47@43.9K) | **11.16 t/s** |
| best depth | 54,692 @ 9.21 | 36K @ 10.45 (wall not re-probed) |

Reading: the 8-31 retarget (q4_0 KV + 22,14 split) costs ~10% at depth
(no-MTP arm 8.54 vs old-config ~9.4-9.5 at similar depth), and MTP n=1
more than pays it back (+31%). The daily driver is now faster at depth
than at any point in the certified record.

## Incidents

- One transient SIGKILL of the bare cert server mid-decode (no VRAM
  error, no shutdown banner, kernel log inaccessible). Matches the
  documented bare-serve fragility under swap-heavy conditions (oom-shield
  was down with the unit during the cert). Repro attempt passed cleanly
  (10.45 stable across 3 rounds) — treated as environmental, and as one
  more data point for the restart-via-unit doctrine.
- Wall probe (>36K) deferred: unprotected-launch fragility makes deep
  fills unreliable outside the unit. The Aug-24 wall (54,692 on tq3_0/
  26,10) remains the reference until a unit-protected wall run.

## Disposition

- New operating numbers adopted; blessed fingerprint already matches the
  live config (no config change involved).
- AGENTS.md updated: depth ladder + MTP depth attribution; the "depth
  behavior with MTP not re-certified" open item is RESOLVED.
- MTP n=1 stays load-bearing in `[spec]`.
