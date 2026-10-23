# Engine provenance

The VITRIOL inference engine is a deeply modified fork of
[ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp), absorbed into
this monorepo on 2026-10-23. This file records exactly what was absorbed,
from where, and under what sync policy.

## Import record

| Field | Value |
|---|---|
| Import date | 2026-10-23 |
| Source commit | `30b128504ebb4ad1a4d9c8aa63d602cf84f07deb` (fork branch `stack-pull`, tag `engine-import-2026-10-23`) |
| Source tree hash | `058c67dc9245a540d91981d7db7fdd2d620672f4` (verified identical to `engine/` at import) |
| Upstream base | ggml-org/llama.cpp `master`, ported through 2026-09 (see fork merge history) |
| Fork divergence | ~1,801 commits ahead of the fork's upstream tracking point |
| Fork archive | `~/Backups/vitriol-engine-2026-10-23.bundle` (all refs) + `github.com/Randozart/llama.cpp` (frozen) |

## Upstream sync policy: FROZEN

As of 2026-10-23 the engine no longer tracks ggml-org/llama.cpp.

- No routine upstream merges. `upstream` merges are not planned; the fork
  repo is archived read-only.
- Security fixes and model-support fixes needed for VITRIOL operation are
  **ported manually** as individual patches, reviewed and committed to
  `engine/` with a `PROVENANCE:` note naming the upstream PR/commit.
- Model-support additions (new arch strings, new quant types) follow the
  same path.
- If a full resync is ever wanted, the fork's merge history (bundle +
  `Randozart/llama.cpp`) is the starting point; a fresh subtree graft from
  upstream would be a new, explicitly planned migration.

## What VITRIOL changes in the engine (top level)

- Sparse KV eviction with attention-probe scoring (`lull-kv`): vitriol-kv-probe,
  `evict_sparse`, eager floor, `llamacpp:kv_ejected_total` metric.
- KV residency tiers: RAM prompt cache (touch-LRU, hit-tracked,
  `VITRIOL_CACHE_KEEP`), Optane disk mirror, slot save/restore, warm resume.
- TurboQuant KV (`tq3_0`/`tq3_1s`/`tq3_4s`) + per-GPU KV quant overrides.
- MTP speculative decode fixes (qwen35_mtp arch, adaptive draft-K, verify
  histograms), EAGLE3/ngram draft paths.
- Context checkpoint ring (`--ctx-checkpoints`), checkpoint-every-N prefill.
- Server: latency observability (TTFT/ITL/queue/LCP histograms, CUDA-graph
  counters), grammar parse cache, X-Session-Id sticky routing, deferred-queue
  slot affinity, fair-share prefill, `/cache`, `/kv-policy`, `/ctx/resize`.
- CUDA: vitriol copy-engine probes, streaming mode (`VITRIOL_MODE`,
  research-only remap behind `VITRIOL_REMAP`), GDN perf counters.
- Server binary ships as `vitriol-server` (CMake target stays `llama-server`).

Per-change rationale lives in `.opencode/plans/` and per-file
`PROVENANCE:` headers.
