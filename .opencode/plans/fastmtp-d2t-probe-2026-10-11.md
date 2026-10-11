# FastMTP d2t probe experiment (draft-vocab trimming vs internal MTP)

- **Date:** 2026-10-11 13:0x
- **Status:** approved by owner ("always willing to experiment - worth
  downloading and at least trying it out")
- **Trigger:** owner-shared article: "How We Made Qwen 3.8 27B Generate 2.4x
  Faster on llama.cpp - Losslessly" (Sreesanth R / Allen Joseph G, 2026-10-08,
  FastMTP / d2t draft-vocab trimming).

## The claim (article)

Trim the MTP draft LM head from 248,320 to top-32,768 vocab columns
(+ `d2t` id-mapping tensor + `-inf` masking) -> draft head matmul -87%.
43 -> ~105 t/s (2.4x) on one RTX PRO 6000 Blackwell, N=3, 82.9% acceptance
on coding traffic. Lossless: spec-decode acceptance changes throughput only;
target distribution untouched; greedy SHA-256 identical with/without draft.
Row-split breaks d2t; layer-split works.

## Applicability to VITRIOL (2x RTX 3060, layer split, blessed 25.7 t/s)

- Fork ALREADY supports it: `--spec-type draft-mtp` (external draft),
  `LLM_TENSOR_D2T` (`llama-arch.cpp:689`), d2t field (`llama-model.h:672`),
  `find_best_mtp` auto-download (`common/download.cpp:784`).
- Draft exists only for OTHER finetunes:
  `HauhauCS/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-MTP-GGUF` ->
  `Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-FastMTP-32K.gguf` (0.9 GB).
  No FastMTP draft for our ISTA GSQ-RCO-BuffedMod lineage -> cross-finetune
  acceptance is THE unknown this probe measures.
- Honest gain math: on their card the draft head dominates decode traffic;
  here the 11.5 GiB target pass (2 cards + PCIe) dominates. Internal head
  (Q6K, ~1 GiB/draft-step, n=3) is ~25% of cycle bytes -> trimming to ~3%
  caps the win near ~10-20% IF acceptance holds. NOT 2.4x.
- Post-trim, larger n_max gets cheap (their N=5 beat N=3).

## Experiment design

**Protocol**: fresh restart per arm, 1 warmup + 3 measured generations
(n_predict=64, temperature=0, ~1k-token prompt), engine `timings` fields,
same binary, same ts 27,21, ctx 49152 (dev1 VRAM headroom for the draft;
shallow numbers are depth-independent). Governor STOPPED during the
experiment (its RESIZE tier would try to grow the 49k window back to 208k
and blow VRAM with the draft resident). Live config untouched - profiles
only; daily driver restored after.

- **Arm A (control)**: internal MTP (`--spec-type mtp --spec-draft-n-max 3`),
  the blessed mechanism, at 49k.
- **Arm B (probe)**: `--spec-type draft-mtp --spec-draft-model
  ~/Models/Qwen3.8-27B-Uncensored-HauhauCS-Aggressive-FastMTP-32K.gguf
  --spec-draft-n-max 3`, same everything else.

**Metrics**: `timings.predicted_per_second` (3-run median), prompt t/s,
speculative acceptance if surfaced, VRAM per GPU, greedy-output SHA-256
across arms (lossless sanity: A vs B vs no-spec must agree on n=64 greedy).

**Download integrity**: ranged fetcher + tree-API `lfs.oid` sha256 check
(AGENTS rule 6).

## Decision gates

- B acceptance collapsed (< ~50% of A's effective speed) -> cross-finetune
  draft rejected; note that a self-trimmed draft (option B: tensor-surgery
  on OUR OWN baked-in head, identity d2t, no retrain) is the follow-up if
  the mechanism is worth pursuing.
- B >= A within noise -> adopt pending a depth-cert pass; bless via
  `[spec]` launcher keys + fingerprint (`spec=draft-mtp:...`), new profile.
- Anything crashes/aborts -> capture, diagnose, no half-wiring.

## Execution checklist

1. Plan doc (this file) - commit
2. Fetch FastMTP-32K draft, sha256 verify
3. Reusable bench script `scripts/bench-shallow.sh`
4. Experiment profiles: `qwen38-buffedmod-49k-internal` / `qwen38-buffedmod-49k-fastmtp`
5. Stop governor -> arm A restart -> bench -> arm B restart -> bench
6. Lossless SHA check
7. Restore daily driver (262k profile) + restart + warm canary
8. Report + commits (per stage)
