# Reference index — every flag, seven articles

Complete flag documentation for the REBIS stack: all 80 server-facing
llama.cpp flags plus the core common flags our launches depend on, and every
VITRIOL fork addition.

| article | covers |
|---|---|
| Reference 1 — model loading & identity | -m/-mu/-hf/-a, dtype choice, control vectors |
| Reference 2 — context & KV cache | -c, shift/reuse, cache types, VITRIOL prompt cache + checkpoints, RoPE/YaRN |
| Reference 3 — compute placement | -ngl/-ts/-sm/-dev/-fa, threads/affinity/priority/poll, batching, mlock |
| Reference 4 — sampling chain | temp/top-k/top-p/min-p, penalties, DRY, mirostat/dynatemp/xtc/nsigma, grammars & JSON schemas |
| Reference 5 — speculative & MTP | draft model family, native MTP, ngram family, removed aliases |
| Reference 6 — HTTP server surface | bind/auth/TLS, slots/metrics/webui/tools, template & reasoning controls, router extras |
| Reference 7 — misc & experimental | eval harnesses, imatrix prep, lookup decoding, logging |

Status legend used throughout: **[measured]** = benchmarked on this rig with
numbers in EXPERIMENT_LOG.md · **[incident]** = we ran it wrong and paid for
it (the most valuable entries) · **[documented]** = semantics from source,
untested here.

Start with `docs/REBIS_FLAGS.md` for the subset REBIS actually launches, then
these articles for everything else. The TUI GUIDE tab serves all of them.

PROVENANCE: descriptions extracted from arg.cpp registrations; REBIS notes
measured 2026-08-21/22.
