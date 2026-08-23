# Reference 5 — speculative decoding, MTP & the ngram family

Flags that make generation faster by drafting multiple tokens per step.

PROVENANCE: arg.cpp semantics; REBIS MTP measurements in
EXPERIMENT_LOG.md / AGENTS.md (Qwen3.8 profiles).

## Two mechanisms

1. **Model-based**: a draft model proposes tokens; the target verifies.
   `--spec-draft-model -md PATH` + all `--spec-draft-*` placement/cache/
   thread mirrors of Reference 3 (`-ngld`, `-devd`, `-ctkd/v`,
   `--n-cpu-moe-draft`, cpu-mask/prio/poll drafts…).
2. **Ngram-based**: no second model — lookup tables over recent context.
   `--spec-type ngram-*` selects: simple, map-k, map-k4v, mod — each with
   `-size-n` (lookup length), `-size-m` (draft length), `-min-hits`.

## Draft tuning

| flag | does | REBIS note |
|---|---|---|
| `--spec-draft-n-max` | max drafted tokens/step | **1** for Qwen3.8 native MTP: trunk-seeded depth-1 is ~100% accepted; deeper regresses (measured sweep: n=5→9.0, n=3→11.3, n=2→12.9 vs n=1→14.1 tok/s) |
| `--spec-draft-n-min` | min before verification | keep low with MTP |
| `--draft-p-split/--draft-p-min` | greedy-split controls | untouched |

## Native MTP

Qwen3.8 ships an embedded Multi-Token-Prediction head; this fork activates it
with `--spec-reasoning`-style arch override (`qwen35_mtp`) + `--spec-type`.
Mellum2 also trained an MTP head but **does not export it in released GGUFs**
— so Luna cannot speculate herself.

## Research notes

- Deeper chains decay: chained drafts drift and each costs ~8ms. Depth-1
  "trunk-seeded" was the sweet spot on this rig.
- A Medusa/Eagle-style *trained* head could go further but requires training
  compute we haven't justified; noted as future work in D2 scoping.

## Removed names

Old `--draft/--draft-n/--draft-min/--spec-ngram-size-*` flags now error with
pointers to their replacements — scripts fail loudly instead of silently
changing meaning.
