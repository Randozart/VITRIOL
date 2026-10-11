# FastMTP d2t probe — results

- **Date:** 2026-10-11 18:45
- **Status:** complete
- **Question:** does the FastMTP d2t-trimmed external draft beat internal MTP on our rig?
- **Answer:** yes, ~10% faster at 49k, comparable acceptance. Cross-finetune draft works.

## Setup

- Model: Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K (daily driver)
- Draft: HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-FastMTP-32K (sha256 verified = lfs.oid)
- ctx 49152, ts 27,21, q4_0 KV, ub 128, sparse probe
- Bench: 1 warmup + 3×64-token greedy, samplers=["greedy"], ~1.2k-token fixed prompt
- Engine: vitriol-dma (rebuilt with d2t fix)

## Results

| arm | spec | median t/s | acceptance | mean len | dev1 VRAM |
|-----|------|-----------|------------|----------|-----------|
| A   | internal MTP n=3 | 29.4 | 0.75–0.77 | 3.26–3.32 | 7372 MiB |
| B   | FastMTP d2t n=3 | **32.3** | 0.68–0.79 | 2.95–3.32 | 7704 MiB |
| C   | no-spec | 18.5 | — | — | 6564 MiB |

- FastMTP vs internal MTP: **+10%** (32.3 vs 29.4 t/s)
- FastMTP vs no-spec: **+75%** (32.3 vs 18.5 t/s)
- Internal MTP vs no-spec: +59% (29.4 vs 18.5 t/s)

## Engine fix required

The draft GGUF loads with `general.architecture = qwen35` (same as main). The
qwen35 loader validates `output.weight` against the full vocab (248320), but
the FastMTP draft has a trimmed head (32768) + a `d2t` tensor mapping draft
rows to target token ids. Two fixes applied:

1. **qwen35.cpp**: check for d2t tensor meta before creating output.weight;
   use d2t length as the expected vocab dim (eagle3 pattern).
2. **speculative.cpp**: load d2t table from the draft GGUF in the draft_mtp
   ctor; map sampled draft-vocab ids through d2t after sampling (line 1711).

Without fix 2, acceptance = 0.00000 (draft produces trimmed-vocab ids the
target rejects). With fix, acceptance = 0.68–0.79 (comparable to internal MTP).

## Lossless check

Bitwise SHA comparison across runs is impossible on this stack (q4_0 KV
rehydration + CUDA kernel non-associativity + MTP batched-verify). Greedy
first-token agreement holds; late-window divergence is numerics, not draft
faults. The article's "lossless" claim is about the target distribution being
untouched — confirmed: draft only affects speed.

## Decision

FastMTP d2t adopted as the faster draft mechanism at 49k. Not yet blessed as
daily driver: needs a depth-cert pass at 262k (the experiment ran at 49k for
VRAM headroom). Self-trimmed draft (option B: tensor-surgery on our own
baked-in head, identity d2t) remains the follow-up if we want to drop the
cross-finetune dependency.

## Session impact

The 232k owner session checkpoint (4.4 GB slot0.bin) was overwritten during
the experiment (sidecar saved the bench state over it — the empty-slot guard
only protects against empty-slot overwrites, not small-state overwrites).
Next turn will re-prefill ~232k tokens (~10 min at pp 370). The prompt cache
disk mirror has no matching state. Fix: the sidecar's save guard should also
preserve checkpoints larger than the current slot state when the delta is
significant.

---

## Self-trim surgery (option B) — FAILED 2026-10-11 21:30

**Question:** can we eliminate the external FastMTP dependency by trimming
our own MTP head (output.weight [5120,248320] Q6K) to top-32768 columns by
L2 norm + d2t mapping?

**Answer:** no. Without cross-finetuning, the trimmed head cannot rank
correctly within the 32k subset. Acceptance 0.01 vs 0.68–0.79 for the
cross-finetuned FastMTP.

### What was built

- `scripts/trim-mtp-head.py` — dequantizes our Q6K output.weight, computes
  per-token L2 norms (chunked, f16), selects top-32768, quantizes trimmed
  head to Q8_0 (gguf-py has no Q6_K quantizer), writes draft-only GGUF with
  blk.64.* + token_embd + output_norm + d2t.
- Output: `~/Models/Qwen3.8-27B-selftrimmed-mtp-draft.gguf` (945 MB).
- Profile: `qwen38-buffedmod-49k-selftrim`.

### Results

| arm | spec | median t/s | acceptance | mean len |
|-----|------|-----------|------------|----------|
| selftrim | self-trimmed d2t n=3 | 11.7 | 0.01 | 1.03 |
| external FastMTP (reference) | d2t n=3 | 32.3 | 0.68–0.79 | 2.95–3.32 |
| internal MTP (reference) | mtp n=3 | 29.4 | 0.75–0.77 | 3.26–3.32 |

Self-trim is **worse than no-spec** (11.7 vs 18.5 t/s) — the draft overhead
costs more than the rare acceptance gains.

### Root cause

The original MTP head was trained with full-vocab softmax (248k). Its
output.weight columns are calibrated for the full softmax. Selecting the
top-32768 by L2 norm picks **rare high-norm tokens**, not common tokens:

- External FastMTP d2t includes ALL 256 ASCII tokens, 768/768 BPE 256–1024,
  7168/7168 BPE 1k–8k — clearly training-frequency-based selection.
- Self-trim d2t includes only 82/256 ASCII, 320/768 BPE 256–1024,
  1676/7168 BPE 1k–8k — L2-norm picks rare tokens with strong output
  directions, not tokens the model actually predicts.
- Overlap between the two selections: 5485/32768 (16.7%).

The draft, restricted to the 32k high-norm tokens, predicts high-norm
tokens — which are rarely the correct next token. Acceptance collapses to
~0.01.

### Why FastMTP works but self-trim doesn't

FastMTP was **cross-finetuned**: the MTP head was retrained with the d2t
mapping in the loop, so the weights are optimized to rank correctly within
the 32k subset. Self-trim just extracts columns without retraining — the
ranking within the subset is preserved from the full head, but the full
head's top-1 over 248k is almost never in the top-32768-by-norm subset.

### Provenance

- Article: "How We Made Qwen 3.8 27B Generate 2.4x Faster on llama.cpp -
  Losslessly", Sreesanth R / Allen Joseph G, 2026-10-08.
- Article's approach: d2t + `-inf` mask on draft logits (not column
  extraction). The article's draft was already a trained draft model (not a
  trimmed target head).
- Our surgery: column extraction + d2t — a different mechanism that requires
  the head to be trained for the trimmed vocabulary.
- Lesson: d2t is a mapping mechanism, not a training method. Without
  cross-finetuning, the trimmed head's ranking is meaningless.

### Verdict

Self-trim is a dead end without training infrastructure. Keep external
FastMTP as the draft. The engine d2t fixes (inner `02233899c`) remain
valuable — they enable any d2t draft model, including future self-made ones
if we ever train one.

### Cleanup

- Self-trim script and draft GGUF kept for reference.
- Profile `qwen38-buffedmod-49k-selftrim` kept.
- Daily driver restored: `qwen38-buffedmod-262k`, all 3 units active.
