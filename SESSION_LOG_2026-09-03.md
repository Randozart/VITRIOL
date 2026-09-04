# Session Log — 2026-09-03

Anchored summary of the day: context-lifecycle recovery, Officina model
auto-sync, model census + IQ4_XS calibration, and the MTP flag-drift
find (owner-initiated). Full details live in the linked plans; this file
is the index + outcome record.

## Context lifecycle (officina-context-lifecycle-2026-09-03.md)

- **P0**: lull worktree committed (`fe32247f8`, branch `lull-kv`) —
  probe-scored sparse eviction, `llamacpp:kv_ejected_total` counter,
  eager floor plumbing. Source/binary drift risk closed.
- **P1**: `serve_unit_guard` in `scripts/vitriol` (`aef1b1e`) — bare
  `serve` refused while the unit manages the engine (verified live);
  AGENTS.md gains the context-lifecycle doctrine (restart-via-unit,
  probe keys, checkpoint flag name, Stage-6 inventory).
- **P2/P3** pending: owner first-prompt probe verification; Stage 5
  floor + Stage 6 port standing.

## Officina model identity (`ad1954f`, `9ceb3e7`)

- "Another model mismatch" → real standing divergence (pi said 9B,
  engine had 27B). Loaded-model row now shows alias + underlying GGUF;
  mismatch rule accepts alias OR basename; `defaultModel` aligned to
  "Lapis Occultus".
- Architectural fix (owner: "shouldn't it auto-update?"):
  `llama-cpp-provider` now fetches `/v1/models` at startup and calls
  `pi.setModel()` at session_start — header always shows the engine's
  truth; `defaultModel` self-heals; mismatch detector becomes a true
  anomaly alarm.

## Census + IQ4_XS (`bc57245`, `8ce8af4`; model-census-iq4xs-2026-09-03.md)

- Owner-directed deletions: 9B pair + UD-Q3_K_XL (~29G freed).
- Calibrator census of the 27B quad + Mellum MoE.
- Sweep-controller port bug fixed (8280→SWEEP_PORT) + tq3_0 KV flags.
- IQ4_XS: pin sweep flat (14.9 t/s); depth probe 32.4K filled → 8.07
  t/s decode, 2.2G dev0 headroom. Q4_KS: no usable depth on this pair.

## MTP flag drift (`7f3b2d2`; mtp-flag-drift-postmortem-2026-09-03.md)

- Owner challenge ("82k? faster?") → both memories correct: the 82K
  window was always live; speed had regressed −18%.
- A/B isolated the cause: `[spec]` silently dropped from
  `~/.vitriol/config` in the 8-31 retarget; the "MTP zero benefit"
  doctrine (35B MoE, n≥2) had wrongly generalized to the 27B dense.
- Restored → live unit **16.49 t/s** (+43%), new operating point.
  AGENTS.md doctrine corrected; `[spec]` declared load-bearing.

## Documentation pass (this commit)

- Post-mortem: `.opencode/plans/mtp-flag-drift-postmortem-2026-09-03.md`
- EXPERIMENT_LOG: 2026-09-03 census/sweep/depth/A-B entry.
- Launcher: fingerprint now carries `spec=<type>:<n_max>` (drift becomes
  review-visible); `--dry-run` exempted from `serve_unit_guard` (it
  launched nothing but was refused while the unit ran).

## Blockers / open

- Depth-with-MTP not re-certified (54,692 @ 9.21 cert predates MTP
  restoration — decode-at-depth may now differ; worth a re-run).
- Owner: first-prompt probe verification (⤓ counter), then Stage 5
  floor decision; Stage 6 port standing.
- Session-selector/markdown officina files remain hand-maintained
  (build-patch does NOT cover them — vendor-patch rule).
