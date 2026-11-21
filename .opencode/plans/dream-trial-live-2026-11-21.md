# Dream pipeline rung 3 — LIVE small-model lane trial (2026-11-21 17:36)

Plan: `dream-rung2-3-plan-2026-11-21.md` (D2). Owner topology call: tiny
local GGUF served via a **base swap on the live engine** — snapshot config,
restart small, score against the frozen eval, restore snapshot byte-exact,
restart back to the 27B. The 27B GGUF is never written (I-P2).

## What ran

| step | command | result |
|---|---|---|
| swap-in | `scripts/dream-trial.sh swap-in` | snapshot `~/.vitriol/config.pre-dreamtrial` sha256 `9396621b…c5a54e`; engine restarted on qwen2.5-0.5b-instruct-q4_k_m (491,400,032 B) |
| fingerprint (trial) | `~/.vitriol/fingerprint.last` | `model=qwen2.5-0.5b-instruct-q4_k_m.gguf ts=none c=16384 kv=q8_0/q8_0 ub=64 par=1 spec=none:0 cram=1024 cmax=8` |
| served identity | `GET /v1/models` | `Dream Trial` |
| seed eval | (auto) | 16 items, hash `d6acf49523b3`, from route ledger |
| score | `scripts/dream-trial.sh score` | **0.0%** — `dreamtrial-2026-11-21T173559+0100.json` |
| probe | one `/completion` | 289.2 t/s; content `" A. 1 B. 2 C. 3 D. 4\nAnswer:\n\nC\n\n…"` (hallucinated multiple-choice) |
| swap-out | `scripts/dream-trial.sh swap-out` | config restored sha256 `9396621b…c5a54e`; engine back on 27B |

## Verification of reversibility (I-P4)

- `diff -q ~/.vitriol/config ~/.vitriol/config.pre-dreamtrial` → **identical**.
- Restored config sha256 `9396621b23aea7284e1808e4c5289149ce480d4f17db5bfe4a38b3a066c5a54e`
  equals the pre-trial snapshot hash recorded at swap-in.
- Live fingerprint after swap-out is byte-identical to the pre-trial live
  point:
  `model=Qwen3.8-27B-GSQ-RCO-BuffedMod-IQ3_S_XL-mtp-Q6K.gguf ts=27,21 c=262144
  kv=q4_0/q4_0 fa=on ub=128 mode=off engine=vitriol-dma ckpt=8192 ckpts=4
  lru=0 sparse=sparse score=probe pool_reset=0 spec=mtp,ngram-simple:2 par=2
  ssp=1 pr=0 cram=16384 cmax=40 cxr=1 ct=chat-template-tolerant.j2 cis=0`.

## Findings

### F1 — the swap machinery works end to end
Snapshot → trial config → restart → serve → score → restore → restart all
succeeded, with a byte-exact restore verified by sha256. The trial is a
**reversible, measured** experiment, not a mutation.

### F2 — 0.0% is a REAL miss, but the seeded eval is weak
Every one of the 16 seeded items asks the *same* question ("Which models
has the routing ledger served more than five turns?") and differs only in
its `mustContain` model string. The 2-item eval split requires
`opencode-go/deepseek-v4.` — which a 0.5B instruct model cannot know. The
0.5B answered with hallucinated multiple-choice (" C").

Mechanically valid (requests succeeded, judge scored real outputs), but the
ledger-model-count heuristic produces a degenerate eval. **This is exactly
the gap the dream generator (D1) fills**: `extractQA` pulls diverse,
answer-shaped question/answer pairs from real transcripts, not one repeated
ledger question. The trial's headline number is therefore *not* a fair
small-model capability baseline — it is a harness smoke test.

### F3 — tiny-model throughput
289 t/s decode on the 0.5B (27B class ≈ 16–25 t/s). Confirms the small
lane is cheap enough for offline distillation cadence; capability is the
constraint, not speed.

## Consequence for rung 4

Rung 4 (QLoRA + shadow-promote) needs a **real** eval set. Next step is to
seed the eval from a `corpus-<hash12>.jsonl` produced by `/dream-gen`
(diverse excerpted pairs) rather than purely from the ledger, then re-run
the trial to get a meaningful baseline before any adapter is trained.

## Invariants honoured this trial
- **I-P2** base immutability: 27B GGUF untouched (only the config path moved).
- **I-P4** reversible snapshot: sha256-verified byte-exact restore.
- **I-P1** measured promotion: report only, nothing promoted.
