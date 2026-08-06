# Provenance: kimi-k3-in-c (inspiration — Apache-2.0, NOT incorporated)

Date: 2026-08-06.

**Repo:** https://github.com/FareedKhan-dev/kimi-k3-in-c — Kimi K3 inference in portable
C99 (2.78T MoE, 1.56 TB safetensors checkpoint, 8.24 GB peak RSS, one CPU, no GPU).

**License:** Apache-2.0 (code). The Kimi K3 **weights** are Moonshot AI's under their own
license (the repo ships none).

**VITRIOL status: Apache-2.0 is GPL-2.0-incompatible → NOT copied, re-derived only.**
Per VITRIOL/AGENTS.md "Licensing and Provenance (GPL-2.0)", Apache-2.0 implementations are
studied and independently re-implemented. Nothing below is borrowed code; each is a design
*learned* and re-derived for VITRIOL's (different) architecture.

## What was learned (candidate re-derivations)

### 1. Three-state expert cache (INFLIGHT / EMPTY / pinned)
The cache slot has three states, not two: a slot being read right now is `INFLIGHT` and
is skipped entirely in victim selection (cannot be double-claimed or evicted mid-read);
`EMPTY` is returned immediately (a free slot beats evicting a live one); pinned slots are
skipped *after* the empty test so pinning never blocks the fast path. Fetches are reserved
serially but read in parallel (batched `pread`s keep the device busy; serial gets idle it).
- VITRIOL today: `g_lru_map`/`g_lru_order`/mutex (vitriol-cuda-integration.cpp), LRU order
  only, no INFLIGHT state; per-expert PCIe DMA dispatched one-by-one.
- Re-derive: add INFLIGHT tracking to victim selection + batch the per-expert DMA.

### 2. Pinned-prefix over LRU for cyclic scans (+ fixed-point budget solver)
For a cyclic layer scan (walking layers 0..N on every token), LRU is pathological — the
least-recently-used layer is always the one just evicted, so an LRU over a 90/93-layer
cycle achieves exactly 0% hit. Pinning the first N layers gives a deterministic N/93 hit
rate (96.8% at N=90). A fixed-point loop sizes the pin-count vs ring-slot split under a
memory budget.
- VITRIOL has `VITRIOL_PIN_FIRST_N_LAYERS` (manual); the fixed-point budget solver is the
  missing piece.
- Re-derive: a helper that computes the pin count from a VRAM budget.

### 3. MXFP4 E2M1 decode (byte→pair LUT)
MXFP4: each weight is a 4-bit E2M1 nibble (16-value table: 0, 0.5, 1, 1.5, 2, 3, 4, 6,
and negatives), one E8M0 shared exponent per 32. Decode via a byte→two-weights LUT (one
lookup instead of two shifts) + a power-of-two table for the shared scale.
- VITRIOL's Mellum MXFP4 path exists; audit against this reference and adopt the byte→pair
  LUT if ours does per-nibble shifts.

### 4. Memory-ladder methodology
cgroup-enforced memory caps (`MemoryMax` + `MemorySwapMax=0` so an over-budget run dies
rather than swapping); byte-identical output across all budgets (determinism); 33% noise
floor with replication mandated.
- Re-derive: enforce memory caps + a determinism check in VITRIOL's benchmark harness.

### 5. KDA (constant-memory attention) — strategic note only
69/93 layers keep a fixed ~626 MB recurrent state at any context length; only the 24 MLA
layers grow with context. No KDA model exists for VITRIOL today; recorded as the
future-model consideration (would change the measured `context = parallel x ctx` budget).
