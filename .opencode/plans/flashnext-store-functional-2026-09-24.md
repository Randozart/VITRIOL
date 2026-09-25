# Flash-Next Optane Store: Functional Validation Plan

Date: 2026-09-24 22:50 CEST.
Status: active.
Supersedes exact-token-equivalence gates in
`.opencode/plans/flashnext-store-correctness-2026-09-24.md`.

## Primary goal

Run Qwen3.8-Flash-Next with routed experts streamed from the Optane raw-store
partition so the model is functional within 2x RTX 3060 12GiB and limited host
RAM. Exact token equality with GGUF mmap is not required.

The direct-to-VRAM architecture is:

`Optane O_DIRECT -> 512B pinned bounce -> LRU VRAM -> CUDA compute`

Store mode must not mutate routed host expert tensors.

## Why exact equivalence is retired

The transport layer has already proven byte-exact behavior:

- store bytes matched GGUF;
- all sampled bounce bytes matched GGUF;
- H2D and VRAM D2H readbacks matched;
- first-layer routing matched the GGUF arm.

Remaining small logit or token differences can come from CUDA scheduling,
kernel reduction order, warmup state, memory alignment, or model sampling
behavior. Hash or token equality is therefore a debugging tool, not a product
acceptance criterion.

## Acceptance gates

### Gate A - Transport correctness

Must remain exact:

- 144 routed tensors bound for Flash-Next;
- descriptor geometry validated;
- zero O_DIRECT or CUDA failures;
- zero NaN/Inf during instrumented tests;
- no all-zero expert activations.

### Gate B - Functional generation

Use a reusable deterministic prompt suite covering:

1. short factual answer;
2. arithmetic;
3. instruction following;
4. code generation;
5. explanation;
6. constrained output;
7. multilingual or translation;
8. longer reasoning.

For each prompt:

- 48-128 new tokens;
- finite response;
- no prompt echo as the entire answer;
- no garbage, whitespace-only result, or obvious decoder corruption;
- output suitable for practical use;
- full command, prompt, response, timing, and counters recorded.

A response need not match GGUF token-for-token.

### Gate C - Stability

Run the suite or a representative subset three times. Require:

- no crashes or CUDA errors;
- no unbounded VRAM/RSS growth;
- zero store read failures;
- clean shutdown counters.

### Gate D - Performance sanity

After Gate B/C:

- one warmup plus 3 x 64-token decode measurements at shallow context;
- record predicted tokens/second, prompt tokens/second, VRAM, store bytes,
  store fills, failures, and LRU counters;
- compare qualitatively against the GGUF mmap baseline.

Depth-ladder testing remains blocked until Gates A-C pass.

## Cleanup scope

Remove diagnostic sprawl before functional testing:

- remove default-off bounce/VRAM hash dumping;
- remove legacy compatibility env variables
  `VITRIOL_STORE_SKIP_GGUF` and `VITRIOL_STORE_FILL`;
- keep canonical `VITRIOL_STORE_SOURCE=gguf|store`;
- keep useful counters: fills, bytes, failures, read time, LRU hits/misses/
  evictions;
- preserve bounded dynamic lock state, exact descriptor checks, buffer-owned
  store lifecycle, bounce/event lifetimes, and max-expert LRU sizing.

## Operational constraints

- Do not modify partitions.
- Do not pursue D2b work.
- `vitriol-server.service` remains stopped/disabled during experiments; its
  configured model path is currently missing.
- No push.
- No unrelated upstream cleanup.
- Commit only after explicit user approval.

## Immediate execution sequence

1. Update this plan.
2. Remove diagnostic hash/dump code and legacy env aliases.
3. Build `llama-server`; run standalone store test.
4. Add functional prompt-suite script under `scripts/`.
5. Launch fresh store-mode server on an isolated port.
6. Run Gate B prompt suite.
7. Capture counters, resource samples, and all responses in this report.
8. If functional gates pass, request commit approval before depth/perf work.

## Execution status - 2026-09-24 23:55 CEST

Cleanup completed:

- removed bounce/VRAM hash probes and legacy env aliases;
- canonical `VITRIOL_STORE_SOURCE=store|gguf` remains;
- clean build succeeded;
- standalone store test: `ALL PASS`.

Added `scripts/vitriol-store-functional-suite.py`. It performs eight varied
deterministic requests per round and checks for empty/prompt-echo/non-printable
or degenerate output.

Fresh store run used:

```text
binary = llama.cpp/build/bin/llama-server
model = /scratch/qwen38fn/Qwen3.8-Flash-Next-UD-Q3_K_XL-00001-of-00003.gguf
argv = -ngl 99 -ts 12,12 --main-gpu 0 -c 512 -ub 64 -fa on
       --host 127.0.0.1 --port 8404 --parallel 1 --slots -t 8
env = VITRIOL_MODE=stream,
      VITRIOL_MAX_LOCKED_MB=8192,
      VITRIOL_STORE=1,
      VITRIOL_STORE_PATH=/dev/nvme0n1p3,
      VITRIOL_STORE_SOURCE=store,
      VITRIOL_VERBOSE=1
```

Gate B result: **FAIL**.

- requests: 24 (3 rounds x 8 prompts);
- quality failures: 24;
- first response repeated `"The capital of France is ..."`;
- most later responses were token 0, `"!"`, repeated;
- final counters: 54,529 store fills, 44,151,296,000 bytes, zero failures;
- LRU: 710,305 hits, 54,529 misses, 53,296 evictions;
- clean SIGINT shutdown, zero orphan registrations.

Resource peak observed during the suite: approximately 13.3 GiB RSS and
6.6 GiB swap. GPU use was roughly 5.8 GiB + 3.5 GiB. The host is over its
comfortable memory envelope and needs a lower-RSS launch or reduced suite
concurrency in future runs.

Interpretation: this is not random sampler behavior and not an Optane transport
failure. The model enters a degenerate state during generation. Current
suspicion is stale/incorrect LRU slot reuse or an interaction with reused
compute state, not storage bytes. Exact transport had previously passed for a
short request.

## Follow-up isolation - 2026-09-25 00:25 CEST

Additional tests ruled out several hypotheses:

- synchronous store H2D still produced the same degenerate output;
- a temporary no-cache store path still failed;
- reducing `VITRIOL_LRU_MB=2` still degraded after the first useful token;
- forcing every store access to reload still failed.

This moves the defect downstream of storage bytes, bounce correctness, H2D
visibility, and simple cache reuse. A matched FFN probe had already shown that
`src1` differs between GGUF and store arms even when the compared weight bytes
match. The next diagnostic should therefore dump tolerance-aware activation
statistics (`NaN`, `Inf`, all-zero count, min/max, L2 norm) for common/shared
tensors and the first differing FFN input, not hashes.

All temporary force-reload diagnostics were removed. The normal LRU path is
restored and the server builds cleanly. No experimental server remains
running.
