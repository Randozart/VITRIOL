# Serving-stack pull plan — SGLang/vLLM ideas into VITRIOL (2026-10-23)

**Branches**: outer `stack-pull`, inner `stack-pull` (both off `main`).
**Mode**: build session with GPUs busy — **no engine runs, no benchmarks,
no GPU tests**. Code + CPU-only checks (compile, `cargo build/test` on the
isolated crate) only. Live gates deferred to the first maintenance window.

## Provenance policy (owner directive, 2026-10-23)

Copies get `PROVENANCE` headers; rewrites list provenance anyway. Incoming
sources this plan touches: vLLM (Apache-2.0), SGLang/RadixAttention
(Apache-2.0), llguidance (MIT), xgrammar (Apache-2.0). All LICENSE-OK per
the AGENTS table. Convention:
`// PROVENANCE: inspiration — <repo> (<license>), <what was learned>, not copied`.

## Rust preference (owner directive)

New standalone logic lands in Rust crates with C ABI shims (pattern of
`vitriol-stream`, `vitriol-governor`). In-process C++ containers that would
cost more in FFI churn than they save stay C++ but note their Rust-shaped
API so a later port is mechanical.

## What VITRIOL already has (do NOT port)

Continuous batching + chunked prefill with fair-share; whole-sequence RAM
prompt cache + Optane disk mirror; MTP/EAGLE3/ngram spec decode + adaptive K;
q4_0/tq3_0 KV; GBNF + JSON schema; runtime KV resize; sparse probe eviction;
cooperative VRAM governor; slot checkpoints + warm restore; X-Session-Id
sticky routing; `--cache-reuse` non-prefix chunk shifting (upstream).

## Stages

### S0 — branches + this plan (done 2026-10-23 12:00)
- Inner: committed the uncommitted `VITRIOL_REMAP` remap-gate leftover
  (`d6f9eb0fe`) that outer `8b0042e` already claimed — closing that gap
  before branching so `stack-pull` starts clean.
- Outer+inner branched `stack-pull` off `main`.

### S1 — observability floor (vLLM/SGLang metrics parity)
Everything below is `/metrics`-visible; unit suites in
`tools/server/tests/unit` updated. **All prefixes/labeled series follow the
existing `llamacpp:` naming.**
- TTFT + ITL + queue-wait histograms (bucketed counters, not summaries):
  task `t_post` at post time; TTFT = first token out − t_post;
  queue-wait = slot claim − t_post; ITL from `t_gen_last` deltas.
- Cache effectiveness: hit-ratio gauge (`n_prompt_cached / (cached+processed)`),
  LCP-length histogram buckets at slot launch (per-request reuse depth),
  per-entry hit counts + `t_last_hit` on `server_prompt_cache` states
  (surfaced via `/cache`).
- Sampling/grammar time: per-request accumulated us inside
  `common_sampler_sample*` path + global `sampling_seconds_total` — the
  signal that decides whether the S2 grammar cache is worth its keep.
- CUDA-graph counters (vLLM "capture checklist" observability): global
  atomics in the ggml-cuda graph map — captures, reuse hits, in-place
  updates, evictions — exposed via `ggml-cuda.h` + surfaced as
  `graph_{captures,reuse,updates,evictions}_total`.
- PROVENANCE: vLLM (Apache-2.0) metrics.py / scheduler metrics; ideas only.

### S2 — grammar parse cache (llguidance/xgrammar insight, GBNF-native)
`llama_grammar_init_impl` re-parses the grammar string on every sampler
build (`llama-sampler.cpp:2688-2718`); agent loops rebuild per request and
per reset. Add a process-wide immutable parse cache keyed by the grammar
string (+ vocab-independent rules AST), with per-request mutable state built
from the cached AST as today. No behavior change; hit/miss counters ride
S1's sampling metrics. No jump-forward yet — that is P3-class work needing
GPU gates; this is the safe subset.

### S3 — prompt-cache eviction: FIFO → touch-LRU + hit counters
`server_prompt_cache::update()` evicts `pop_front()` (insertion order) and
`load()` *consumes* the entry it restores (`server-task.cpp:2004-2036`).
Port the LRU half of vLLM's block-pool policy (~ARC comes with the Rust
index in S5): keep restored entries resident (drop the consume), track
`n_hits`/`t_last_hit`, evict least-recently-hit under the `cache_ram`
budget. Identical-resend then becomes a pure hit instead of
save→consume→re-save churn. `/cache` shows the new fields.

### S4 — launcher: `cache_reuse` key + llguidance opt-in
- Wire `server.cache_reuse` → `--cache-reuse N` (upstream flag exists; the
  key was parsed but never emitted — `scripts/vitriol:414` dead-ends), add
  `cr=<n>` fingerprint field, template default 0.
- `scripts/build-llama-server.sh`: `VITRIOL_LLGUIDANCE=1` adds
  `-DLLAMA_LLGUIDANCE=ON` (Rust cargo build dep; opt-in, default off).

### S5 — `vitriol-cache` Rust crate: radix prefix index + ARC (NEW CODE)
Rust crate + C ABI, **not wired into the server this session** (wiring is
the first GPU-window stage; until then it is inert library code with its own
CPU tests).
- Content-hash chained radix trie over token IDs (SGLang RadixAttention
  shape): insert(prefix) → node, match_prefix(tokens) → (node, depth),
  refcount-style pin/unpin, LRU-with-pins eviction walk.
- ARC eviction policy (vLLM `kv_offload/cpu/policies/arc.py` shape,
  ~150 L, re-derived).
- C ABI: `vc_new/vc_free/vc_insert/vc_match/vc_pin/vc_unpin/vc_evict/vc_len`.
- This becomes the lookup/eviction brain for S3's state store and later the
  disk-mirror index (replacing the file scan) — the C++ side keeps owning
  KV bytes, the crate owns policy+index.

### Deferred (need GPU windows, next sessions)
- P2 wiring: vitriol-cache into `server_prompt_cache` + disk hash index.
- P3 jump-forward grammar-forced drafting (spec-verify path).
- P4 priority/preemption + shape-bucketed CUDA graphs.
- P5 spikes: paged KV COW, CacheBlend, EP (likely rejected: PCIe 2.0 x8).

## Execution rules
- Commit per stage (AGENTS workflow). No engine restarts, no `killall`,
  no port 8279 traffic, no GPU allocation of any kind this session.
- CPU-only verification: inner rebuild compile check where cheap; the Rust
  crate gets `cargo build` + `cargo test` (pure logic, no CUDA).
- Every benchmark/perf claim stays forbidden until the live A/B gate runs.

---

## Results log

### S0 DONE (2026-10-23 12:30)
- Inner `d6f9eb0fe`: committed the uncommitted VITRIOL_REMAP gate (outer
  `8b0042e` had already claimed it; inner commit was missing).
- Branches `stack-pull` created outer + inner.

### S1 DONE (2026-10-23 13:05) - inner `102b80db2`
Observability floor, all compile-verified (CPU build to 100%:
llama + llama-server-impl; CUDA build of ggml-cuda.cu object OK):
- `ttft_seconds` / `itl_seconds` / `queue_time_seconds` histograms
  (10/25/50/100/250/500/1000/2000 ms buckets, prometheus `_sum`/`_count`)
- `cache_reuse_lcp_bucket` reuse-depth series + `cache_reuse_lcp_zero_total`
- `cache_hit_ratio` gauge, `sampling_time_seconds_total` / `sampling_calls_total`
- CUDA-graph counters via `ggml_cuda_graph_stats()` (weak-symbol resolved;
  zero with CPU-only/backend-DL builds) -> `graph_{captures,reuse,updates,evicts}_total`
- `server_task.t_post` set at first post, survives deferral
- ITL semantics: one sample per decode step; per MTP verify round when
  drafting (tokens arrive together)
- test: `test_metrics_latency_and_cache_observability` (structural, needs
  server run - first GPU window)
NOTE: CPU-only llama-server link fails in this fork independent of these
changes (libllama references CUDA backend symbols unconditionally); all
server TUs compiled clean.

### S2 DONE (2026-10-23 13:10) - inner `31bfe305d`
Grammar parse cache: process-wide LRU (16 entries) keyed
(vocab ptr, root, text); hit path skips parse + validation +
left-recursion detection; per-request stacks/triggers still fresh.
libllama build green.

### S3 DONE (2026-10-23 13:15) - inner `30b128504`
Prompt-cache touch-LRU (was FIFO) in update() + alloc() make-room;
entries carry n_hits/t_last_hit, shown in /cache; VITRIOL_CACHE_KEEP=1
opt-in keeps blob+tokens resident on restore (identical resend = pure
hit); default stays consume-on-load for OOM discipline. alloc()
containment skip prevents duplicate kept entries.

### S4 DONE (2026-10-23 13:20) - outer `bad8a4e`
cache_reuse was ALREADY fully wired (parse -> CACHE_REUSE ->
--cache-reuse -> creuse= fingerprint; exploration snapshot stale).
Added: bless-path `creuse=` parity in config_fingerprint +
VITRIOL_LLGUIDANCE=1 build opt-in (cargo-gated, default off).

### S5 DONE (2026-10-23 13:30) - outer `3c3d668`
vitriol-cache crate: radix trie (deepest-ancestor match, edge split,
pin+recency eviction) + Megiddo-Modha ARC + panic-free C ABI
(vc_* / vc_arc_*). cargo test 17/17 green (CPU-only). NOT wired into
the server yet by design.

## Deferred to first GPU window (in order)
1. Wire vitriol-cache into server_prompt_cache (index+eviction; ARC for
   disk mirror) behind an env gate; identical-resend + branch-fork +
   restart-warm live gates.
2. Run new metrics unit test + read a real agent-loop trace off the
   TTFT/ITL/LCP/grammar-time series.
3. VITRIOL_CACHE_KEEP=1 A/B vs consume-on-load (host RAM headroom).
4. Then P3 jump-forward, P4 priority+graphs per plan above.
