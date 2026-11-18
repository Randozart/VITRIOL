# 2026-11-18 14:06 — Optional client startup priming

`python3 scripts/vitriol-prime.py --url http://127.0.0.1:8080 --model MODEL_ALIAS --messages-file /path/to/system-messages.json --output /path/to/prime-result.json --timeout 30`

The messages file is a nonempty JSON array of text role/content messages shared by
future requests. The caller owns engine startup/readiness: invoke once before
accepting work, not periodically during an active conversation. The tool does not
start, restart or reconfigure the engine. No production profile/launcher changes.

It waits for health, renders two distinct user probes with the engine's own chat
template, tokenizes them, and evaluates only their shared prefix via native
/completion. One sampled token is discarded and never returned as an answer or
added to application history. Inference must finish without truncation and expose
matching cached/evaluated counters. A total deadline covers every request. Errors
return nonzero and do not publish a new ready file. Always check exit status; an
older result file must not be treated as current readiness after a failed run.

The JSON result includes full argv, context/prefix hashes, actual token counts and
elapsed time. A VITRIOL-FINGERPRINT line accompanies the invocation. Input messages
are not logged. Re-prime after changing the context/template/date or explicitly
recovering a displaced cache; do not claim that startup priming pins state against
later cache eviction. This is a text HTTP client tool, not an alternate inference
runtime, model-specific optimization or disk persistence guarantee.

Model-free tests: `python3 -m unittest libvitriol.test_prompt_prime -v`.
Actual same20B eviction and avatar playback validation is tracked separately by
.opencode/plans/client-prefix-primer-2026-11-18.md and the Avatar research report.

## 2026-11-18 14:32 — Validation scope

Current PTL CPU GPT-OSS20B: actual template/counter admission, partial displacement,
full private-process restart, resend/history/cancel recovery pass. Private Avatar
server3/3 and both real browser playback gates pass, with exact exported WAV/text
controls and40msmaximum drift. Startup prefix reuse does not guarantee fast later
execution under shared-memory pressure. No warm-turn speedup, home-GPU or full
8192token-depth certification is claimed. Before optimizing placement, compare
priming after other heavy initialization and measure working-set/pagefault costs.
No existing launcher/profile is changed by installation.
