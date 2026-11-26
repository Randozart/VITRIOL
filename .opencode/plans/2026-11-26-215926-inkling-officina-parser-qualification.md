---
schema: "codex-work-plan/v1"
created_at: "2026-11-26T21:59:26+01:00"
timezone: "Europe/Amsterdam"
status: "completed"
baseline_commit: "e96927203f752fb9c5b4a3fd3ebc66094d0e9608"
supersedes: []
superseded_by: []
related: [".opencode/plans/2026-11-26-214613-officina-rust-port-model-comparison.md"]
---

# inkling-officina-parser-qualification

## Purpose

Find a reproducible Inkling 2 Mini setup for local Officina using the existing IQ4_NL model and both RTX 3060s.

## Approach and alternatives

Compare VITRIOL generic parser evidence to the official llama.cpp Inkling parser and fixes. Qualify a pinned upstream CUDA build as a scratch endpoint, without replacing the installed engine or production profile. Reuse the GGUF, run structured API and real Officina code-edit probes, and record fitting layer placement.

- Prefer a stock upstream endpoint to a parser backport while qualifying
  the behavior. VITRIOL's current generic parser returned raw tool syntax.
  A new GGUF or alternate runtime would change more than one variable.

## Scope and documentation

This is a local qualification. The verified upstream package, launch
script, private Officina catalog, test workspaces and evidence are under
`/tmp/vitriol-inkling-upstream/`. The existing IQ4_NL file is reused
read-only. The production engine, model profile and user model catalog
are outside the experiment.

## Verification

- Confirm upstream package checksum, direct tool-call schema, post-tool reply, selected local model, mutations and Cargo outcomes in scratch, both GPU memory and final release. If the new endpoint cannot fit, preserve exact error and compare smaller window or measured layer distribution.

## Invariants

1. Tool output counts only if the server emits a declared tool with the correct JSON arguments and Officina executes the corresponding scratch tool. Plain-text syntax is recorded as failure.
2. The official upstream release runs on a dedicated loopback port and reuses the same model file as the failed VITRIOL run. Test processes, provider settings and generated edits are isolated from production.
3. The benchmark records actual context, free VRAM and the exact tested launcher. Additional quants, context lengths and speculation remain separate hypotheses.

## Results and evidence

### Source findings

VITRIOL's architecture port includes the Inkling loop implementation.
The previous endpoint logged `peg-native`, a shared format label, and
returned Inkling's tool delimiters as response text. The current vendored
chat sources lack upstream's Inkling template detector and token parser.
Upstream added its dedicated parser in #32381 and switched it to explicit
token recognition and scanning grammar in #32833. The latter fixed the
earlier grammar trigger failure. Automatic mode can still generate a
tool whose name was not declared, as the open #32966 report and the
release parser's any-name branch show. The fixtures exercise one declared
read, the post-tool answer and a normal answer with tools offered.
[Dedicated parser](https://github.com/ggml-org/llama.cpp/pull/32381) ·
[Parser update](https://github.com/ggml-org/llama.cpp/pull/32833) ·
[Pinned source](https://github.com/ggml-org/llama.cpp/blob/b12774/common/parsers/inkling.cpp) ·
[Automatic name issue](https://github.com/ggml-org/llama.cpp/issues/32966).

Thinking Machines reports 71.6% on Terminal-Bench 2.1 using Pi v0.83.0
and describes a risk of repetitive debugging reasoning. The GGUF card
documents the 17.6 GB IQ4_NL and three passes through the middle layers,
each with its own KV and convolution state. These support trying the
model with real tools and a modest context before optimizing. Third-party
smaller quants, DSpark and other runtimes were considered as later
comparisons. This qualification uses no new weights or speculative draft.
[Model card](https://huggingface.co/thinkingmachines/Inkling-2-Mini) ·
[GGUF card](https://huggingface.co/ggml-org/Inkling-2-Mini-GGUF).

### Runtime and Officina

The official v1.1.0 release points to build b12774. The Linux CUDA 13.5
archive matched its GitHub asset checksum and loaded with the local
CUDA runtime on NVIDIA 620.73.04. The executable reports build 12774,
commit `171527522`. [Release](https://github.com/ggml-org/llama.cpp/releases/tag/v1.1.0) ·
[Binary assets](https://github.com/ggml-org/llama.cpp/releases/tag/b12774).

At 32,768 configured tokens the 45:55 layer split tried to allocate
14,799.80 MiB on GPU0 and exited. The 1:3 split, ub32, F16 KV and
`reasoning_effort=minimal` passed readiness on both GPUs. Observed free
memory after initialization was 579 MiB on GPU0 and 1,503 MiB on GPU1.
The repeatable scratch launcher now defaults to the verified split.

The tested path for Officina is a dedicated `openai-completions`
provider pointing at `http://127.0.0.1:18569/v1`, with model ID
`inkling-officina-trial` and advertised context 32,768. The launcher
sets minimal effort via `--chat-template-kwargs` so the client does
not send a nonstandard reasoning value. The host had the CUDA 13
runtime libraries under `/opt/cuda/lib64`; no driver changes were needed.
This is an on-demand alternative on this host since both cards are
occupied by the current quant.

The direct tool probe returned the declared `read_fixture`, exact
`{"path":"fixture.txt"}` arguments and `AMBER-3060` after the tool
response. It also returned clean `4` on a normal turn with optional
tools. Under the installed Rust Officina TUI, the same scratch Caduceus
port task took 141.26 s. It successfully used read, grep, ls and edit,
and two calls to an undeclared `shell` were rejected by the harness.
The first submitted Rust implementation passed all 11 Cargo tests.
It generated 1,719 tokens over the turn with 8,894 prompt-plus-output
tokens reported on the last request. Kintsugi had passed the same
tests after one repair, taking 46.00 s and 16.37 s of active turn time.
These are two observed workflows with different model backends and
GPU placements. No inference about model-wide correctness follows.

Evidence under `/tmp/vitriol-inkling-upstream/evidence/` includes
`package-check.log`, the two server fingerprints, the API probe
responses, `inkling.first.json`, `officina.cargo.log` and GPU readings.

### Filled-depth check

2026-11-26T22:07+01:00. A synthetic Rust-source prompt filled 24,055
tokens and recalled `BIRCH-871` exactly. The cold prefill measured
155.18 tok/s over 155.01 s. Generation used 182 tokens, including
reasoning, at 11.62 tok/s. GPU0 and GPU1 had 471 and 1,369 MiB free
after the probe. This qualifies one recall sample above 24K. Agent
work above the observed approximately 9K task, larger windows
and concurrent desktop demand remain open. See `api-24k-recall.json`,
`depth.run.log` and `gpu-after-24k.csv`.

The Officina driver and the temporary server stopped cleanly. The
production engine and autosave began and ended inactive. No sudo,
new model weights or production catalog edits were needed. Run
changed-scope verification against `e969272`, with review and
artifacts under `/tmp/vitriol-inkling-upstream/`.

## Change history

- 2026-11-26T21:59:26+01:00 — Plan created before implementation.
- 2026-11-26T22:07+01:00 — Completed the upstream parser, Officina and 24K recall comparison.
