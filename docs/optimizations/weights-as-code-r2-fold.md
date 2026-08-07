# Optimization: weights-as-code (R2-FOLD) — REFUTED

Status: **refuted** — blacklisted for CC 6.1, L2 < ~8 MB.
Lever: none (removed); represented a ternary-weight representation re-derivation.

## What it is

Represent model weights as program code — an "execute the weights" scheme where
the ternary weight matrices compile to instructions instead of being fetched as
bytes.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| claim | measured |
|---|---|
| bit-exact parity | ✓ bit-exact |
| packed bytes | **92.8× larger** (346 MB/tensor vs 3.73 MB); whole model ~73 GB |
| cacheability | un-cacheable on 2 MB L2 |

The packed-bytes blowup makes the representation untransportable and the whole
point (avoid weight fetch) moot — the instruction form is bigger than the data
it replaces. Refuted.

Source: `.opencode/plans/2026-08-06-spagyric-shader-test.md` §14 (as cited in
`docs/spagyric-autotuner.md` §3).

## Undo

Already removed. If a denser instruction encoding were found (e.g. macro-based),
the blacklist rule (`CC 6.1, L2 < ~8 MB`) must be re-checked first.
