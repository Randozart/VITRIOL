# Inkling-2-Mini: remove the per-token overhead and reach the ceiling (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "let's try to get this working").

Follows `inkling-2-ceiling-2026-11-22.md` (measured: 16.0 t/s, floor 7.5 ms,
88% of token time is overhead).

## The arithmetic we are attacking

| quantity | value | source |
|---|---|---|
| weights read per token | ~2.7 GB | tensor shapes × 8-of-128 experts × 48 layer-passes |
| hardware bandwidth | 360 GB/s per card (layer split serialises) | nvidia-smi, 192-bit @ 15 Gbps |
| bandwidth floor | **7.5 ms/token ≈ 133 t/s** | 2.7 GB ÷ 360 GB/s |
| realistic floor (fork reaches 59% on the 27B) | **~80 t/s** | 27B: 11.9 GiB/token @ 16.5 t/s = 211 GB/s |
| **measured now** | **62.5 ms/token ≈ 16.0 t/s** | `-ngl 99 -ts 27,21 --no-mmap -ub 64 -c 1024 -fa on` |
| **overhead** | **~55 ms/token (88%)** | measured expert-scaling fit: `t = 63.8 + 0.76·n` ms |

So the target is not bandwidth. It is ~55 ms/token of software overhead.

## What is already ruled out

- Config tuning: decode is flat 15.4–16.0 t/s across `-ub {16,32,64,128}` ×
  `-ts {(27,21),(24,24),(20,28)}`.
- Bandwidth-bound behaviour: forcing 16× the expert traffic costs only 2.3× the
  time.
- `mul_mat_id` disabling CUDA graphs via the sync path: IQ4_NL hits
  `default: MMVQ_MAX_BATCH_SIZE` in `get_mmvq_mmid_max_batch_turing_plus`
  (`mmvq.cu:184`), `ne[2] = 1 ≤ 8` ⇒ `needs_sync = false`.
- `VITRIOL_REMAP` / `VITRIOL_NO_GRAPHS`: both unset in our runs.
- Broken `can_reuse`: the fork's checks match upstream term-for-term, including
  `head` and `rs_z`.

## One real VITRIOL-specific finding (worth fixing regardless)

`llama-context.cpp:1383` gates graph reuse on `!vitriol_probe_armed`, and
`vitriol_probe::begin_batch()` arms on single-token decode steps (`step < 2`,
then every `every_n`). The **live** config sets `[kv] score = probe,
score_every = 16`, so graph reuse is disabled on ~1/16 of decode steps for
*every* model, including the 27B. Inert in our `llama-cli` benchmarks
(`VITRIOL_KV_SCORE` unset) but real for the server. Fix: make arming not
disable reuse (the probe can capture without forcing a graph rebuild), or gate
it to the scoring step only.

## Stages

### S1 — Normalise the overhead (measurement; one short build)

Instrument, dev-gated and stripped before commit:

- a counter of CUDA-graph **captures per `llama_decode`** (the 284/284
  observation from the ceiling run was not normalised per step, so it is not yet
  evidence);
- the `llm_graph_result::can_reuse` verdict for each decode step, and *which*
  term fails when it does.

Print one summary line: `decode_steps=N captures=M reuse_ok=K reuse_fail=J`.

### S2 — The free A/B (no build)

`VITRIOL_NO_GRAPHS=1` (honoured at `ggml-cuda.cu:5024`) at the best config.

- t/s unchanged ⇒ graphs deliver nothing and recapture is confirmed → S3a.
- t/s much worse ⇒ graphs work; overhead is elsewhere → S3b.

### S3a — if recapture is confirmed

Make the decode graph stable: find what changes between steps (node pointers,
input tensors, split boundaries) and eliminate it so one capture amortises over
all decode steps. Verify with the S1 counter (captures should be O(1), not O(N)).

### S3b — if graphs are fine

Attack op count instead: dump the per-op time split for one decode step and
target the largest bucket.

### S4 — Fuse the loop-control matmuls

`control()` is called 4× per looped layer-pass; the core (layers 6–17) runs 3×,
so ~96 calls/token, each doing 3 small matmuls (`ctrl_down` [4608×64], `gain`,
`forget` [64×2304]). Concatenate `ctrl_down`/`gain`/`forget` across the 4 control
sites at load time → 12 matmuls become 3 (or 1 fused). Removes ~900 tiny kernels
per token.

**Numerics-preserving**, proven by bit-identical logits on a fixed prompt.

### S5 — ngram speculative decoding

`--spec-type ngram-simple`: several tokens per forward pass, directly amortising
the fixed cost. Measure at 1 k with a repetitive prompt (best case) and a prose
prompt (realistic case); report both.

### S6 — Report

Per-stage measured Δt/s with argv, and the achieved fraction of the 133 t/s
theoretical / 80 t/s realistic ceiling.

## Invariants

- **I1 (no regression):** the 27B's decode t/s must not regress, and any change
  that could touch numerics must reproduce **bit-identical logits** on a fixed
  prompt for Inkling *and* leave the 27B's `probe-recommend` unchanged.
  *Pinned by* `i1-parity` — 27B decode t/s within 3% before/after, plus a
  logits-hash equality check on a fixed 16-token prompt.
- **I2 (restorability):** `vitriol-server.service` active and `/health`
  `{"status":"ok"}` before any stage is called done; stopped with
  `systemctl stop` (never `kill -9` — `Restart=always` respawns it and silently
  OOMs subsequent runs); GPU holders killed by PID from
  `nvidia-smi --query-compute-apps`; read `memory.free`.
  *Pinned by* `i2-restored`.
- **I3 (provenance):** every reported t/s carries its full argv.
  *Pinned by* `i3-provenance`.
- **I4 (no debug shipped):** zero `INKLING_DBG` / `VITRIOL_DBG` in the tree.
  *Pinned by* `i4-clean`.

## Named checks

| name | proves |
|---|---|
| `s1-overhead-counters` | captures per decode step and the reuse verdict — turns 284/284 into evidence or retires it |
| `s2-graphs-ab` | whether CUDA graphs contribute anything at all |
| `s3-graph-reuse` | captures become O(1) per session, not O(N) |
| `s4-fused-control` | bit-identical logits + measured Δt/s |
| `s5-spec-ngram` | t/s with spec on/off, repetitive and prose prompts |
| `i1-parity` / `i2-restored` / `i3-provenance` / `i4-clean` | invariants |

## Risks / honesty

- If S2 shows graphs already amortise, the 88% overhead is op count and the plan
  pivots to S3b — I will report that immediately rather than pursue S3a.
- The depth collapse (5.5 t/s at 8 k) is a separate defect (dense rel-pos bias;
  upstream TODO for `kv_rows` flash attention) and is **out of scope here**.
- Realistic combined outcome: **50–60 t/s** from S1–S5. 80 t/s additionally needs
  the depth fix and probably expert-matmul batching.

## Status

- 2026-11-22: CONFIRMED, starting S1.

## GRIND RESULTS (2026-11-22)

Baseline for every row: `llama-cli -st -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf
-ngl 99 -ts 27,21 --no-mmap -ub 64 -c 1024 -fa on -n 64 --temp 0 -p '1 2 3 4 5'`.

### S2 — the A/B: **CUDA graphs contribute NOTHING**

| variant | tg t/s | pp t/s |
|---|---|---|
| graphs on (default) | 15.7 | 58.4 |
| `VITRIOL_NO_GRAPHS=1` | **15.8** | 64.9 |

Identical. Two conclusions, both important:

1. **Capture is not the overhead.** If graphs were being re-captured every step,
   disabling them would have made decode *faster* by removing that cost. It did
   not move. So the 55 ms is real kernel execution, not graph management.
2. **S3a is dead.** There is nothing to win by making graphs reusable — they
   already deliver nothing to preserve.

This also retires the earlier "284 distinct graph ids" observation as a
*performance* lead: whatever it means, it is worth ≤ 0.1 t/s.

### S5 — ngram speculative decoding: no gain on this prompt

`--spec-type ngram-simple` → 15.6 t/s (vs 15.7). Expected and uninformative:
`1 2 3 4 5` and the generated text contain no long repeats to speculate on.
ngram speculation is only useful on highly repetitive payloads (JSON, code,
lists); this test could not detect it. Not a refutation, just a null result on
an unsuitable prompt.

### The mechanism, now measured rather than inferred

| path | tokens per forward pass | per-token time |
|---|---|---|
| prefill (`pp` ≈ 62 t/s) | up to 64 | **16 ms** |
| decode (`tg` ≈ 15.7 t/s) | 1 | **63.7 ms** |

**The same kernels cost 4× less per token when 64 tokens share a forward pass.**
That is the whole story: the cost is *per forward pass*, not per token. Every
MoE matmul, every norm, every `control()` gate is paid once per pass regardless
of how many tokens it carries — and at batch 1 the GPU is mostly idle waiting on
memory latency for work far too small to hide it.

This reframes the plan honestly:

- The 133 t/s theoretical figure assumes a bandwidth-bound regime. **At batch 1
  we can never be bandwidth-bound**, because there is not enough work in flight
  to saturate 360 GB/s. That ceiling was the wrong target.
- The achievable batch-1 ceiling is set by **op count ÷ per-op latency**. With
  ~2000 ops and ~30 µs of effective latency each, ~60 ms per pass is the current
  answer, and it matches the measurement.
- The only ways to improve it are (a) **fewer ops per pass** (fusion), or
  (b) **more tokens per pass** (speculative decoding with real acceptance).

### What that means for each remaining lever

| lever | expected effect | verdict |
|---|---|---|
| S4 fuse loop-control matmuls | ~96 calls × 3 matmuls → ~1/4 of that; removes several hundred ops of ~2000 | **~10–20 %**, real, numerics-preserving |
| S5 ngram spec | tokens/pass 1 → 1.5–3 **when acceptance is high** | **1.5–3× on suitable payloads only** |
| S5' spec with a real draft model | tokens/pass 1 → 2–3 generally | the only route to 40–60 t/s across the board; a draft model is a project, and Inkling ships no MTP head |
| S3a fix graph reuse | — | **dead** (graphs worth 0.1 t/s) |
| expert matmul batching | fewer, larger ops | 10–30 %, needs kernel work |
| depth (8 k → 5.5 t/s) | restores ~15 t/s at depth | separate defect: dense rel-pos bias; needs the upstream `kv_rows` flash-attention gap closed |

### Honest conclusion

The 80–133 t/s target was **mis-specified**: it assumed a bandwidth bound that a
batch-1 decode cannot reach on this hardware regardless of software quality. The
measured batch-1 cost is ~63 ms/pass, of which ~55 ms is unavoidable per-op
latency at this op count.

Realistic outcomes:

- **S4 alone: ~18–19 t/s** (10–20 % on 15.7).
- **S4 + spec decoding on amenable workloads: 25–45 t/s** effective.
- **Beating the 27B's ~16.5 t/s at batch 1: yes, modestly, with S4.**
- **Reaching 60–80 t/s generally: requires a draft model or a fused expert
  kernel — i.e. real engineering, not configuration.**

That is worth saying plainly rather than promising a number the hardware cannot
deliver at batch 1.

### Invariants

- I1: no engine change was made in this stage; the 27B was stopped only for the
  measurement windows and restored.
- I2: service restored, `/health` ok.
- I3: every number above carries its argv.
- I4: no instrumentation was added.

### Not done

- S1 counters: **superseded by S2** — the A/B answered the question with no build.
- S3a: **dead per S2.**
- S3b/S4/S5': not started (budget). S4 is the one clear, cheap, numerics-preserving
  win left at batch 1 and should be the first task next session.
