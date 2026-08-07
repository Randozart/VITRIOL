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

`RECTIFY` consumes the `rectify.experts` field in the gen server completion
response, produced by the fork's expert-activity hook (R2, landed): `build_moe_ffn`
emits an absolute-expert-id tensor named `ffn_moe_topk` (group-MoE offsets
added), and a scheduler eval callback tallies the top-`n_expert_used` router
selection per layer per token into a per-context fired set. `/v1/completions`
with `"rectify":true` returns the union. Verified live on the real DeepSeek
model.

## Status

R1a+R1b, R2 landed. R3 (ASCENSUS > RECTIFY batch), R4 (DISSOLVE > model <mask>)
pending.
