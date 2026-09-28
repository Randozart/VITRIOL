# Slot Scaling Investigation — measured — 2026-09-28

**Question (owner)**: "Two slots running in parallel are slower than the sum
of their parts — cache locality or something else? No guessing."

**Answer (measured, dual RTX 3060, ts 24,24, ub 128, 38K contexts)**: two
slots deliver **~1.0-1.17× the throughput of one slot** in every phase.
Slots are a latency-multiplexing mechanism on this engine, not a throughput
multiplier. The cause is **load placement, not cache locality**: decode
compute concentrates on GPU0 (SM 49-100%) while GPU1 idles (0-4%) because
the layer-count split ignores where the context-scaling work lives.

## Experiment matrix (2026-09-28 07:54-08:44, engine cleared, production config)

### E1 — concurrent decode (MTP on, 38K contexts, 256 tok each)

| arm | slot0 t/s | slot1 t/s | aggregate | ratio vs solo |
|---|---|---|---|---|
| solo | 22.63 | 23.62 | — | 1.00 |
| concurrent ×3 | 16.66/10.58/10.23 | 5.44/15.8/15.2 | 22.1 / 26.4 / 25.4 | **0.98 / 1.17 / 1.12** |

### E1b — concurrent decode (MTP OFF, spec=none, config reverted after)

| arm | solo t/s | concurrent agg | ratio |
|---|---|---|---|
| 2 reps | 14.98 / 15.15 | 16.19 / 16.05 | **1.08 / 1.07** |

MTP is NOT the serializer — scaling stays ~1.0× without it.

### E2 — concurrent prefill (20K each, fresh content, 3 reps)

Aggregate 365.1 / 390.0 / 374.0 t/s vs solo 367.7/388.0 → **ratio ≈ 1.0×**.
Each slot gets ~half. Solo prefill already saturates the pipeline.

### E3 — mixed (20K prefill slot0 + 256-decode slot1)

- prefill: 181-185 t/s (≈ half of solo) — pays for sharing
- **decode: 20.2-20.4 t/s vs 22.6-23.6 solo (−11%)** — nearly immune.
  The fairness chunk cap works: interactive decode rides along while the
  sibling mega-prefill eats the loss.

### GPU utilization (nvidia-smi dmon, 1 Hz) — the smoking gun

During concurrent-decode windows: **GPU0 SM 49→100% while GPU1 SM 0-4%**
(persistent across consecutive samples). During prefill phases the GPUs
alternate (GPU1 97% while GPU0 0-23%). The `ts 24,24` layer split divides
*layer count* evenly, but attention work scales with *context* — at 38K
tokens the context-scaling compute lands on one GPU and the second sequence
adds work to the already-saturated one. GPU1's FFN phases are comparatively
instant. Aggregate = solo.

Also observed: E1 requests re-prefilled their full 30K prompts
(`prompt_n ≈ 30K` despite `cache_prompt=true`) — sparse-KV eviction had
punched holes in the cached prefixes during the E0 decode bursts. Known
interaction (sparse eviction vs prompt-cache prefix), separate issue.

## Hypothesis verdicts

| # | hypothesis | verdict |
|---|---|---|
| H1 decode batches ~free | **REJECTED** (1.0-1.17×, not 1.7-2.0×) |
| H2 prefill saturates solo | **CONFIRMED** (E2 ≈ 1.0×) |
| H3 cross-slot ubatch split | MINOR (folded into H2; not the dominant term) |
| H4 MTP draft serialization | **REJECTED** (MTP-off still 1.07×) |
| H5 KV locality | UNRESOLVED (second-order; not the dominant term) |
| H6 placement imbalance / pipeline bubbles | **CONFIRMED — dominant** (dmon: GPU1 0-4% SM during decode) |

## Fix directions (next session, ranked)

1. **`--split-mode row`** (NCCL row-split: both GPUs run all layers on
   partial batches — balances context-heavy attention perfectly; GGML_CUDA_NCCL
   is already ON in the build). Re-run E1/E2 under row-split: if decode
   aggregate reaches 1.5×+, this is the config fix. Watch: PCIe allreduce
   traffic on the narrow bus.
2. Attention-aware layer split (weighted ts) if row-split regresses.
3. Accept + document: slots for prefill overlap/interactivity (E3 shows
   decode is shielded), single-lane for decode throughput.

## Raw data

/tmp/opencode/slotmatrix/{results.jsonl,summary.json,dmon.log,run.log},
/tmp/opencode/mtpoff.jsonl. Config restored: `spec=mtp:1` verified via
fingerprint after the MTP-off arm.

## Addendum 1 (2026-09-28): fix arms attempted — both blocked

### Row-split (`--split-mode row`)
- Launcher gained a `gpu.split_mode` config key (parse, TS_ARGS `-sm row`,
  fingerprint `sm=row`) — wired and committed to the launcher.
- **BLOCKED**: model load fails with `device CUDA0 does not support split
  buffers` — the fork's buffer types (vitriol-dma/RAM Shot + the quant set)
  lack ggml split-buffer support. Unit crash-looped under Restart=always;
  reverted. Fixing it = ggml CUDA backend work (implement split buffers for
  the custom buffer types) — upstream-level task, not a config fix.

### Weighted layer split (`ts 22,26`)
- Rationale: decode load sat on GPU0 (attention + draft + sampling); give
  GPU1 more layers. **Does not fit**: 26/48 of weights + 26/48 of the
  262K KV + compute buffers exceed GPU1's 12 GB (unit crash-looped,
  restart counter 24). GPU1 is already at capacity at ts 24,24 — the
  rebalance direction is VRAM-infeasible.

### Standing conclusion
Two slots = one slot's throughput on this placement, and the two cheap
placement fixes are blocked (buffers / VRAM). Slots remain a latency
multiplexer (E3: interactive decode -11% under sibling mega-prefill), not
a throughput multiplier. If throughput concurrency ever matters here, the
path is split-buffer support in the CUDA backend (enables row-split +
NCCL), not config.

### Launcher artifact
`gpu.split_mode` config key shipped (default `layer`, fingerprint `sm=row`
when set) — the knob exists for the day split-buffer support lands.
