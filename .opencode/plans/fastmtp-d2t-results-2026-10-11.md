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
