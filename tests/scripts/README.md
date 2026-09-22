# VITRIOL Long-Context Qualification Harness

Standalone depth-certification and KV-quant accuracy tests against the
`vitriol-server` llama.cpp engine (OpenAI-compatible `/v1` endpoints, default
port 8279).

## Provenance

Derived from `saas-home/Qwen3.8-27B-NV-16GB` `tests/scripts/`
(MIT License, Copyright (c) 2026 Mia's AI Lab), vendored 2026-09-21.
MIT notice retained; VITRIOL offer remains Apache-2.0 OR MIT.
Adaptations: default endpoint → llama-server port 8279,
`VITRIOL-FINGERPRINT` provenance embedding, `--blocks` depth scaling on the
stress tests, vision stage dropped (engine runs text-only).

## Flag-provenance contract

Every run MUST carry the engine fingerprint. The launcher prints a
`VITRIOL-FINGERPRINT:` line at start; export it before invoking:

```sh
export VITRIOL_FINGERPRINT='VITRIOL-FINGERPRINT model=... ts=... c=... kv=...'
python3 tests/scripts/<script>.py --fingerprint "$VITRIOL_FINGERPRINT" ...
```

`--fingerprint` also defaults to `$VITRIOL_FINGERPRINT`. Every JSON result
file embeds the string. A result without a fingerprint is invalid for
experiment records.

## Scripts

| script | use |
|---|---|
| `test_stream.py` | plumbing check: TTFT, prefill, decode on short prompts |
| `benchmark_context.py` | salted cold-prefill/decode ladder (epoch+UUID salt isolates cold prefill from cache hits) |
| `high_entropy_stress_test.py` | KV-quant accuracy boundary: 25-pair high-entropy recall + 8-hop dependency chain, `--blocks` scalable (~100 tok/block) |
| `extreme_precision_stress_test.py` | 14-stage ALU chain + 25-tx float ledger, `--blocks` scalable (~75 tok/block) |
| `intensive_accuracy_test.py` | code-gen + NIAH + math + JSON-schema 5-domain suite |
| `compare_benchmark.py` | 4-task coding/architecture suite |
| `test_concurrency.py` | multi-client stress (respects `parallel = 1` daily-driver config) |
| `llm_server_full_test.py` | 14-stage flagship; use `--test` subsets (skip 2 = vision) |

## Depth notes (27B dual-GPU)

- Blessed live config: ts 24,12 · q4_0/q4_0 KV · MTP n=1 · ctx 81920.
- Depth-cert comfort zone: ≤ 36k filled tokens (2026-09-04 re-cert).
- Historical wall: 54,692 tok on tq3_0 KV (q4_0 KV is heavier → shallower
  wall); re-probe only with headroom and expect SIGKILL transients.
- Accuracy arms (stress tests) should run with sparse-KV eviction OFF
  (fixed KV) so quantization is the only variable; throughput arms run the
  live config. Record which in the experiment report.
- **Thinking budget caveat**: llama-server defaults `enable_thinking` ON for
  the Qwen3 template; a reasoning model can spend the whole token budget in
  `reasoning_content` and emit empty `content` (2026-09-22: GSQ-RCO KV-recall
  "0/8" was this artifact; 8/8 with `--no-thinking`). Run KV/ledger accuracy
  tests with `--no-thinking` for stable protocol.
- **Multi-hop dependency test is not discriminative** for verbose-reasoning
  models: they compute intermediate XORs in long reasoning and exhaust the
  token budget before the final answer (fails identically on Q3_K_M and
  GSQ-RCO). Treat as design-artifact, not a quant signal.
