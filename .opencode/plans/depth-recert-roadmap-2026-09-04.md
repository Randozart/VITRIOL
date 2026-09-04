# Depth re-certification + session roadmap — 2026-09-04

Follows: mtp-flag-drift-postmortem-2026-09-03.md (MTP restored, +40%
shallow, live 16.49 t/s), model-census-iq4xs-2026-09-03.md. Models
deleted 2026-09-04: UD-Q4_K_S, UD-IQ4_XS, Mellum2-12B (37.6G freed);
roster is now Q3_K_M + mmproj only.

## Roadmap (prioritized)

1. **Depth-with-MTP re-certification** (this session) — certified
   54,692 tok @ 9.21 t/s predates MTP restoration. Certify the LIVE
   operating point at depth; re-bless on success; update AGENTS.md.
2. **Hygiene batch** — usage() duplicate officina line; stale opencode
   provider entries in AGENTS.md ([STALE?] qwen38-mtp/qwen38-262k);
   stale profile notes (ts 27,9 era vs today's 22,14+MTP).
3. **Owner first-prompt verification** (owner, ~10 min) — Officina
   restart: model-sync header, probe activation in gen log, ⤓ counter.
4. **Stage 5 floor decision** — blocked on #3's counter data.
5. **Stage 6 lull-kv → main port** — standing, separate session.

## Part 1 methodology: depth cert at the live operating point

**Config under test** (= blessed fingerprint): Q3_K_M, build/bin (lull
engine), ts 22,14, q4_0/q4_0 KV, ub 64, -fa on, --no-mmap,
--kv-unified, --cache-idle-slots, c 81920, --spec-type mtp
--spec-draft-n-max 1 --spec-draft-ngl all, resident (VITRIOL_MODE off).

**Departures from the Aug-24 cert** (NOT isolated variables): KV
q4_0 (not tq3_0), ts 22,14 (not 26,10), MTP n=1 (was off), 82K window
(not 131K). This cert answers "what does the daily driver deliver at
depth NOW", not a controlled A/B. One isolation arm: MTP-off decode at
32K depth (single fill + 3×64) to attribute MTP's depth contribution.

**Procedure** (port 8291, unit stopped, VRAM free):
1. Launch, health-wait (~40s).
2. Depth ladder on one server: exact-fill 16K / 32K / 45K via
   /tokenize+prefill (prefill t/s measured per depth), then decode 3×64
   at that depth (cache_prompt=True for rounds 2-3 — sparse eviction
   remains ACTIVE during decode: that is the real operating point).
3. Wall probe: fresh server per attempt, fills 60K → 70K → 80K until
   failure or window cap; nvidia-smi after each depth (creep signature).
4. Compare vs certified 54,692 @ 9.21 (Aug-24) and shallow 16.49.

**Outcomes**:
- success → new depth numbers in AGENTS.md + re-bless fingerprint
- wall found → documented (window ≠ depth discipline)
- MTP-at-depth regressions → documented; shallow-vs-depth doctrine
  updated; [spec] stays (shallow +40% dominates daily use)

**Risks**: OOM at depth may wedge the slot (fresh server per wall
attempt); sparse eviction + MTP draft interplay is untested — a decode
collapse at depth is a FINDING, not a failure of the cert.

## Part 2 hygiene (after cert lands)

- `usage()`: duplicate `vitriol officina` line (2-line fix)
- AGENTS.md: resolve [STALE?] opencode provider entries; refresh
  profile table notes to the 22,14+MTP operating point; census facts
  (deleted models)
