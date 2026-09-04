# MTP flag-drift post-mortem — 2026-09-03

**Symptom (owner-reported)**: "it used to run faster as well." The
daily-driver engine (Qwen3.8-27B-Q3_K_M, `vitriol-server.service`) had
been decoding at ~11.5 t/s shallow. The owner remembered ~14.

**Impact**: ~40% decode throughput lost for weeks (11.55 vs 16.49 t/s),
silently, on the primary engine. Nobody noticed because nothing in the
launch path *asserted* the speed-critical flags were still present.

## Timeline

| date | event |
|---|---|
| 2026-05-25 | MTP 5×5 sweep on Qwen3.6-**35B MoE**: "zero benefit" → doctrine written ("omit unless A/B-ing") |
| 2026-08-24 | 27B certifications (depth, tq3_0 KV, ts 26,10); qwen38 profiles carry `[spec] type=mtp draft_n_max=1` |
| 2026-08-31 | **Config retarget** (`ts 22,14`, ctx 81920 for display headroom): `~/.vitriol/config` regenerated/edited — **`[spec]` section silently dropped** |
| 2026-08-31 → 09-03 | Engine runs MTP-less. 11.5 t/s. No fingerprint field, no canary, no alarm. |
| 2026-09-03 | Owner challenges census numbers → A/B → root cause → fixed |

## Root-cause chain

1. **Doctrine overreach**: "MTP has zero benefit" was measured on the
   35B *MoE* with draft depths n≥2, then generalized to the 27B *dense*
   with n=1 — where it is flat wrong (+40%).
2. **Config regeneration without diff**: the 8-31 retarget rewrote
   `~/.vitriol/config` (ts/ctx/parallel) and dropped `[spec]` in the
   same edit. No canary grepped for it; the profile
   `~/.vitriol/profiles/qwen38-mtp-131k/config` still had it — the live
   config just stopped carrying it.
3. **Fingerprint blind spot**: the launcher's mandatory
   `VITRIOL-FINGERPRINT:` line did not include spec fields. Every launch
   log since 8-31 dutifully recorded a fingerprint that was *complete*
   for what it covered and silent about the one flag that mattered.

## Evidence (A/B, 2026-09-03, build/bin, Q3_K_M, c 81920, q4_0 KV, ub 64, 3×64 shallow)

| config | t/s (median) |
|---|---|
| A: ts 22,14, no MTP (drifted live config) | 11.79 (11.75–11.81) |
| B: ts 22,14 + MTP n=1 | **16.46** (16.40–16.49) |
| C: ts 27,9 + MTP n=1 (era config) | 14.69 (14.66–14.74) |

- Live unit after `[spec]` restore: **16.49 t/s** through systemd.
- Bonus: the 8-31 `ts 22,14` split is *better* than the old `27,9`
  once MTP is back (+12%). The retarget's only sin was collateral.

Secondary verification the same day (also owner-prompted): "82k context"
never regressed — `-c 81920` q4_0 KV was in the live argv throughout.
Standing caveat unchanged: window ≠ filled depth (Aug-24: big windows
bottom out ~45-61K *filled*).

## Fixes applied

1. `[spec] type=mtp draft_n_max=1` restored in `~/.vitriol/config`;
   declared **load-bearing** in AGENTS.md (MTP CORRECTION paragraph).
2. `VITRIOL-FINGERPRINT` now emits `spec=<type>:<draft_n_max>` — every
   future launch log either shows the spec config or shows it empty,
   and an empty field next to a 27B launch is review-visible.
3. AGENTS.md "zero benefit" doctrine annotated as superseded for the
   27B dense with n=1 (it stands for the 35B MoE and n≥2 depths).

## Prevention doctrine (the general lesson)

**Config files are flag carriers. Regenerating or hand-editing a config
is a flag-drift event and must be treated like the vendor-patch rule:
diff against the last known-good (profile) before shipping, and make
the fingerprint cover every flag that moves t/s.** The vendor-patch
incident (2026-09-01, four features lost to regeneration) and this one
(2026-08-31, +40% lost to a config retarget) are the same failure class:
a generated/edited artifact silently diverging from intent, discovered
only because a human's memory disagreed with the machine.

Rule: **speed/load-bearing keys in `~/.vitriol/config` (`[spec]`,
`[kv] score*`, `ts`, `ubatch`) are provenance-bearing. Any config write
must be followed by a fingerprint check against the previous one.**

## Open items

- Depth behavior with MTP n=1 not re-certified (the 54,692 @ 9.21 cert
  predates this restoration; decode-at-depth with MTP may differ).
- IQ4_XS verdict unchanged (14.9 shallow / 8.07 @ 32.4K depth) — but
  the driver now outruns it everywhere, so the upgrade case is dead.

## ADDENDUM 2 (2026-09-04): the guard shipped with a landmine — and caught four more losses

**The landmine**: `_fp_delta`'s loop bodies used `[[ test ]] && cmd` under
the launcher's `set -euo pipefail`. A short-circuited final iteration
makes the for-loop inherit status 1 → errexit → silent death of the
launcher. Every serve while blessed ≠ current exited 1;
`vitriol-server.service` crash-looped ~20 min (a4d2807 fixes: if-blocks).

**The vindication**: fixing it required regenerating the config via
`config set` — and the journal immediately exposed that `write_config`'s
template never emitted FIVE load-bearing keys: `kv.score`,
`kv.score_every`, `model.alias` (the pi model-sync identity!), `model.mmproj`
(vision), and `kv.checkpoint_every_n_tokens`. The 8-31 retarget may have
been only the first time this class of loss happened — any regeneration
silently dropped them. All five now emit from the template, the live
values are restored, and `config blessed` matches the running launch.

**Doctrine reinforced**: the journal worked exactly as designed —
CONFIG-EDIT trail + FLAG-DELTA made both the landmine and the five-key
loss impossible to miss. Passive logs alone would have recorded both
without flagging either.
