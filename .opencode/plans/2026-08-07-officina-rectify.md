# Officina — RECTIFY, named masks, versioned transactions, commit-safety

Date: 2026-08-07.

## 1. Vision

**RECTIFY** (from VITRIOL's "R" = *Rectificando* — "by rectifying"): activation-
based profile-guided pruning. Your own queries are the heat that distills the
model — the active "spirit" weights from the inert "dross." A persistent,
**named** Rectification Mask tallies which MoE experts actually fire across your
workload; dross experts (never fired) are the prune targets.

Decisions (user, 2026-08-07):
- **Expert-level firing first** (real, measurable via a fork hook; dense-layer
  "fired weights" is ill-defined). Parameter-level recorded as a later option.
- **Fork hook approved**: modify the llama.cpp fork to trace expert activity.
- **Masks as files first** (`~/.vitriol/masks/*.mask`, JSON), Pymander later.
- **Strict commit-safety**: bare `COMMIT >` on base-destructive ops is blocked.

## 2. The model

A named mask is an **ordered sequence of sparse transaction masks**:

    M_final = M_1 ∪ M_2 ∪ … ∪ M_T

Each `RECTIFY` pass is a transaction `{ id, ts, prompt, source, fired: Vec<u32> }`
(a few KB — sparse). The flat mask is **always derived**: the union of remaining
transactions. **Surgical rollback** of transaction *j* is just recomputing the
union excluding *j* — no flat-mask bookkeeping, no pollution to guess away.

## 3. Grammar (R1a)

- **COMMIT kinds**: `COMMIT >` (bare) · `COMMIT overwrite >` · `COMMIT as "name" >`.
  `Command` gains `commit_kind: Option<CommitKind { Overwrite, SaveAs(String) }>`.
- **`ASCENSUS >` modifier**: consumed as a prefix — `ASCENSUS > RECTIFY > …`
  → `Command { cloud: true, keyword: Rectify, … }`.
- **New keywords**: `RECTIFY`, `DISCARD`, `LOG`, `REVERT` (LOG/DESCRIBE
  read-only; RECTIFY/DISCARD/REVERT probe-by-default).
- **Strict-block validator**: base-destructive ops (RECTIFY-mask-write, DISCARD,
  REVERT, DISSOLVE, COAGULATE) with bare `COMMIT >` are blocked with the
  overwrite/as prompt. Probe (no commit) always allowed — shows impact.
  COMPILE/RECORD/STOP/PLAY keep bare COMMIT (artifact/recipe). **Breaking**:
  existing grimoires with `COMMIT > DISSOLVE` need the explicit form.

## 4. Mask engine (R1b, pure Rust)

- `officina/mask.rs`: `MaskFile { name, transactions }`; `union_active()`,
  `add(txn)`, `revert(id)` (drop txn), `stats(total_experts)` → active %,
  dross count, est bytes (per-expert size from the catalog when available).
- Files at `~/.vitriol/masks/<name>.mask` (JSON); `config.rs` `masks_dir()`.
- **Ops**:
  - `RECTIFY > "prompt" [into <mask>]` — run a generation, record fired experts
    (live data from R2; honest error until then).
  - `DISCARD > model <mask>` — delete the mask file.
  - `LOG > model <mask>` — transaction history newest-first.
  - `REVERT > model <mask> <id>` — probe shows impact, commit drops the txn.
  - `DESCRIBE > model <mask>` — mask census (active %, dross, est size).
- **Journal `[MASKS]` section**: list masks + active % + most-recent highlight.

## 5. Fork hook (R2) — the real firing-data gate

In the llama.cpp fork MoE path (ggml top-k / llama-graph): accumulate chosen
expert indices per slot across the request; expose a `rectify: { experts: [...] }`
field in the completion response. RECTIFY consumes it. Requires a fork rebuild
(compiles without GPU).

## 6. ASCENSUS > RECTIFY (R3)

Rust Gemini client (ureq + the `~/.vitriol/secrets` key): intent → N
cloud-generated calibration prompts → sequential RECTIFY batch populating the
named mask. Probe shows count; commit runs the batch. Solves the cold-start
problem — a specialized mask in minutes, not weeks.

## 7. DISSOLVE integration (R4)

When P3's offline rewrite lands: `DISSOLVE > model <mask>` drops the dross
experts from the rewritten model. `COMMIT overwrite/as` targets the model file.

## 8. Provenance

`docs/provenance/officina-rectify.md`: activation-based profiling / dynamic
profile-guided pruning — a standard ML technique re-implemented independently;
"model-is-the-database" continuity from LARQL (public idea). No third-party code.

## 9. Tests

Mask union/revert/exclusion, stats, JSON roundtrip, id ordering; grammar
(COMMIT kinds, ASCENSUS modifier, new keywords); safety (bare COMMIT blocked on
destructive, probe allowed, artifact ops fine); journal `[MASKS]`; LOG/REVERT
flow. Gates: test/clippy/fmt/Praetor each commit; release rebuild.

## 10. Results

- **R1a+R1b landed** (`10bf411`): grammar (COMMIT overwrite/as kinds, ASCENSUS
  modifier, RECTIFY/DISCARD/LOG/REVERT/GUIDE), strict commit-safety validator,
  `mask.rs` engine, ops, journal `[MASKS]`, `docs/officina-guide.md`, Shift+arrow
  tab nav.
- **R2 landed** (`61dd39876` fork + `5ef89fe`): expert-activity hook in the
  llama.cpp fork. `build_moe_ffn` emits an absolute-expert-id tensor
  (`ffn_moe_topk`); a scheduler eval callback tallies the top-k router selection
  per layer per token into the context's fired set; `/v1/completions` with
  `"rectify":true` returns `rectify.experts`. Verified LIVE on the real
  DeepSeek model (HTTP 200, sane 0..63 ids, top-6/layer/token unioned). The
  model's diverse routing fires all 64 experts on generic prompts — a genuine
  finding, not a bug.
- **R4 landed** (`f70919c`): `DISSOLVE > model <mask>` drops dross experts. `DrossEdit::apply()` zeroes every block fully inside a dross expert's FFW row range (conservative: blocks straddling an expert boundary are kept). `n_ffn_expert` auto-detected as the largest dim divisible by n_expert. **Live-verified on the real DeepSeek model**: 133 FFN tensors, 515,049 blocks zeroed for dross 40..63; expert 63 → 1368/1368 blocks (100%), expert 0 kept; replan OK.
- The full Officina loop is live: RECTIFY (tally) → LOG/REVERT/DISCARD (manage) → DESCRIBE > model <mask> (census) → COMMIT as "x" > DISSOLVE > model <mask> (drop dross) → COMMIT as "x" > COMPILE (bundle).
- COAGULATE remains unimplemented (low value — the model's FFN weights are quantized, not size-preservingly foldable). Live logit-parity (server restart on the rewritten file) pending GPU.

