# Dream-eval harness + rung 2-4 pipeline — plan (2026-11-21)

Owner's goal: rung 4 — models that modify their own weights over time —
approached through the prerequisite ladder, because rung 4 without rungs 1-3
is unverifiable weight poisoning.

## Architecture (the loop)

```
sessions + ledger ──> dream generator ──> gated corpus ──> QLoRA trainer ──> adapter
      ^                                                              │
      │                     dream-eval harness <─────────────────────┘
      └── outcomes (route ledger = ground truth) <── shadow lane (slot 1)
```

Everything is data through existing infrastructure: the lane registry serves
adapters/models, the ledger provides ground truth, dual-slot provides the
shadow lane.

## Stages (each independently useful)

**S1 — dream-eval harness** (`dream-eval` extension + `dream-eval-core.ts`).
The gate everything else stands on:
- Frozen held-out set: sampled from the route ledger + MEMORY facts, split
  train/eval 90/10, eval frozen by hash (I-E1: same hash = same eval, forever;
  hash printed in every report).
- Runner: any GGUF/adapter endpoint (`--base-url`, `--model`, `--lora`) →
  score = pass rate on the eval set. Judged by the SAME verify machinery as
  the sleep lane (coverage + numbers + parrot guard) plus exact-match for
  commands.
- Report: JSON + markdown, model id, eval hash, per-question results.
- Promotion rule: candidate ≥ baseline pass-rate AND no regression on any
  "protected" question class (commands, numbers).

**S2 — dream generator** (extends sleep-lane).
From ledger rows + session transcripts, generate practice pairs:
- fix-pattern pairs: "symptom → root cause → fix" from turns with errors that
  were subsequently fixed.
- convention pairs: from verified MEMORY facts (already human-blessed).
- environment pairs: hardware, config, toolchain facts with numbers.
Each pair carries provenance (session, ledger row) and passes the I6
injection-resistant framing. Corpus = JSONL, versioned by content hash.

**S3 — small-model lane trial** (proves the promotion path with NO training).
Pick an existing small GGUF (e.g. a Qwen3.6-8B), run the eval, add it as a
lane ranked after Lapis for in-domain turns where the eval shows parity.
This validates: lane registry promotion, shadow-lane evaluation, ledger
ground truth — the entire loop — with zero training risk.

**S4 — QLoRA trainer** (offline script, NOT in the serving process).
QLoRA on a 4-8B base with the gated corpus; runs on one 3060 overnight
(ShellStart). Output: adapter file (GGUF-compatible lora). The trainer is an
external script in `scripts/dream-train/` — llama.cpp serves adapters, it
does not train.

**S5 — adapter shadow promotion.** Adapter loaded via `--lora` on slot 1;
shadow-bench vs base on the eval; promotion = lane registry rank change.
Rollback = adapter file removal. Base GGUF is NEVER modified (the paper's
"synaptic pruning" = adapter rotation).

## Invariants

- **I-E1 frozen eval**: the eval set is content-hashed; every report embeds
  the hash; a changed hash means a different eval and a reset baseline.
  Pinned by hash-stability test.
- **I-E2 train/eval separation**: no train-corpus row may appear in the eval
  set. Pinned by disjointness test.
- **I-P1 promotion is measured**: a lane is promoted only with an eval report
  showing ≥ baseline pass-rate and zero protected-class regressions. Pinned
  by promotion-rule unit tests.
- **I-P2 base weights immutable**: the base GGUF file is never written by any
  part of the pipeline; adapters are separate files. Pinned by hash check
  before/after training in the trainer script.
- **I-P3 ledger is ground truth**: dream answers must be traceable to a
  ledger row or MEMORY fact; untraceable samples are dropped. Pinned by
  provenance test.
- **I-P4 the loop is reversible**: removing an adapter/lane restores prior
  behaviour exactly. Pinned by lane-registry test.
- **I-P5 injection resistance**: dream corpus prompts inherit the sleep
  lane's quoted-DATA framing. Pinned by prompt-shape test (I6 inherited).
