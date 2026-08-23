# Reference 1 — model loading & identity

Every flag that controls which model loads and how its tensors are shaped.
Defaults from `llama.cpp/common/arg.cpp`; REBIS notes measured on this rig.

PROVENANCE: extracted from arg.cpp registrations; REBIS notes measured
2026-08-21/22 (EXPERIMENT_LOG.md).

## `-m, --model PATH`

The model file to load. GGUF format. In REBIS: Sol =
`Qwen3.8-27B-UD-IQ2_S.gguf`, Luna = `Mellum2-...-IQ4_XS.gguf`. Both served
from a **private build** (`build-rebis`) because the shared `build/` tree is
managed by another pipeline that deletes binaries.

## `-mu, --model-url URL` / `-hff, --hf-file FILE` (+ `--hf-repo`)

Download-and-load variants. Useful for one-off pulls; REBIS pins local files
so head identity never changes under us mid-session.

## `-a, --alias NAME`

Sets the model id reported by `/v1/models` and used in OAI requests. The
gateway overrides this per-backend (`--luna-model/--sol-model`), which is how
hermes sees a single `rebis` id while two different GGUFs answer.

## Dtype / precision overrides

`--type-k`, `--type-v` are the KV-cache dtypes (see Reference 2). Weight
dtype comes from the GGUF itself — pick it at quantization time (UD-IQ2_S vs
IQ3_S choice moved Sol's prefill from 239→428 tok/s by eliminating PCIe expert
streaming entirely; see EXPERIMENT_LOG 2026-08-21).

## `--control-vector FILE[,...]`

Adds control-vector (activation-steering) adapters. Multiple allowed,
applied additively. Untested on this rig.

## Audio extras

`-mv, --model-vocoder` + `--tts-speaker-file` + `--tts-use-guide-tokens`:
TTS generation support in the common layer; unused by REBIS servers.

## Removed/deprecated aliases

`--draft*` family now errors pointing at `--spec-draft-*` replacements
(Reference 5). Old names die loudly so scripts fail visibly rather than
silently change behavior.

## Research notes

- Model *file choice* is the single largest performance lever we found:
  resident-vs-streaming changed prefill 1.7× at equal decode quality.
- Model identity strings flow through to clients — the gateway rewrites them
  so hermes only ever sees `rebis`.
