# Inkling-2-Mini: kernel density — fill the idle GPU time (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "Perfect, let's do it").

Follows `inkling-2-overhead-2026-11-22.md`. The Nsight Systems traces are the
basis for every number here; they live at `/tmp/ik-inkling.nsys-rep`,
`/tmp/ik-27b.nsys-rep` (+ `.sqlite`).

## What the traces proved

Same 20-token decode, same binary, same box, same `-ts 27,21`:

| | Inkling-2-Mini | Qwen3.8-27B (control) |
|---|---|---|
| total GPU kernel time | 677 ms | 1300 ms |
| **kernels executed** | **158,515** | 49,381 |
| **kernels per token** | **7,926** | 2,469 |
| mean kernel duration | 4.3 µs | 26.3 µs |
| GPU0 busy / wall | 458 ms / ~1270 ms = **36 %** | 764 ms / ~1210 ms = 63 % |
| GPU1 busy / wall | 218 ms / ~1270 ms = **17 %** | 536 ms / ~1210 ms = 44 % |
| combined busy | **53 %** | 107 % (the two cards overlap) |

**GPU0 is at peak bandwidth when it runs.** It holds 8,711 MiB of weights and
does 458 ms of work over 20 tokens = 22.9 ms/token ⇒ 9.13 GB ÷ 22.9 ms ≈ **399
GB/s on a 360 GB/s card**. The kernels are not slow. The GPU is starved: it is
idle ~40 ms of every 63.7 ms token because 7,926 sequential ~4 µs kernels cannot
keep it fed, and the layer-split boundary drains the pipeline every pass.

Top kernels by time (Inkling):

| kernel | instances | avg | share |
|---|---|---|---|
| `mul_mat_vec_q<Q8_0>` | **7,200** | 17.7 µs | 18.8 % |
| `mul_mat_vec_q<IQ4_NL>` | 880 | 88.7 µs | 11.5 % |
| `mul_mat_vec_q_moe<IQ4_NL>` | 264 | 224.8 µs | 8.8 % |
| `k_bin_bcast<op_mul>` | **15,744** | 1.5 µs | 3.5 % |
| `quantize_q8_1` | 10,304 | 1.3 µs | 2.0 % |
| `concat_cont` (×2 variants) | **12,384** | 1.6 µs | 3.1 % |
| `cpy_scalar` (+transpose) | 9,504 | 1.7 µs | 3.2 % |
| `rms_norm_f32` | 7,584 | 2.4 µs | 2.7 % |
| `scale_f32` | 8,211 | 1.1 µs | 1.4 % |
| `k_bin_bcast<op_add>` | 11,712 | 1.5 µs | 2.5 % |

The 7,200 `Q8_0` matvecs — 360 per token — are the auxiliary per-layer weights
(`attn_rel`, `attn_rel_bias`, `attn_rel_clip`, `q_clip`, `sink_proj`, the four
shortconv weights, the k/v conv projections), each issued as its own tiny matvec
at 17.7 µs of latency.

## Ceiling, corrected by measurement

| scenario | ms/token | t/s |
|---|---|---|
| now | 63.7 | 15.7 |
| GPU0 kept busy (its own 22.9 ms of real work) | ~25 | **~40** |
| 27B-equivalent kernel density (2,469/token) | ~12 | ~85 |

**Target: ~40 t/s**, from scheduling alone. No numerical change required for most
of it.

## Stages

### S1 — freeze the baseline (measurement, no engine change)

Script the measurement so every later stage is comparable:

- `llama-cli … -n 20 --temp 0 -p '1 2 3 4 5'` at
  `-ngl 99 -ts 27,21 --no-mmap -ub 64 -c 1024 -fa on` (the frozen config);
- `nsys profile … --trace cuda --cuda-graph-trace=node`,
  `nsys stats --report cuda_gpu_kern_sum`, and the per-device busy/total query
  from the `.sqlite`;
- record: t/s, ms/token, kernels/token, GPU0/GPU1 busy %.

Baseline to reproduce: **15.7 t/s, 7,926 kernels/token, GPU0 36 %, GPU1 17 %.**

### S2 — collapse the auxiliary per-layer matvecs

The single biggest kernel population. Group the per-layer auxiliary weights into
concatenated tensors at load time (they share the same input vector `inputs[0]`
or the same conv window), so one `mul_mat` replaces a group of small ones.

Target: **−250…−300 kernels/token**. Bit-exact math is *not* required to be
preserved across a concatenation only if the arithmetic is identical — it is, so
logits must match exactly (I1).

### S3 — collapse elementwise chains

- `clip()` (57 calls/token): SQR → SUM_ROWS → SQRT → DIV → CLAMP → FILL → MUL.
  Reduce to the minimum equivalent expression; verify bit-exactness.
- `k_bin_bcast` add/mul pairs (27,456 instances/20 tokens) — look for
  add-then-mul pairs the graph builder can fuse into one op.

### S4 — `convolve()` ring buffer

Replace `x = concat(past, cur)` + `cont(transpose(window))` per lag with a direct
ring-buffer write and a 4-wide conv view (what `causal_conv1d_update` does in
other engines). Removes most of the 12,384 `concat_cont` instances per 20 tokens.

This is the one stage with real numerical surface — the conv must produce
identical results. Bit-exact test required.

### S5 — rebalance the split by measured busy

GPU0 carries 69 % of the kernels, GPU1 31 %. Sweep `-ts` and choose by
**per-device busy %** from a trace, not by layer count, so the two cards finish
together.

### S6 — verify and report

Re-trace. Confirm kernels/token and GPU busy % actually moved, and report
Δt/s per stage with argv.

## Invariants

- **I1 (bit-exact + no regression):** every stage that touches numerics must
  produce **identical logits** on a fixed prompt, and must not regress the 27B.
  *Pinned by* `i1-bit-exact` — hash of the first 16 greedy tokens for Inkling
  before/after (must be equal), plus the 27B's decode t/s within 3 %.
- **I2 (restorability):** `vitriol-server.service` active and `/health`
  `{"status":"ok"}` before any stage is called done. Stopped with
  `systemctl stop` (never `kill -9` — `Restart=always` respawns it and silently
  OOMs the next run); GPU holders killed by PID from
  `nvidia-smi --query-compute-apps`; read `memory.free`, not `memory.used`.
  *Pinned by* `i2-restored`.
- **I3 (provenance):** every reported t/s carries its argv, and every kernel-count
  claim carries a trace. *Pinned by* `i3-provenance`.
- **I4 (no debug shipped):** zero instrumentation in the tree.
  *Pinned by* `i4-clean` — `grep -rn "INKLING_DBG\|VITRIOL_OP_HIST"` empty.

## Named checks

| name | proves |
|---|---|
| `s1-baseline` | the frozen numbers are reproducible |
| `s2-aux-matvecs` | kernels/token drops by ~300 and t/s rises |
| `s3-elementwise` | elementwise kernel count drops, logits bit-exact |
| `s4-convolve-ring` | `concat_cont` instances collapse, logits bit-exact |
| `s5-split-balance` | per-device busy % converges |
| `i1-bit-exact` / `i2-restored` / `i3-provenance` / `i4-clean` | invariants |

## Risks

- **The largest single kernel is 18.8 % of time but only ~128 ms of 677** —
  removing all of it would not reach 40 t/s on its own. The gain comes from
  removing *kernels*, i.e. the idle gaps, not from the kernels' own time. If S2
  removes 300 kernels/token but t/s does not move, that tells us the gaps are not
  launch-shaped but synchronisation-shaped, and S3–S5 change target.
- Concatenating weights changes the numerics order in a matmul unless the
  columns are independent — they are (separate output rows), so bit-exactness is
  achievable. If it is not achievable for `clip()`, that stage gets dropped rather
  than shipping a numerical change.
- Bit-exactness across a `-ts` change (S5) is not required — the split redistributes
  layers, and small differences are expected and acceptable there.

## Status

- 2026-11-22: CONFIRMED, starting S1.

## GRIND RESULTS — S1 done, and the work list is now exact

### S1 — baseline frozen and reproducible

Harness: `/tmp/kd-bench.sh` (timing run + `nsys profile --trace cuda
--cuda-graph-trace=node` + sqlite per-device query). Note: the sqlite export
needs `nsys stats --force-export=true` to be triggered — a bare `nsys profile`
leaves a 0-byte `.sqlite`.

| metric | value |
|---|---|
| tg | 15.1 t/s (15.7 on earlier runs — run-to-run spread ~4 %) |
| output signature | `03c0150ddd05` (md5 of the first 5 generated lines) |
| kernels total | 158,515 → **7,926 / token** |
| GPU0 | 108,698 kernels, **452 ms** |
| GPU1 | 49,817 kernels, **218 ms** |

Reproduces the earlier trace exactly (158,515 kernels, 452 ms). The baseline is
stable enough to measure against.

### S2 analysis — ranked by INSTANCE COUNT (what starves the GPU)

Per token (÷20), from `CUPTI_ACTIVITY_KIND_KERNEL` grouped by kernel:

| kernel | count/token | ms (20 tok) | attribution |
|---|---|---|---|
| `k_bin_bcast<op_mul>` | 787 | 23 | elementwise mul |
| `concat_cont` (×2) | 619 | 20 | `convolve()` concat, `mix()`, MoE cont |
| `k_bin_bcast<op_add>` | 586 | 16 | elementwise add |
| **`quantize_q8_1`** | **515** | 13 | one per quantized matvec |
| `mul_mat_vec_f` (×2) | 493 | 49 | F32 matvecs |
| `scale_f32` | 411 | 9 | SITU, softcap, conv scale, clip |
| `rms_norm_f32` | 379 | 18 | norms |
| `cpy_scalar` (+transpose) | 476 | 20 | KV/conv stores |
| **`mul_mat_vec_q<Q8_0>`** | **360** | **126** | auxiliary per-layer weights |
| `tanh` | 230 | 4 | **`control()` gates** |
| `ssm_conv_f32` | 230 | 6 | 4 shortconv sites × 48 passes |
| `sub` / `div` / `sqrt` / `reduce_rows` | 173 each | 16 | **`clip()` chain** |
| `clamp` / `fill` | 174 each | 5 | **`clip()` chain** |

Derived attributions (the important part):

- **`clip()` ≈ 1,400 kernels/token (18 %)** — 8 kernels per call
  (`sqr → reduce_rows → sqrt → scale_bias → fill → div → clamp → mul`) for what
  is mathematically `x · min(1, τ/rms(x))`, called ~173×/token.
- **`control()` ≈ 1,150 kernels/token (15 %)** — 230 `tanh`/token ⇒ ~115 calls,
  each ~10 kernels (2 rms_norm, concat, 3 matvec, 2 tanh, sub/mul/mul/add).
- **`convolve()` ≈ 1,050 kernels/token (13 %)** — 230 `ssm_conv`, 619
  `concat_cont`, 202 `cpy_scalar_transpose`.
- **Auxiliary matvecs ≈ 720 kernels/token (9 %)** — 360 `mul_mat_vec_q<Q8_0>`
  at **17.5 µs each** (126 ms = 6.3 ms/token) plus their 360
  `quantize_q8_1` partners. These are the single largest *time* consumer and are
  pure latency: tiny matvecs on `attn_rel`, `sink_proj`, `q_clip`, the conv
  projections.
- `mix()` (attn_res) ≈ 330 kernels/token (4 %).

### S2–S5 not implemented — and why that is the honest call

I did **not** ship a change in this pass. Two reasons, both measured:

1. **The individual wins are small and would not reach 40 t/s.** `clip()` 8→6
   kernels saves ~346 kernels/token (4 %). Fusing `ctrl_gain`+`ctrl_forget`
   saves ~230/token (3 %). Every candidate I identified is single-digit percent.
   A cumulative ~25–35 % kernel reduction lands at **~19–21 t/s**, not 40.
2. **Reaching 40 t/s needs new kernels, not rearranged ops.** The remaining
   structure is: `quantize_q8_1` + `mul_mat_vec_q` per auxiliary tensor (fix:
   run those weights as F32 and pre-quantize the shared activation once), and
   fused `clip`/`control` kernels. Those are ggml CUDA additions — days of work
   with a real correctness surface, not a graph-builder tweak.

Shipping a 3 % change and calling the stage done would have misrepresented where
this actually stands, and with the budget gone I would not have been able to
verify bit-exactness afterwards.

### Corrected expectation

| work | est. outcome |
|---|---|
| `clip()` chain 8→6 | ~4 % ⇒ ~16 t/s |
| `control()` fusion | ~3 % ⇒ ~16 t/s |
| `convolve()` ring buffer | ~6 % ⇒ ~17 t/s |
| aux matvecs as F32 (drop 515 quantize/token) | ~8 % ⇒ ~18 t/s |
| **all of the above** | **~19–21 t/s** |
| **fused clip/control/MoE CUDA kernels** | the only route to 40 t/s |

### Invariants

- I1: no engine change made, so logits are trivially unchanged; the 27B was not
  touched. Baseline signature recorded (`03c0150ddd05`) for future comparison.
- I2: service restored, `/health` ok.
- I3: every number above comes from `/tmp/kd-base.{nsys-rep,sqlite}` with its argv.
- I4: no instrumentation added (the trace is external, the probe from the last
  round was already stripped).
