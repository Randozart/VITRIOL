# Reference 4 — the sampling chain

Every knob that shapes which token comes next. Order matters: samplers run
in `--samplers` order (default: penalties → top-k → typical → top-p →
min-p → temperature).

PROVENANCE: arg.cpp semantics; REBIS defaults measured 2026-08-22.

## The core four

| flag | default | does |
|---|---|---|
| `--temp` | 0.80 | scales logit distribution; 0 = greedy |
| `--top-k` | 40 | keep only k highest-probability tokens |
| `--top-p` | 0.95 | nucleus: smallest set with cumulative p |
| `--min-p` | 0.05 | floor relative to top token probability |

REBIS: drafter turns ship JetBrains-recommended **0.6 / 20 / 0.95** via
shim setdefaults; audits and verdicts run **temperature 0.0** for determinism.

## Repetition control

| flag | default | does |
|---|---|---|
| `--repeat-penalty` | 1.0 (off) | multiplies repeated-token logits |
| `--repeat-last-n` | 64 | window for repeat detection (-1 = ctx) |
| `--presence-penalty` | 0.0 | flat penalty once a token appeared |
| `--frequency-penalty` | 0.0 | scaled by occurrence count |
| `--dry-penalty-last-n` | 0 | DRY: penalizes exact multi-token sequence repeats — strongest anti-loop tool we have |

## Exotic

`--typical-p` locally-typical · `--top-nsigma` σ-based trim ·
`--xtc-probability/--xtc-threshold` excise top tokens occasionally (creative
writing) · `--dynatemp-*` dynamic temperature curve · `--mirostat*`
adaptive perplexity control (disables top-k/top-p/typical) ·
`--logit-bias TOKEN±W` manual token push/pull.

## Structured output

`--grammar-file GBNF` / `-jf --json-schema-file`: constrain generation to a
grammar. The gateway's constrained verdicts use the server-side
`json_schema` field of `/completion` (same engine, per-request instead of
per-launch). This is what makes REBIS verdicts parseable *by construction* —
no more unparseable-auditor failure mode, and verdict token counts dropped
from 8192-cap rambles to 150–570 tokens.

## Seed & length

`-s/--seed` (random by default) · `-n/--predict` completion length cap.
Gateway sets explicit caps so one turn can't eat the window.

## Research notes

- Verdict determinism matters more than diversity: audits at temp 0.2 chat
  still rambled; temp 0 + schema fixed it completely.
- Luna no-think at temp 0.6 on trivial prompts produced degenerate
  newline-loops twice in testing — short prompts need either thinking ON or
  very tight max_tokens when serving raw completions.
