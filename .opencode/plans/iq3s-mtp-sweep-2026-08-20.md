# IQ3_S @ 131k — MTP-depth × tensor-split sweep (results)

Status: complete
Date: 2026-08-20

## Goal

Find fastest (MTP draft depth × tensor split) for Qwen3.8-27B-UD-IQ3_S @ 131k
context via VITRIOL (3060 12GB + 1070 Ti 8GB), used by Hermes for multi-hour
sessions.

## Setup

- Model: /home/randozart/Downloads/Qwen3.8-27B-UD-IQ3_S.gguf (12.04 GB)
- Context 131072, KV q4_0/q4_0, flash attn on, ctx-shift on, reasoning off
- spec.type=mtp (EMBEDDED head in IQ3_S — works; the separate
  mtp-Qwen3.8-27B-Q4_0.gguf head does NOT decode, lacks trunk layers)
- Benchmark: fixed Rust-coding prompt, warmup + 3 measured rounds of 64-token
  gen, t/s from server eval timings, VRAM via nvidia-smi

## Results

### ts 75,25
| draft_n_max | t/s |
|-------------|-----|
| 1 | 18.68 |
| 2 | 18.42 |
| 3 | 17.94 |
| 4 | 17.79 |
| 5 | 16.07 |

### ts 70,30
| draft_n_max | t/s |
|-------------|-----|
| 1 | 17.09 |
| 2 | **19.43** |
| 3 | 16.61 |
| 4 | 15.86 |
| 5 | 15.29 |

### Confirmation runs
- ts 70,30 d2 → **20.03 t/s** (re-run; stable)
- ts 75,25 d1 → 17.85 t/s (re-run)

## Winner

**ts 70,30 + draft_n_max=2 → ~20 t/s**

- The 70,30 split (more 3060 headroom: ~1.8 GB free with MTP) lets the fast
  Ampere 3060 dominate, with the 1070 as overflow.
- draft_n_max=2 is the MTP sweet spot at 70,30 — deeper (3-5) drifts and
  regresses (acceptance decays), matching the AGENTS.md finding for Q3_K_M.
- Note the interaction: at 75,25 depth-1 won (18.68); at 70,30 depth-2 won
  (19.43) — the split changes the MTP optimum. 70,30+d2 is best overall.

## Deliverable

Profile saved: `qwen38-iq3s-131k` (ts 70,30, mtp, draft_n_max=2, ctx 131072).

Verified: hermes tool-calling works (read_file /etc/hostname → Randy-PC),
20.58 t/s in the hermes run.

## Post-experiment candidates (from the alchemical toolkit list)

All native to this fork — nothing needs upstream pull:
- Prompt-lookup (--spec-type ngram-simple / ngram-map-k4v): 0-VRAM speedup for
  repetitive code; worth testing on top of the winner.
- GBNF grammar (--grammar-file): strict tool-call syntax determinism.
- LoRA (--lora): domain sidecars (briev-grammar, rust-expert).
- Parallel slots (-np): multi-agent (Trismegistus).
