# Qwen3.8-27B Phase E — decode diagnostics instrumentation

Status: instrumentation implemented, build in progress, measurement pending
Date: 2026-08-19
Related: `.opencode/plans/qwen38-phase-d-bottleneck-2026-08-19.md`

## Goal

Pin down, per decode, exactly where the ~80 ms of wrapper overhead lives
(Phase D conclusion: graph build + CUDA-graph capture/replay + host syncs +
scheduling, hitting both non-MTP 11.03 and MTP 14.37 t/s baselines). Expose it
live in the TUI so config A/B tests can be read off the screen.

## Instrumentation (env-gated by `GGML_CUDA_GDN_PROFILE=1`, same toggle as the
existing `[GDN]`/`[DEC]` timers)

### llama-context.cpp — decode split (E1)
- `process_ubatch`: times **build** (graph build + alloc + `set_inputs`) vs
  **compute** (`graph_compute`, the synchronous CUDA work) per ubatch.
- `llama_decode`: times **total**, and derives **post** =
  total − build − compute. `post` includes logits/embd extraction plus the MTP
  hook — i.e. the nested `ctx_mtp` decode and its `synchronize()`.
- Reentrancy: a thread-local depth counter makes the nested `ctx_mtp` decode
  accumulate into the outer decode's `post` instead of emitting its own line.

### ggml-cuda.cu — graph mode + op census (E2)
- `ggml_cuda_graph_evaluate_and_capture`: counts **capture** (only when
  `use_cuda_graph && cuda_graph_update_required`) vs **replay** (graph launch
  with no re-capture) per decode.
- The node loop counts per-`op` node totals (capture/eval path only) for a
  top-4 op-class breakdown.
- New `GGML_BACKEND_API`: `ggml_cuda_perf_reset()`, `ggml_cuda_perf_get()`.

### MTP hook — sync census (E3)
- `handle_mtp_for_ubatch`: times each `synchronize()` (the `ctx_mtp` pipeline
  stall) and counts them.

## Output

One `[PERF]` line per outermost `llama_decode`:

```
[PERF] total=97.5ms build=12.0ms compute=80.2ms post=5.3ms graph=1C/57R sync=2(2.0ms) top_ops=FFN=64 attn=32 MUL_MAT=16
```

- `graph=1C/57R` — 1 capture, 57 replays this decode. If verify re-captures
  every decode, C ≫ R and that is the smoking gun for the MTP overhead.
- `sync=2(2.0ms)` — MTP-hook synchronize stalls.
- `top_ops=` — most numerous op classes by node count (graph composition).

### TUI surfacing
- `poller.rs`: `parse_perf()` / `parse_perf_line()` parse the newest `[PERF]`
  line; `model.rs` gains `PerfSnapshot` on `GenSnapshot`.
- `ui.rs`: the GEN card renders a breakdown line
  (`total Xms [build | compute | post]`) and a graph/sync line, colored warn
  when captures outnumber replays.
- 2 unit tests lock the parser.

## How to run

```sh
# kill stale server, then start the winner config with perf mode on
killall -9 llama-server
cd ~/Downloads
GGML_CUDA_GDN_PROFILE=1 ~/Desktop/Projects/VITRIOL/llama.cpp/build/bin/llama-server \
    -m Qwen3.8-27B-Q3_K_M.gguf -c 131072 -ts 24,12 --main-gpu 0 -ub 128 \
    --cache-type-k q4_0 --cache-type-v q4_0 --spec-type mtp --spec-draft-n-max 1 \
    --port 8090 &> ~/.vitriol/logs/vitriol_gen.log &
```

Generate, then grep `[PERF]` from the log, and watch the GEN card in the TUI.

## Interpreting

| observation | conclusion | candidate fix |
|---|---|---|
| `graph=1C/0R` repeated | verify re-captures every decode | stabilise graph (fix `n_tokens` drift / seed handling) |
| `compute` ≈ total, `build`+`post` tiny | CUDA graph launch is the cost | graph-stability, replay reuse |
| `build` large | graph rebuild per decode | cache graph across identical `gparams` |
| `post` large (MTP hook sync) | `ctx_mtp` decode stalls the main GPU | overlap / reduce sync |
| `top_ops` shows many `FFN`/`MUL_MAT` | weight-streaming compute floor | split-mode / quant / kernel work |

## Results (measured 2026-08-19, GTX 1070 Ti + RTX 3060)

Run with `GGML_CUDA_GDN_PROFILE=1`, winner config (131072, ts 24,12, q4_0 kv,
MTP n=1) vs same minus MTP. Stable generation-phase rows:

| config | total | build | compute | post | graph | sync | nodes (top) |
|---|---|---|---|---|---|---|---|
| MTP n=1 (winner) | 98.0ms | 0.3ms | 87.5ms | 10.0ms | **2C/1R** | 1×9.7ms | 189 MUL_MAT |
| no-MTP | 78.1ms | 0.1ms | 78.0ms | 0.0ms | **1C/1R** | 0 | 112 MUL_MAT |

### Findings
1. **`compute` = 78–87 ms is 90–99% of the decode.** build is tiny (0.1–0.3 ms),
   so the Phase D "graph rebuild" guess is falsified.
2. **The CUDA graph re-captures every decode, MTP or not** (`1C/1R` no-MTP,
   `2C/1R` MTP). It never reaches stable replay. Root cause:
   `ggml_cuda_graph_update_required` (ggml-cuda.cu:3340) compares each node's
   full `ggml_tensor` copy + every `src->data` pointer; some pointer/layout
   changes every decode → returns true → warmup reset → re-capture.
3. MTP adds vs no-MTP: ~9.5 ms compute (189 vs 112 MUL_MAT = the head) + 9.7 ms
   sync stall = ~20 ms, but verifies 2 tokens/decode → still ~14 vs ~11 t/s.
4. MTP-hook `synchronize()` stall = 9.7 ms (the whole `post`). A clean, isolated
   10% overhead.

### Next: why props change every decode
Need to determine WHICH node property flips (data pointer vs ne/nb vs src ptrs).
Likely the scheduler rotates/relocates an input buffer (or dual-GPU split
buffers) so addresses differ each decode. If stable, replay-only compute should
be well under 78 ms (Phase D weight-stream floor ~19–40 ms) → big win.

### CORRECTION (2026-08-19, after adding [CUGR] field-diff diagnostics)

The re-capture theory was a **red herring**. Full measured picture:

`[CUGR]` field-diff showed the only generation-phase flip is **`ne[1]` = the
growing attention/KV context length** in the 16 full-attention layers — never
data pointers. When the graph is stable (long generation, drafts consistently
accepted → `n_tokens` stable), it reaches **`0C/1R` pure replay**.

Per-graph-type breakdown (winner config, steady state):

| graph | n | total | compute | post | sync | what |
|---|---|---|---|---|---|---|
| 0C/0R | 328 | 11.2ms | 8.3ms | 2.5ms | 2.4ms | ctx_mtp MTP-head draft |
| 0C/1R | 412 | 98.1ms | 52ms | 46ms | 45.6ms | **main verify (2 tok)** |
| 1C/0R | 6 | 109.5ms | 73ms | 36ms | 36ms | prompt processing |

**The main verify decode is ~98 ms wall and its total is INVARIANT** whether the
graph captures (run 1: `2C/1R`, compute=87 ms, sync=10 ms) or replays (run 2:
`0C/1R`, compute=52 ms + sync=46 ms). The re-capture merely re-attributes the
time between `compute` and `post`; the ~90 ms of **genuine GPU execution** is
unchanged.

GPU util during generation samples **40–67%** on both GPUs — consistent with
~90 ms GPU work per ~142 ms of wall per 2 tokens (not 100%, but the decode
itself is GPU-bound at ~98 ms).

### Corrected bottleneck ranking
1. **Main decode GPU execution ~98 ms / 2 tokens (~49 ms/token)** — dominant.
   27B Q3_K = 13.8 GiB → ~140 GB/s effective, well under the ~300+ GB/s
   achievable floor → headroom in kernel/attention/split efficiency.
2. **Inter-decode host overhead ~22 ms/token** — ctx_mtp draft decode (~11 ms,
   0C/0R rows) + sampling/KV ops between main decodes. Brought 20.4 t/s
   (2 tok/98 ms) down to the measured 14 t/s.
3. **NOT** graph build (0.3 ms), **NOT** re-capture (stable replay reached),
   **NOT** per-layer PCIe syncs, **NOT** delta-net.

### Recommended attacks (next)
- **Attention over 131K context** in the 16 full-attn layers is the prime
  suspect for the 2× floor gap — grows per token, expensive at long ctx.
- **Tensor-split 24/12 imbalance** (3060 vs 1070 Ti) may under-utilize the 3060.
- **Q3_K kernel / stream efficiency** to close toward the ~47 ms floor.

### Actions
- [x] build llama-server both archs
- [x] `sudo vitriol setup`
- [x] run winner config with `GGML_CUDA_GDN_PROFILE=1`
- [x] capture `[PERF]` rows, fill table
- [x] isolate which node prop flips (answer: `ne[1]` growing context length)
- [x] determine if re-capture matters (answer: no — total invariant, GPU-bound)
- [ ] measure attention cost at large ctx vs short ctx (isolate the 16 full-attn layers)
- [ ] try `--split-mode row` to rebalance 3060 vs 1070 Ti
