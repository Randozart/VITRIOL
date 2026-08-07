# Officina — RECTIFY / named masks / versioned transactions — provenance

Date: 2026-08-07.

## What

`vitriol-tui/src/officina/mask.rs` + grammar/ops: named Rectification Masks that
tally which MoE experts fire across a workload. A mask is an ordered sequence of
sparse transactions; the flat active mask is always the derived union, so
`REVERT` is an exact transaction drop. Ops: RECTIFY (record a firing pass),
LOG, REVERT, DISCARD, `DESCRIBE > model <mask>`. `COMMIT` gains explicit write
targets (`overwrite` / `as "name"`) with a strict safety contract. `GUIDE`
renders `docs/officina-guide.md`.

## Kind

`paper-spec` / public-concept, re-derived. **Activation-based profiling /
dynamic profile-guided pruning** is a standard ML technique (which weights
fire per input → prune the silent ones). Re-implemented independently in Rust.
The "model-is-the-database" framing continues from **LARQL** (public idea,
2026). Alka name repurposed from the user's dropped `alka-lang` project; no
code borrowed.

## Live firing data

`RECTIFY` consumes a `rectify.experts` field in the gen server completion
response, produced by a planned llama.cpp fork hook (R2). Until the hook is
enabled, commit reports an honest "no expert-activity data" error — no fake
results. The mask engine, versioning, rollback, census, and safety contract are
fully functional and tested with constructed firing data.

## Status

R1a+R1b landed. R2 (fork hook), R3 (ASCENSUS > RECTIFY batch), R4
(DISSOLVE > model <mask>) pending.
