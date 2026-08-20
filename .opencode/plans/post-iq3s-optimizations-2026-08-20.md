# Post-IQ3_S optimization tests — results

Status: complete
Date: 2026-08-20

## Baseline (winner from prior sweep)

IQ3_S @ 131k, ts 70,30, MTP draft_n_max=2, ~20 t/s. Hermes verified working.

## 1. Prompt-lookup / ngram speculative decoding — NOT adopted

- Tested `--spec-type ngram-simple` (wrapper spec.type). The server process
  died during warmup (no health, process gone). Log showed a Gated DeltaNet
  "layer 0 on CPU" warning and the model never finished warming.
- Verdict: ngram-simple is not a clean win on this rig; the MTP path already
  gives ~20 t/s. Not worth chasing further given the instability.
- ngram-map-k4v not tested (same family, same risk).

## 2. GBNF grammar — NOT adopted (by design)

- A global `--grammar-file` would force EVERY response into a single strict
  format, breaking plain-text agent answers (the agent must both answer
  questions AND make tool calls).
- VITRIOL already has the correct mechanism: the lazy tool-call grammar +
  PEG chat parser that engage ONLY on `<tool_call>` (verified working —
  hermes makes proper structured calls). This is strictly better than a
  global GBNF for a general agent.
- GBNF remains useful only for a dedicated single-output-type endpoint
  (e.g. a JSON-only code-completion route), not the agent chat endpoint.

## 3. Prefix / KV-cache reuse — CONFIRMED ACTIVE (the big win)

Two-turn test sharing a ~1806-word system prefix:
- Turn 1: prompt eval = 19,112 ms / 4,213 tokens (full prefill)
- Turn 2: prompt eval = 1,293 ms / 257 tokens (only NEW tokens!)

The 2nd turn processed only the 257-token diff; the ~3,956-token shared
prefix was reused from the KV cache. This is the vLLM `--enable-prefix-caching`
equivalent, and it is **already on by default** in this llama.cpp fork
(server prompt cache + `--cache-idle-slots` from the wrapper).

Implication for multi-hour sessions: the system prompt + repo context are
re-read once, then reused instantly on every follow-up — no per-turn prefill
lag. Nothing to enable.

## Excluded

- Parallel slots (`-np`): user runs 1 agent only.

## Recommendations (final stack for hermes)

- IQ3_S @ 131k, ts 70,30, MTP draft_n_max=2, KV q4_0/q4_0, flash attn, ctx-shift.
- Prefix caching already active — rely on it for long sessions.
- Skip ngram (unstable) and global GBNF (breaks plain answers).
- If ever needed, GBNF belongs on a dedicated single-format endpoint.
