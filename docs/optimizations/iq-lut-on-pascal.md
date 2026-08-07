# Optimization: IQ-LUT execution on Pascal — REFUTED

Status: **refuted** — blacklisted for CC 6.1.
Lever: none (removed); a lookup-table codebook execution scheme.

## What it is

Execute IQ (integer-quantized) weights via lookup tables instead of arithmetic —
codebook tables that map quantized values to full values at runtime.

## Measured (bitshaper-ai, 2026-08-06, GTX 1070 Ti)

| constraint | result |
|---|---|
| codebook tables vs SMEM | **tables exceed 48 KB shared memory** |

The codebook for the scheme does not fit the Pascal shared-memory budget
(48 KB/SM), so the LUT path cannot even load. Refuted for CC 6.1.

Source: `docs/spagyric-autotuner.md` §3 (citing the shader-test plan).

## Undo

Already removed. On a part with a larger SMEM/SMEM-per-block budget or with the
codebook pushed to registers/const memory, re-test before trusting.
