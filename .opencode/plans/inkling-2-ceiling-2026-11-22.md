# Inkling-2-Mini: find the decode ceiling (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "confirm").

Follows `inkling-2-cpu-bisect-2026-11-22.md` (numerical corruption fixed —
`9ce106a`; the model is coherent) and the DISCUSS turn that produced the ceiling
arithmetic below.

## Ceiling arithmetic (baseline to beat)

Hardware: 2× RTX 3060 12 GB, **360 GB/s each** (192-bit @ 15 Gbps; nvidia-smi
reports 7501 MHz max memory clock).

**Layer split serialises decode**: with `-ts` (LLAMA_SPLIT_MODE_LAYER) layer N+1
cannot start before layer N finishes, so the aggregate bandwidth is *not*
720 GB/s. The ceiling is one card at a time ≈ 360 GB/s.

Weight traffic per token, from the real tensor shapes:

| component | elems/token | bytes @ 4.5 bpw |
|---|---|---|
| experts (8 of 128) × 48 layer-passes | 8·(5.31M + 2.65M)·48 ≈ 3.06 G | ≈ 1.72 GB |
| attention + dense (48 passes) | ≈ 1.24 G | ≈ 0.70 GB |
| `output` matrix [2304, 201024] | 463 M | ≈ 0.26 GB |
| **total** | | **≈ 2.7 GB/token** |

| reference point | t/s | effective bandwidth | % of 360 GB/s |
|---|---|---|---|
| theoretical ceiling | ~133 | 360 GB/s | 100% |
| **realistic** (fork's own dense efficiency) | **~78–85** | ~212 GB/s | ~59% |
| **Inkling now** | **15.9** | ~43 GB/s | **~12%** |
| 27B Qwen3.8 now (11.9 GiB/token) | 16.5 | 211 GB/s | 59% |

The engine demonstrably sustains 59% of peak on dense matmuls (the 27B). Inkling
manages 12%. **The ~5× gap is the expert path, not the hardware.**

## Stage S1 — config grid at ctx 1024

Decode t/s only (prefill at short prompts is overhead-dominated and is not
reported). Grid: `-fa on`, `-c 1024`, `-n 64`, `--temp 0`:

- `-ub {16, 32, 64, 128}`
- `-ts {(27,21), (24,24), (20,28)}`

Record t/s and the resulting `% of peak`. This bounds how much of the 5× is
config rather than kernel.

## Stage S2 — restore ubatch headroom

`ggml_cont` (the correctness fix) grew the compute buffer and forced `-ub 16`.
Either
(a) make the materialisation conditional — cont only where the consuming op
needs contiguity — or
(b) keep it and shrink `-c` for the sweep.

Goal: `-ub 64` and `-ub 128` load again, so prefill is measurable and decode gets
its normal ubatch.

## Stage S3 — time the expert path

Determine whether the 96 small MMQ launches per token (`gate_up` + `down` × 48
layer-passes) dominate. Approach: compare t/s at `-ub 1` vs `-ub 8` vs `-ub 32`;
if it is launch-bound the curve is flat and the fix is structural.

If launch-bound, report the structural options rather than implementing them:
batch the two expert matmuls, cut the loop's expert re-reads, or a fused
gate_up+down kernel.

## Stage S4 — spec decoding and KV quant

- `--spec-type ngram-simple` (arch-agnostic; the 27B already benefits).
- KV `-ctk/-ctv {f16, q8_0, q4_0}` — measure at ctx 1024 (small effect) and keep
  for S5.

## Stage S5 — the decision number: depth curve

Identical prompts, both models, at **1K / 8K / 32K** filled context:
Inkling-2-Mini (best config from S1–S4) and the 27B live config. Report t/s and
`% of peak` per point.

This is the number that answers "can it beat Qwen3.8": Inkling keeps KV for only
4 full-attention layers (20 are SWA-1024), so its decode cost should be nearly
flat with depth while the 27B degrades 12.7 → 10.5 t/s between 13K and 36K.

## Stage S6 — report

Write the results into this file: table of t/s and % of peak, the dominant
bottleneck from S3, and an explicit statement of the achieved ceiling vs the
133 t/s theoretical.

## Invariants

- **I1 (parity, no regression):** any engine change is arch-gated or
  backend-generic, and the 27B's decode t/s must not regress. *Pinned by*
  `i1-parity` — 27B `llama-cli` decode t/s before/after within 3%, and
  `libvitriol probe-recommend` output identical.
- **I2 (restorability):** `vitriol-server.service` active and `/health`
  `{"status":"ok"}` before any stage is called done. The unit is
  `Restart=always`, so it is stopped with `systemctl stop`; GPU holders are
  killed by PID from `nvidia-smi --query-compute-apps`; read `memory.free`.
  *Pinned by* `i2-restored`.
- **I3 (provenance):** every reported t/s carries its full argv and the
  `VITRIOL-FINGERPRINT` line. A number without argv is not a result.
  *Pinned by* `i3-provenance` — each row in the results table has an argv.
- **I4 (no debug shipped):** zero `INKLING_DBG` / `VITRIOL_INKLING_DBG` in the
  tree. *Pinned by* `i4-clean`.

## Named checks

| name | proves |
|---|---|
| `ceiling-bandwidth` | the 2.7 GB/token figure recomputed from the loaded model's actual tensor types, not assumed 4.5 bpw |
| `s1-config-grid` | how much of the gap is configuration |
| `s3-expert-path` | whether decode is launch-bound (structural) or bandwidth-bound |
| `s5-depth-curve` | the decision number vs the 27B at 1K/8K/32K |
| `i1-parity` / `i2-restored` / `i3-provenance` / `i4-clean` | invariants |

## Risks / honesty

- I do **not** expect 133 t/s. A realistic outcome is **50–80 t/s**; if S3 shows
  a structural limit, the honest answer is "~25–30 t/s without kernel work".
- Each point costs ~2 min (load + 64 tokens). The grid is ~12 runs ≈ 30 min.
- `-ub 128` may still not fit; S2 exists to fix that and may itself fail — in
  which case report and continue with `-ub 16/32`.

## Status

- 2026-11-22: CONFIRMED, starting S1.

## GRIND RESULTS (2026-11-22)

All runs: `llama-cli -st -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf
-ngl 99 -ts 27,21 --no-mmap -fa on --temp 0 -n 64 -p '1 2 3 4 5'` plus the
variations below. Prefill is reported only for context; it is not the metric.

### S1 — config grid at ctx 1024: decode is FLAT

| ub | ts | tg t/s | pp t/s |
|---|---|---|---|
| 16 | 27,21 | 15.5 | 40.7 |
| 16 | 24,24 | 15.5 | 48.1 |
| 16 | 20,28 | 15.8 | 46.3 |
| 32 | 27,21 | 15.8 | 58.8 |
| 32 | 24,24 | 15.4 | 61.0 |
| 32 | 20,28 | 15.8 | 62.7 |
| 64 | 27,21 | **16.0** | 61.5 |
| 128 | 27,21 | **16.0** | 62.7 |

**16.0 t/s is the best, and the spread is 15.4–16.0 (4%).** Configuration is not
the lever. (Decode has `n_tokens = 1`, so ubatch is irrelevant to generation by
construction — the flat decode curve is expected and informative.)
Prefill *does* respond: 40.7 → 62.7 t/s from ub 16 → 128.

**S2 is moot**: `-ub 64` and `-ub 128` load fine (no OOM) once the 27B engine is
actually stopped. Earlier OOMs were the `Restart=always` respawn, not the buffer.

### S3 — expert-count scaling: NOT bandwidth-bound

`--override-kv inkling2_loop.expert_used_count=int:N` changes the expert traffic
4× and 16× respectively:

| experts | tg t/s | token time | Δ |
|---|---|---|---|
| 8 (native) | 14.3 | 69.9 ms | — |
| 32 | 11.5 | 87.0 ms | +17.1 ms for 4× traffic |
| 128 | 6.1 | 163.9 ms | +76.9 ms for 4× traffic |

Fit `t = a + b·n`: **b ≈ 0.76 ms/expert, a ≈ 63.8 ms.**
At n = 8 the bandwidth-dependent share is **≈ 9 % — 91 % of the token time is
fixed cost independent of expert traffic.**

If this were bandwidth-bound, 16× the traffic would cost ≈ 16× the time
(≈ 0.9 t/s). It costs 2.3×. **The binding constraint is per-token fixed cost
(op count and per-step overhead), not memory bandwidth.**

### CUDA graphs: captured, but never reused across steps

`-v` run shows `CUDA graph warmup complete` **284 times with 284 distinct graph
ids**, ids marching upward and never repeating. Graphs are therefore being
**re-captured continuously** instead of being reused per decode step — capture
cost paid every token, benefit lost.

Ruled out as the *cause* of that: `mul_mat_id` does **not** force the sync path
for us — IQ4_NL falls to `default: MMVQ_MAX_BATCH_SIZE` in
`get_mmvq_mmid_max_batch_turing_plus` (`mmvq.cu:184-193`), `dst->ne[2] = 1 ≤ 8`,
so `ggml_cuda_mul_mat_id_needs_sync` returns false and `[TAG_MUL_MAT_ID_CUDA_GRAPHS]`
does not disable graphs. `VITRIOL_REMAP` and `VITRIOL_NO_GRAPHS` are both unset.

Remaining suspects for the per-step recapture: the scheduler rebuilding the
compute graph each step (node pointers change → new `cgraph->nodes[0]` key →
new `cuda_graph` entry), i.e. `can_reuse` returning false.

### S5 — depth: **the depth thesis is REFUTED**

Prompt = 400 × `the quick brown fox jumps over the lazy dog.` (~8 k tokens),
`-c 16384 -ub 64 -fa on`:

| filled context | tg t/s | effective BW |
|---|---|---|
| ~1 k | 16.0 | ~43 GB/s (12 % of peak) |
| ~8 k | **5.5** | ~15 GB/s (4 % of peak) |

Generation drops **3×** between 1 k and 8 k — *faster* degradation than the 27B
(12.7 → 11.16 t/s from 13 k → 26 k). The hypothesis that Inkling's SWA-heavy KV
would hold speed at depth is **wrong on this engine**.

Likely reason: the relative-position bias is materialised **densely** —
`kq_b` is `[n_head, n_kv, n_tokens]` plus a dense mask add, O(n_kv) per token per
full-attention layer. Upstream carries the same TODO ("needs a kv_rows flash
attention with a per-head mask (CUDA has none)"), so this is an inherited
architectural limitation, not a port defect.

### Answer to the question

- **Ceiling (bandwidth-bound, if we got there): ~80 t/s realistic, ~133
  theoretical.** We are at 16.0, i.e. ~12 % of peak versus the 27B's 59 %.
- **But we are not bandwidth-bound**, so "tuning memory" cannot get there. The
  ceiling we are actually up against is a **software per-token overhead
  ceiling**, dominated by (a) ~48 layer-passes and (b) a CUDA graph capture per
  decode step.
- **At equal depth, Inkling does not beat the 27B today**: 16.0 vs ~16.5 t/s at
  1 k, and 5.5 vs ~11 t/s at 8 k. The 27B wins at depth.
- The honest headroom is real but requires work in the graph/overhead layer, not
  configuration: fix per-step graph reuse, and defer the depth regression to the
  upstream kv_rows flash-attention gap.

### Invariants

- I1: no engine change was made in this stage, so parity is trivially preserved;
  the 27B was stopped only for measurement windows and restored.
- I2: service restored, `/health` ok.
- I3: every number above carries its argv.
- I4: no instrumentation was added.

### Not run

- **S4** (spec decoding `ngram-simple`, KV quant sweep) — deferred. `ngram-simple`
  is the most promising remaining *cheap* lever now that config is exhausted,
  because it attacks the fixed per-token cost directly by emitting several
  tokens per forward pass.
