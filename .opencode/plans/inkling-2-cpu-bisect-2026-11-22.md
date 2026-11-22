# Inkling-2-Mini: CPU-offload bisect of the numerical corruption (2026-11-22)

Status: CONFIRMED 2026-11-22 (owner: "confirm").

Follows `inkling-2-conv-state-2026-11-22.md` (S1–S3 cleared, `-fa on` fixed,
the "frozen logits" finding retracted as a stale-host-buffer artifact) and
`inkling-2-cross-engine-implementations-2026-11-22.md`.

## Problem

Inkling-2-Mini runs on both GPUs and produces *structure* but corrupted text:

```
prompt: 1 2 3 4 5            → 0 2 3 0 0 0 1 0                 (greedy)
prompt: The capital of France is → 0 . The capital of France is 0 0 . ### 0 ###   (temp 0.8)
```

It is not stateless (it echoes the prompt and sampling varies) and not dead
(`2` and `3` land in the right positions), so the defect is a **numerical
corruption in the shared graph machinery**, not a missing state or a frozen
cache.

Everything checked so far has been cleared by measurement: conv recurrent state
(correct size, advances), `n_rs_seq`/`rs_z`, phantom-slot hparams, KV slot
allocation (all 48), no KV `share`, loop graph ctor (identical to upstream), rel
bias placement, chat template, V-aliasing (fixed), flash attention (fixed).

Notably **every** test so far ran fully offloaded and split across two devices.

## Stage S1 — the bisect (one run)

Run with most layers on the **CPU**:

```
llama-cli -st -m ~/Models/inkling-2-mini/Inkling-2-Mini-IQ4_NL.gguf \
  -ngl 10 -ub 32 -c 512 -fa off -n 16 --temp 0 -p '1 2 3 4 5'
```

(`-fa off` on the CPU path, since flash attention on CPU is a different kernel;
`-ngl 10` keeps ~7 GB of weights on GPU and the rest on CPU, which fits easily.)

Decision:

- **coherent** (`… 6 7 8`) → the corruption is in the **GPU expert-matrix /
  layer-split path** → S2a.
- **still garbage** → the corruption is in the **graph math** → S2b.

This halves the problem space in one run.

### Why this is the right bisect

Inkling-2's MoE is a **fused** `ffn_gate_up_exps` tensor of shape
`[2304, 2304, 128]` (17.6 GB at IQ4_NL) — a 3-D expert tensor with a fused
gate+up axis. The fork has its own expert paths (`build_lora_mm_id`,
`ffn_up_exps_s` per-expert scale, `vitriol-buffer.cpp` placement) that **no other
arch in this fork exercises at this shape**. Moving layers to the CPU changes
which backend runs those ops without changing the graph.

## Stage S2a — if GPU-side

1. `-ngl` ladder 10 → 24 → 48 on **one** card (ctx 512, ub 32) to separate "one
   GPU" from "two GPUs" (weight placement, `n_layer_phys` split, cross-device
   expert fetch).
2. Inspect the fused expert matmul: `build_lora_mm_id(gate_up_exps, cur,
   selected_experts, up_exps_s)` and the per-expert `.scale` wiring added in
   `a1f42226` — verify the scale is applied to the right half of the fused
   axis. A mis-applied per-expert scale would corrupt every expert's output
   while keeping shapes valid, which matches the symptom exactly.
3. Diff `mul_mat_id` dispatch for `IQ4_NL` at `ne = [2304, 2304, 128]` against
   upstream (does the fork's `mul_mat_id` support a 3-D fused expert tensor for
   this quant type, or silently fall back to something wrong?).

## Stage S2b — if graph-side

1. Dump per-layer `attn_out` / `ffn_out` RMS norm for layers 0..5 at one decode
   and look for a blow-up or collapse. The healthy stream should stay O(1..10);
   logits saturating at the softcap (`max=27.47` vs `cap=30` ⇒ pre-cap ≈ 75)
   suggests a growing stream.
2. Check `mix()` / `attn_res` block bookkeeping (`blocks.size()` vs
   `il / block_size`) — H5, still untested.
3. Check the SWA `v_idxs` path (H2, still untested): assert
   `get_v_idxs_swa()` non-null and equal to `get_k_idxs_swa()`.

## Stage S3 — gate

```
llama-cli -st … -ngl 99 -ts 27,21 --no-mmap -ub 32 -c 1024 -fa on \
  -n 32 --temp 0 -p 'The capital of France is'
```
Pass = ≥ 2 distinct tokens **and** a plausible continuation. Then repeat with
the GGUF chat template. Also run the engine's existing test suite to confirm no
regression.

## Stage S4 — cleanup

Strip every `INKLING_DBG` / `VITRIOL_INKLING_DBG` probe, clean rebuild, commit
per stage, and restore the engine via
`systemctl start vitriol-server.service`.

## Invariants

- **I1 (parity):** no arch other than inkling2 changes behaviour; any change is
  arch-gated. *Pinned by* `i1-parity` — `libvitriol probe-recommend` for the 27B
  GGUF identical before/after (`qwen35, 65 layers, 11.9 GiB`), and the existing
  engine test suite green.
- **I2 (restorability):** `vitriol-server.service` active and `/health`
  `{"status":"ok"}` before any stage is called done. The unit is
  `Restart=always`, so it is stopped with `systemctl stop` (never `kill -9`) and
  GPU holders are killed by PID from `nvidia-smi --query-compute-apps`.
  *Pinned by* `i2-restored`.
- **I3 (no debug shipped):** zero `INKLING_DBG` / `VITRIOL_INKLING_DBG` in the
  tree. *Pinned by* `i3-clean` — `grep -rn` empty.
- **I4 (coherence gate):** greedy 32-token continuation has ≥ 2 distinct tokens
  and is semantically plausible, raw and templated. *Pinned by*
  `s3-coherent-gate`.

## Named checks

| name | proves |
|---|---|
| `s1-cpu-bisect` | whether the corruption is GPU-side or graph-side |
| `s2a-gpu-path` | the split / fused expert matmul is sound (or names the fault) |
| `s2b-graph-math` | per-layer stream sanity; H5 blocks; H2 SWA v idxs |
| `s3-coherent-gate` | coherent text, raw and templated |
| `i1-parity` / `i2-restored` / `i3-clean` | invariants |

## Risks

- The CPU path may be slow (26B IQ4_NL, partial offload) — expect minutes per
  run, not seconds. Use `-c 512 -n 8`.
- CPU and GPU kernels differ in accumulation order, so a *small* difference in
  output is expected; only the coarse question (coherent vs garbage) is being
  asked.
- The fork's CUDA `mul_mat_id` may not support IQ4_NL fused 3-D experts at all,
  in which case S2a step 3 needs a real kernel addition (S4-fattn territory) —
  in that case stop and report, do not fake it.

## Status

- 2026-11-22: CONFIRMED, starting S1.

## S1 RESULT — bug found and fixed. THE GATE PASSES.

The CPU bisect worked exactly as intended, and the answer was immediate: the CPU path
did not produce garbage, it **aborted**.

```
ggml-cpu/ops.cpp:4703: GGML_ASSERT(ggml_is_contiguous(src0)) failed
```

Naming the offending tensor (temporary instrumentation in the SCALE assert) gave:

```
INKLING_DBG SCALE src0 name='ffn_moe_gate-0' type=0 ne=[1152,8,1,1]
                  nb=[4,9216,73728,73728]
```

`nb[1] = 9216` where a contiguous F32 tensor with `ne[0] = 1152` has `nb[1] = 4608`
— a 2× row stride. It is the **gate half of the fused `ffn_gate_up_exps` tensor**,
a strided `ggml_view_3d` (the `up` half is the twin at `offset n_ff*nb0`).

### The bug

Upstream `8a97fa98` feeds those two strided views straight into the `LLM_FFN_SITU`
activation:

```cpp
ggml_tensor * act = ggml_scale(ctx0, ggml_tanh(ctx0, ggml_scale(ctx0, cur, 1.0f/beta)), beta);
```

That is legal only because upstream's (newer) ggml handles non-contiguous SCALE/TANH
inputs. **This frozen fork does not.** The CPU kernel asserts; the CUDA kernel reads
the views with the wrong stride and silently corrupts every expert output while
keeping every shape valid. That is why the model produced *structured but garbage*
text and why a very long series of shape/state/placement checks all came back clean.

### The fix (`src/llama-graph.cpp`, build_moe_ffn fused path)

Materialise both halves:

```cpp
cur = ggml_cont(ctx0, ggml_view_3d(ctx0, gate_up, n_ff, ..., 0));
up  = ggml_cont(ctx0, ggml_view_3d(ctx0, gate_up, n_ff, ..., n_ff * gate_up->nb[0]));
```

Cost: two small copies per MoE layer, far below the matmul.

### Before / after

| prompt | before | after |
|---|---|---|
| `1 2 3 4 5` | `0 2 3 0 0 0 1 0` | `[Start thinking] The user has provided a very short input: "1 2 3 4` |
| `The capital of France is` | `0 . The capital of France is 0 0 . ### 0` | `[Start thinking] The user asks: "The capital of France is". This is a simple factual question. The capital of France is Paris. … Need final only.` |

`[ Prompt: 55.8 t/s | Generation: 15.9 t/s ]` at `-fa on -ub 16 -c 1024 -ts 27,21
--no-mmap`, both GPUs.

### Why this took so long / the lesson

Every prior check compared *shapes, states, and structure* — all of which were
correct. The defect was a **stride** mismatch that only manifests as wrong numbers.
The two things that finally found it:

1. running the model on a backend that **asserts** instead of lying (CPU), and
2. naming the offending tensor in the assert rather than reasoning about it.

Inventing instrumentation that *names the node* is far cheaper than any amount of
source diffing once the model itself is suspect.

### Invariants

- I1 parity: the change is inside `build_moe_ffn`'s fused-`gate_up_exps` branch. It
  makes that branch correct for every arch that uses it, and adds two copies only
  where a fused expert tensor exists. Verified `probe-recommend` for the 27B
  unchanged (`qwen35, 65 layers, 11.9 GiB`).
- I2 restored: `vitriol-server.service` active, `/health` ok.
- I3 clean: all instrumentation reverted (`grep -rn INKLING_DBG engine/src engine/ggml/src`
  empty), clean rebuild `BUILD:0`.
- **I4 coherent gate: PASSES.**
