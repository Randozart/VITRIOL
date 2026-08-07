# Optimization: input prefolding — REFUTED

Status: **refuted**.
Lever: none (removed); an input-structure precomputation scheme.

## What it is

Precompute/fold input structure ahead of execution so the runtime avoids
re-deriving it per request.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| claim | result |
|---|---|
| exact input prediction | impossible — input is not predictably known ahead of time |
| structural prefolding | contradicted by measurement |

Exact input prediction is impossible by construction, and the structural
prefold was contradicted when measured. Refuted.

Source: `docs/spagyric-autotuner.md` §3.

## Undo

Already removed. Nothing to restore; any future scheme must predict inputs
deterministically, which the refutation rules out for arbitrary requests.
