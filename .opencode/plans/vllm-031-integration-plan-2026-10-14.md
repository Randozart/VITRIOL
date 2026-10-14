# vLLM 0.31 survey + integration plan — 2026-10-14 00:00

Source: vLLM 0.31.0 installed tree,
`~/.cache/uv/archive-v0/9InspFUO35OuRAn_/vllm` (commit `gdb9527a46`,
`License-Expression: Apache-2.0`). Incoming Apache-2.0 is LICENSE-OK per
AGENTS licensing table — any verbatim copy keeps SPDX header + gains a
`PROVENANCE:` header naming vllm (Apache-2.0), what was learned/copied.
Triton kernels get re-derived as CUDA (mechanical), never linked.

Worktree: `VITRIOL-vllm` (outer branch `vllm-031`, inner branch `vllm-031`
off `main` in both repos). All stages below execute there.

## Stage 0 — survey (done)

Catalogue (file:line refs are into the vLLM tree):

### Matched to open problems

| # | vLLM mechanism | ref | VITRIOL target |
|---|---|---|---|
| a | Per-position spec-decode acceptance histogram | `v1/spec_decode/metrics.py:18-50` | Tier-1 #2 (acceptance 2/9, hand-measured) |
| b | Residual-mass recovered-token rejection sampling (exact stochastic verify) | `v1/sample/rejection_sampler.py:678,829,1004` | verify-correctness — llama.cpp truncation biases target dist |
| c | Dynamic draft-K by batch size + survival-budget adaptive verification | `config/speculative.py:492-495`, `adaptive_verification.py:282-350` | Tier-1 #2 — n=3 drafts wasted |
| d | `_reserve_prefill_lookahead` (never end prefill chunk within K of end) | `v1/core/sched/scheduler.py:541-560` | MTP × chunked prefill (VITRIOL analog: nextn `process()` hook must cover EVERY prefill ubatch incl. final `{4, 4+n_ubatch}` tail chunks — `common/speculative.cpp:1494-1509` warns, degrades) |
| e | Multi-module MTP head rotation (step i → trained head i) | `v1/worker/gpu/spec_decode/multi_module_mtp/speculator.py` | Tier-1 #2 — conditional on checkpoint having >1 nextn layer |
| f | Chained content-hash prefix cache + partial blocks + LRU free queue | `v1/core/block_pool.py:35-904`, `kv_cache_utils.py:58-99,246-370` | Tier-2 #5/#6/#9 — disk mirror evicts active conv; `id_slot` bypass; content addressing fixes both |
| g | O_DIRECT pipelined disk backend (aligned staging, sep store/load threads) | `v1/simple_kv_offload/disk_backend.py:49-110` | Tier-2/3 #14 — Optane tier head-of-line |
| h | ARC eviction policy (~150 L) | `v1/kv_offload/cpu/policies/arc.py:16-212` | `cache_ram` tier beats LRU |
| i | TQ continuation-prefill: small chunks use decode split-KV kernel against compressed cache instead of dequant-everything | `v1/attention/backends/turboquant_attn.py:79-84` (`_CONTINUATION_DECODE_THRESHOLD=128`) | 200k ctx chunked prefill with tq3_0 — O(prefix/chunk) dequant tax |
| j | Uniform-decode CUDA-graph pad: verify batches always `1+K` rows | `v1/core/sched/scheduler.py:1086-1108`, dispatcher `uniform_decode_query_len=1+K` | Tier-5 #18 — VITRIOL graph identity = exact `ubatch.n_tokens` (`llama-graph.h:812-839`), MTP verify len varies per step → rebuild/warmup thrash |
| k | Per-layer / boundary-layer KV-quant skip list | `config/cache.py:167-169`, `turboquant/config.py:177-213` | verify tq3_0 skip on hot boundary layers of hybrid |
| l | Startup-plan fingerprint cache | `v1/worker/startup_plan.py:3-17` | fingerprint doctrine applied to memory solve |
| m | Split-row top-p pipeline (batch≈1 latency) | `topk_topp_triton.py:991-1610` | sampler latency |
| n | Captured-graph operational checklist: large-shape-first capture, warmup runs, workspace lock, pad with NULL sentinels | `gpu_model_runner.py:6755-6859`, `turboquant_attn.py:409-530` | existing CUDA-graph work |

### Ideas only (rewrite, not copy)

- HiSparse host-resident KV + GPU hot staging (`v1/hisparse/runtime.py`) — architecture twin of VRAM→RAM→Optane tiers.
- Two-level block→token scoring gate (`mla/sparse_indexer.py`) — cheapen lull probe scoring.
- Async scheduling: sampled tokens stay on GPU, side-stream D2H (`gpu_model_runner.py:759-769,3741-3752`) — kill per-token host sync.
- Multi-tier cascade state machine w/ promotion pinning (`kv_offload/tiering/manager.py`).

### Negative findings (do NOT port / vLLM is behind)

- No H2O/pyramid/heavy-hitter eviction, no runtime KV resize (VITRIOL governor ahead), no AMC.
- No tree drafting — vLLM regressed vs llama.cpp tree; keep tree.
- Chain-only drafting, TP local-argmax, ray/cluster glue — irrelevant single-user.

## Stage 1 — quick wins (metrics + MTP prefill guard)

Land in `tools/server` + `common/speculative.cpp`.

1. **Acceptance telemetry**: per-pos data already exists
   (`server-common.h:497`, fill `server-context.cpp:4363-4369`, merge
   `:4521-4529`, labeled emission `server-task.cpp:1649-1658`). VERIFY what
   `/metrics` actually exposes today vs. vLLM's `SpecDecodingStats`
   `[draft_len × accepted]` histogram; gap-fill: emit
   `spec_decode_num_accepted_tokens_per_pos_total{position}` (if absent) +
   acceptance-rate gauge. Test: `tools/server/tests/unit/test_metrics.py`.
2. **MTP prefill-lookahead guard (vLLM `d` analog)**: audit that
   `common_speculative_process()` runs on every prefill ubatch incl. the
   deliberate `{4, 4+n_ubatch}` tail chunks (`server-context.cpp:3944-3958`)
   and the mtmd skip path (`:4001-4002`); either harden the
   `begin()` warn (`speculative.cpp:1494-1509`) into a chunk-boundary
   constraint or fail-closed. Confirm with a deep-prefill run: zero
   "process() hook may not have run" warnings at 44k+.
3. **Fingerprint gate (vLLM `l`)**: config/fingerprint diff check around
   engine alloc — cheapest hook consistent with AGENTS flag-provenance rule
   (warn if launch fingerprint changed KV-bearing keys since last alloc).

Acceptance: metrics unit test green; deep-prefill guard verified live;
commit.

## Stage 2 — TQ continuation-prefill kernel switch (i)

1. Study VITRIOL tq3_0 attention path (`ggml/src/ggml-cuda/`, llama-kv-cache
   TQ store/decode) — confirm whether chunked continuation prefill
   re-dequantizes the prefix each chunk.
2. If pathology confirmed: implement vLLM's threshold switch — continuation
   chunks ≤ ~128 tokens score against compressed cache via decode-style
   split-KV path; full dequant only above threshold. Reference:
   `turboquant_attn.py:79-84` + store/decode Triton (`v1/attention/ops/triton_turboquant_*.py`).
3. Also audit boundary-layer skip (k): is tq3_0 applied to first/last N
   layers of hybrid arch where it hurts? Compare vLLM policy
   (`turboquant/config.py:177-213`).

Acceptance: A/B at 131k/208k filled — prefill t/s improves or neutral; no
PPL/KL regression on the known eval harness; commit.

## Stage 3 — spec-decode correctness + adaptive K (b, c, e, m)

1. **Residual-mass recovered tokens**: port `rejection_random_sample_kernel`
   semantics (exact-distribution stochastic verification, excess-mass
   inverse-transform) into the sampling path
   (`common/sampling.cpp:678-706` area / server verify). Greedy path
   unchanged. One CUDA/host kernel + unit test.
2. **Dynamic draft-K**: batch-size→K lookup (vLLM `c`) — for VITRIOL
   single-user: runtime acceptance-driven K trim (K=3 → shrink when rolling
   acceptance < threshold), wired to the new Stage-1 histogram. Config flag
   `[spec] adaptive_k = 0/1`, rides fingerprint.
3. **Multi-module MTP (e)**: inspect Qwen3.8 checkpoint `num_nextn_predict_layers`;
   if >1, rotate draft step i → head i (`multi_module_mtp/speculator.py` spec).
   If ==1, document no-op and skip.
4. Optional: shared top-k across MTP iterations (deepseek_mtp.py:173,189 trick).

Acceptance: stochastic verify exactness test (chi-square vs target dist);
adaptive-K A/B shows acceptance/tok-per-step improvement or auto-disables;
commit.

## Stage 4 — prefix cache content-addressing + tier hardening (f, g, h)

Biggest project; directly kills Tier-2 #5/#6/#9 (disk-mirror eviction of
active conversation, `id_slot` bypass, restart re-prefill).

1. **Chained content-hash keys**: hash prompt token ranges (sha256/xxh3 of
   token IDs + extras) per block-sized unit; lookup by content, not slot
   position → identical resend / id_slot / cross-restart all hit. Model on
   `block_pool.py:35-904` + `kv_cache_utils.py:58-99`. Partial (sub-unit)
   hits optional phase.
2. **Eviction**: refcounted free queue, LRU by touch, then upgrade RAM tier
   to ARC (`policies/arc.py:16-212` ~150 L port). Fix Tier-2 #5 class bug:
   eviction can never drop blocks with live slot refs.
3. **Disk tier hardening**: O_DIRECT-aligned staging + separate store/load
   threads + pinned buffers for Optane mirror (`disk_backend.py:49-110`);
   addresses queue head-of-line + sidecar guard #14.
4. Optional later: event stream (`distributed/kv_events.py`) for Hermetis
   invalidation.

Acceptance: identical-resend + id_slot requests hit cache (re-prefill
count drops to ~0); eviction never touches active conversation (regression
test); restart warm-restore still certified; commit (likely sub-commits:
hash layer / eviction / disk).

## Stage 5 — CUDA-graph uniform verify batches + capture checklist (j, n)

1. **Uniform `1+K` verify batches**: VITRIOL graph identity is exact
   `ubatch.n_tokens` (`llama-graph.h:812-839`); MTP verify len varies per
   step (`server-context.cpp:521-558`) → rebuild/warmup thrash
   (`ggml-cuda.cu:4907-4929,2926-2942`). Pad drafting to exactly
   `draft_n_max` tokens every step (sentinel drafts that always reject —
   vLLM pads with `-1`, `scheduler.py:1086-1108`) so verify shape is fixed.
   KV-corruption risk check: padding must be masked (no KV write) — study
   before implementing; fall back to documenting infeasible.
2. **Capture checklist (n)**: apply large-shape-first capture + warmup +
   workspace lock observations to ggml-cuda capture path; add a capture-hit
   counter (Tier-5 #18 observability gap: no counter proves graphs engage).
3. Graph-key collision: confirm per-key warmup flag can't thrash across
   differing shapes (`ggml-cuda.cu:2901-2903`).

Acceptance: counter shows decode-graph hits across MTP steps; shallow-bench
decode ≥ baseline; commit.

## Execution rules

- Every stage: build + test + `PROVENANCE` headers on copied algorithms +
  own commit (AGENTS workflow). Flag-bearing configs fingerprint-diffed.
- Deep-context claims state FILLED token counts.
- Reports of results appended to this file or sibling reports in
  `.opencode/plans/` with ISO timestamps.

---

## Results log

### Stage 1 — DONE (2026-10-14 11:10)

Inner commits (worktree `VITRIOL-vllm/llama.cpp`, branch `vllm-031`):

- `589443873 server: expose spec-decode acceptance breakdown` — offered-per-pos
  denominator + `[draft_len][accepted]` verify histogram + labeled metrics
  (`spec_decode_num_offered_tokens_per_pos_total{position}`,
  `spec_decode_verify_outcomes_total{draft_len,accepted}`) + ratio gauges.
  Unit tests: `test_metrics.py` 10/10, `test_speculative.py` 9/9 (incl. new
  `test_metrics_acceptance_breakdown`).
- `f95a61881 server: fail-closed on draft ctx coverage gap` —
  `common_speculative_begin` now returns bool; coverage checks upgraded to
  fail-closed (`pos_max < N-x || pos_min > 0`) in eagle3/generic/MTP begin
  paths; slot-level `spec_draft_off` disables drafting for the request with
  `SLT_WRN`. (Item 2 of Stage 1.)
- `ab8d1b374 server: refresh cached /metrics at every task boundary` —
  BONUS FIX: the 1s time throttle on the post-update_slots refresh left the
  served `/metrics` copy stale after the last completion (queue only runs
  update_slots while tasks flow; counters stuck at boot zeros — the exact
  failure `test_metrics_prometheus_format` had on main too). Throttle removed;
  task boundary is the rate limit. Verified live: post-completion
  `n_decode_total 8`, `prompt_tokens_total 7`, `tokens_predicted_total 8`.

Stage 1 verdicts against original plan items:

- 1a (acceptance telemetry): DONE as above.
- 1b (MTP prefill-lookahead guard): DONE as fail-closed coverage guard.
  Live deep-prefill warning sweep (44k+) still pending a deep-context run on
  the worktree binary.
- 1c (fingerprint gate): NO-OP confirmed — `scripts/vitriol` already has
  `fingerprint_journal`/`_fp_delta` (per-launch diff vs last + blessed,
  dry-run), exceeding vLLM `startup_plan`.

Stage 3 revised verdicts (from source survey, not yet implemented):

- 3.1 residual-mass recovered tokens: NO-OP —
  `common_sampler_sample_and_accept_n` (`common/sampling.cpp:678-706`)
  samples y ~ p per position and accepts iff y == draft; exact by
  construction, no residual math needed. Skip.
- 3.3 multi-module MTP: verify `num_nextn_predict_layers` on the live
  Qwen3.8 checkpoint (expected == 1 -> document no-op, skip).

Stage 2 revised finding (supersedes the "TQ continuation-prefill" framing):
`ggml_cuda_fattn_kv_type_supported` (`ggml/src/ggml-cuda/fattn.cu:471-490`)
does NOT list TQ3_0 -> FA entirely disabled with tq3_0 KV; attention falls
back to the classic MUL_MAT path. Enabling FA-vec for tq3_0 is the primary
lever. Additionally, for quantized KV, prefill (Q->ne[1] > 2) selects
TILE/MMA_F16 which dequantizes the whole K/V prefix to f16 per attention call
(`fattn.cu:650-674`) — the exact vLLM O(prefix) pathology, also hitting the
live q4_0 KV config. Prereqs: `ggml_cuda_type_traits<GGML_TYPE_TQ3_0>`
(`common.cuh:1170`) lacks `bs`; `dequantize_block_tq3_0` in `convert.cu` is
file-static (needs export for fattn-vec).

### Stage 2 — DONE in two parts (2026-10-14 12:51)

Inner commits (branch `vllm-031`):

- `eafd15ca1 cuda: port TQ3_0 KV into flash-attention (fixes broken tq3
  V-cache)`. Root cause chain established by source archaeology: GPU tq3_0
  KV is RAW-domain end to end (cpy-utils `quantize_f32_tq3_0_block` no-WHT,
  convert dequant no-WHT, MMVQ/MMQ vec_dot centroid-direct, set_rows KV
  writes); the CPU ref quantizer rotates (weights only; no tq3 weights exist
  in any model, llama-quant even remaps tq3 ftypes to q4_0). fattn rejecting
  TQ3_* pushed the FA op to CPU, whose rotated-domain dequant clashed with
  GPU raw bytes -> the 2026-09-02 "TQ3 V-cache KNOWN-BROKEN" corruption.
  The `ggml_turbo_wht` op and the "pre-rotated activations" MMVQ comment are
  dead/stale relics of an older rotated design. Port = vec_dot + dequantize_V
  + gates; no WHT anywhere. Verified: set_rows-based harness
  max|FA(tq3) - CPU-ref-over-device-dequantized-bytes| <= 0.0023 (5 shapes x
  2 devices); tinygemma3 e2e tq3_0/tq3_0 output identical to f16/q4_0
  (was degraded); test-backend-ops FLASH_ATTN_EXT 5921 OK / 0 FAIL.
- `17c2e74cb cuda: small-batch quantized-KV attention skips the f16 prefix
  conversion` (Stage 2b first cut). Pathology QUANTIFIED with a
  set_rows+timing harness: quantized-KV FA converts the whole prefix to f16
  per call - 8k: 1.35 ms vs 0.15 ms (f16 KV), 40k: 6.5 vs 0.65, 82k: 12.9
  vs 1.3 (nq=4, 8 heads x D=128 per layer-tensor). Linear in prefix;
  affects q4_0 exactly like tq3_0; baked into every certified depth number
  to date. Fix: `VITRIOL_QUANT_KV_VEC_MAX_ROWS = 4` in
  `ggml_cuda_get_best_fattn_kernel` - quantized same-type KV with
  Q->ne[1] <= 4 (decode nq=1, MTP verify nq=2..4) routes to the vec kernel
  (cols_per_block=2 handles nq>1 natively; reads quantized cache directly).
  Measured nq=4: 40k 7.1 -> 3.4 ms, 82k 14.2 -> 6.5 ms. Correctness:
  harness nq=4 (VEC) + nq=8 (MMA) vs dequant ref <= 0.0027; FLASH_ATTN_EXT
  suite 0 FAIL; e2e text identical across f16/tq3_0/q4_0 KV.

Harness methodology notes (both bugs found and fixed the hard way):
`ggml_cpy` returns a NEW op node - expanding the dst leaf builds an EMPTY
graph (zeros in, zeros out, identical "errors"); f32->tq3_0 device quantize
is NOT a cpy - it is SET_ROWS (cpy.cu:609 aborts). Valid graphs build from
the returned op nodes in execution order.

Hardware note: this box now reports 2x RTX 3060 (both sm_86, 23.9 GiB) -
NOT the 3060+1070 Ti duo in AGENTS. Pascal/TILE paths are untestable here.

Open follow-ups (Stage 2 residual):

- 2c: prefill (nq >= 64) still pays the O(prefix) conversion. Candidates:
  in-kernel dequant for MMA (big), cols=4/8 VEC template (medium),
  or vLLM-style chunked-prefill threshold tuning. Crossover measured:
  VEC(2 col-blocks) beats MMA+conversion at all tested depths for nq=4;
  VEC loses beyond ~nq=8.
- 2d: VEC is still ~5x off the f16-resident ideal (3.4 vs 0.65 ms at 40k,
  nq=4). A quantized-input MMA variant or wider VEC col-blocks could close
  more of it.
- LIVE-MODEL VALIDATION PENDING: all perf/correctness so far is tiny-model +
  microbench. The 27B tq3_0/q4_0 A/B at depth needs an engine maintenance
  window (both GPUs shared with the live unit).

### Stage 3 — DONE as scoped (2026-10-14 13:40)

- 3.1 residual-mass sampling: NO-OP (see Stage-1 verdicts; sampler is exact
  by construction).
- 3.2 adaptive draft-K: DONE. Inner `86137a81d server: acceptance-driven
  adaptive draft length (--spec-adaptive-k)` - effective K = longest prefix
  of draft positions with empirical acceptance >= 0.125, gate after 32
  offers per position, full-K re-exploration every 128 drafting rounds.
  Greedy output invariant by construction; `test_adaptive_k` proves it
  e2e + metrics consistency under the trim (test suite 10/10, metrics
  10/10). Launcher wiring: outer `8a6f2a5` - `[spec] adaptive_k` config
  key -> `--spec-adaptive-k`, conditional fingerprint field `specak=1`
  (no fingerprint diff when off; live config untouched, default 0).
- 3.3 multi-module MTP: NO-OP confirmed - GGUF metadata of the live
  BuffedMod checkpoint reads `qwen35.nextn_predict_layers = 1` (single
  MTP head; nothing to rotate).

Session wrap: dev1 blocked by an active auto-antislop training run
(pid 746029, 6.9 GiB, started 12:47) - the 27B live A/B waits for it to
finish per owner decision. Stages 4 (content-addressed prefix cache) and
5 (graph-shape capture cache) remain.
