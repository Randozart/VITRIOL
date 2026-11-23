# Inkling-2-Mini: DSpark speculative decoding + upstream perf ports

**Date:** 2026-11-22
**Status:** CONFIRMED — implementing
**Research:** `.opencode/plans/inkling-2-unsloth-upstream-research-2026-11-22.md`
**Supersedes as critical path:** `inkling-2-kernel-density-2026-11-22.md` (fused
`clip`/`control` kernels move to deferred; they optimise the wrong variable).

## Problem

Inkling-2-Mini decodes at **15.1 t/s** in our frozen fork, against a measured
7,926 kernels/token (vs the 27B's 2,469). The prior conclusion was that the only
remaining lever was fused CUDA kernels that would trade away bit-exactness
engine-wide. That was wrong: the forward pass does not need to become faster per
token if the engine can produce **2–4 tokens per pass**. Upstream ships exactly
that and our fork already contains the machinery.

## Goal

Raise measured decode t/s for Inkling-2-Mini on this hardware using speculative
decoding, without an engine-wide numerics change, and without disturbing the live
27B engine that serves "Lapis Occultus".

## Approach (staged)

### S0 — Drafter format probe (no GPU)
Download the 0.40 GB `dspark-Inkling-2-Mini-Q8_0.gguf`. Dump `general.architecture`
and every tensor name/shape. Diff against the `create_tensor(tn(LLM_TENSOR_*))`
call sites in `engine/src/models/dflash.cpp` (lines 99–203). Three outcomes:
compatible / name-mapped-adapter-needed / incompatible. Decides everything else for
400 MB instead of an 18 GB model load.

### S1 — Baseline and first speculative measurement
Control: current frozen flags, signature `03c0150ddd05`. Then `--spec-type
draft-dspark -md <drafter> --spec-draft-n-max {2,3} -ngld 99`, draft KV **q4_0**.
Record decode/prefill t/s, acceptance rate, and the `VITRIOL-FINGERPRINT:` line.

### S2 — n_max and prompt-shape sweep
Upstream measured code +68 % and **prose −17 %**, so the win is prompt-dependent.
Sweep n_max ∈ {2,3,4} × {code, prose} prompt on this hardware; report the honest
per-shape result rather than the best cell.

### S3 — `#32855` `mix()` operand swap
Two lines in `src/models/inkling2.cpp:205,208`; +14–37 % prefill claimed, no
numerics trade (same products, different operand order for the backend's benefit).

### S4 — `--no-op-offload` diagnostic
We run 2 splits over 47 layer slots; upstream reports `--no-op-offload` fixes the
garbled-output family (#32710 / #32778 / #32785: 445–509 graph splits).

### Deferred (documented, not abandoned)
- **#32808** kv_rows flash attention with a per-head mask — the real 8k depth fix
  (16.0 → 5.5 t/s collapse today; `llama-graph.cpp:2670` forces FA off when
  `kq_b != nullptr`). Large, separate project.
- Fused `clip`/`control` kernels (18 % / 15 % of kernel count) — still valid, now
  optimises the *verification* pass rather than the token rate.

## Results (measured 2026-11-23)

### S0 — DSpark: BLOCKED, incompatible drafter architecture

Probed both published drafters against our fork's `dflash` implementation.

| file | size | sha256 | verdict |
|---|---|---|---|
| `dspark-Inkling-2-Mini-Q8_0.gguf` (root, Unsloth fmt) | 402,278,560 | `2ec78510…f3773` | fails: `markov_w1` padded to 201024, fork wants 200058 |
| `Mainline_Support/dspark-Inkling-2-Mini-Q8_0.gguf_file` | 503,889,952 | `3da00cf7…d72e0` | fails: `tensor 'fc.weight' not found` |

Full tensor diff for the mainline file: **36 of 58 drafter tensors recognised**.
The fork demands **86 tensors the drafter does not have**, and the drafter carries
**22 the fork never asks for**:

- drafter-only: `attn_k_ctx`, `attn_v_ctx`, `attn_k_ctx_norm`, `ctx_norm`, `ctx_mix`,
  `joint_proj`, `mask_embd`, `markov_ctx_mix`, `markov_ctx_norm`, `markov_w2.bias`,
  `token_embd_norm`
- fork-only: MLA `attn_q_a/attn_q_b/attn_kv/attn_kv_a_norm`, MoE
  `ffn_gate_exps/ffn_down_exps/ffn_up_exps/ffn_gate_inp/ffn_*_shexp/exp_probs_b`,
  hyper-connections `hc_attn_*/hc_ffn_*`, `selector_*`, `fc`, `conf_proj`, `output`

So our `dflash` is a **different DSpark variant** (MLA + MoE + hyper-connections),
not the same architecture with renamed tensors. The drafter is 3 plain-attention
blocks + dense FFN (9216) + markov head + ctx taps.

**Feature-tap infrastructure does exist** (`dflash.cpp:24` reads `target_layers`;
`llama-hparams.h:253` fuses `target_layers × target_hidden`), so a port is
mechanically possible — it needs a new drafter arch (`attn_*_ctx`, `ctx_mix`,
`joint_proj`, `mask_embd`, `markov_ctx_*`, `has_markov_context`), resolved upstream
in still-open PR **#32218**. `dflash.target_layers = [5, 23, 41, 48]` indexes
**loop-expanded** layers of the 24-physical/47-slot model — the same index space
that produced our earlier placement bug, so the tap must be verified against the
unrolled graph. Estimated: a bounded but multi-stage engine project.

### S3 — `mix()` operand swap (#32855): PASS on speed, NOT bit-exact

Applied upstream's exact hunk to `src/models/inkling2.cpp:192-216`. Strict A/B,
identical flags (`-ngl 99 -ts 27,21 --no-mmap -ub 64 -fa on --temp 0`):

| arm | decode t/s | prefill 5-tok | prefill 400-tok | signature |
|---|---|---|---|---|
| baseline | 15.5 / 16.1 | 62.0 / 62.0 | **141.9** | `03c0150ddd05` |
| mix swap | 15.7 / 15.5 | 66.6 / 64.7 | **153.3 (+8.0 %)** | `49c3660b7b9b` |

- Prefill **+5.8 %** (short prompt) / **+8.0 %** (400-token prompt). Upstream claims
  +14–37 %; we measure less on sm_86.
- Decode **flat** — expected, the `query->ne[1] == 1` path is a reshape, and decode
  issues one token per pass.
- **The plan's claim of "no numerics trade" is refuted by measurement.** The swap is
  mathematically a double transpose with identical summands, but `ggml_mul_mat`
  selects a different kernel for `m = n_head` vs `m = n_embd`, changing accumulation
  order within the dot product. Output stays coherent and semantically identical
  (`The user has provided a very short input: "1 2 3 4`), so this is last-bit drift,
  not corruption. **Signature re-baselined to `49c3660b7b9b`.**
- Kept: free prefill on a coherent path. I2 below is amended accordingly.

### S4 — `--no-op-offload`: NOT APPLICABLE, bug does not reproduce

~230-token prompt (well past the 96-token garbled threshold in #32778):

| arm | prefill | decode | output |
|---|---|---|---|
| default | 105.8 | 15.0 | coherent |
| `--no-op-offload` | 106.8 | 15.0 | coherent, identical |

The upstream garbled-output family (#32710 / #32778) **does not reproduce here**,
consistent with our fork lacking `sched_copy_experts` (the mechanism in #32807).
No action needed.

## Invariant status

- **I1** 27B: unchanged — the patch is inside `llama_model_inkling2::graph::mix`,
  which no other architecture executes; `libvitriol` untouched. Engine `active`.
- **I2** AMENDED: DSpark-off baseline signature is now `49c3660b7b9b` (S3 accepted).
- **I3** not exercised (no drafter), vacuously true.
- **I4/I5** not exercised (no drafter); no config mutation, no fingerprint drift.
- **I6** clean: one source file + two Markdown notes.
- **I7** engine restored via `systemctl start vitriol-server.service`.

Numbered properties this change MUST preserve. Each is tied to the check that pins
it. An invariant is a property, not a test name — it must survive refactors.

- **I1 — 27B untouched.** The live engine still serves "Lapis Occultus" and
  `libvitriol` `probe-recommend` output for the 27B GGUF is byte-identical to
  before the change.
  *Pinned by:* `probe-recommend` diff against the recorded baseline; engine
  `/health` + a live completion before every stage is called done.

- **I2 — DSpark off ⇒ nothing changed.** With the drafter flags absent, decode
  signature is exactly `03c0150ddd05` and t/s is within the 15.1–16.0 band.
  *Pinned by:* `/tmp/kd-bench.sh` signature comparison.

- **I3 — Speculation changes speed, not distribution.** With DSpark on, greedy
  output on a fixed prompt is identical to the non-speculative greedy output for
  the same flags. Speculative decoding is lossless only under greedy; any
  divergence means the verify path is wrong.
  *Pinned by:* text diff on three fixed prompts (code, factual, prose) at
  `--temp 0`, both arms.

- **I4 — Draft KV is q4_0.** f16 draft KV is a measured 3× regression (22.7 t/s);
  it must never be the default and must be visible in the fingerprint.
  *Pinned by:* fingerprint line assertion in the S1/S2 reports.

- **I5 — Provenance.** Every result carries the full `VITRIOL-FINGERPRINT:` line
  including the new spec flags (`spec=draft-dspark:<n_max>`, drafter path).
  *Pinned by:* each report excerpting its fingerprint.

- **I6 — Reversibility.** No repository, config or model file is mutated by a
  measurement. The drafter is an additive download; every stage is re-runnable
  from a clean tree.
  *Pinned by:* `git status` clean at each commit; `~/.vitriol/config` unmodified
  (measurements pass flags explicitly, never via `config set`).

- **I7 — Engine restoration.** The 27B engine is stopped only for a model-load
  window and is `systemctl start`-ed again before any stage is declared done.

## Named tests / checks

| check | proves |
|---|---|
| `gguf-dump` tensor list vs `dflash.cpp` call sites | S0 compatibility verdict |
| `/tmp/kd-bench.sh` signature `03c0150ddd05` | **I2** |
| three-prompt greedy text diff, DSpark on vs off | **I3** |
| fingerprint grep for `spec=draft-dspark` + draft KV type | **I4**, **I5** |
| `libvitriol` `probe-recommend` byte-diff | **I1** |
| `git status --porcelain` empty at commit | **I6** |
| `systemctl is-active vitriol-server.service` == active + `/health` ok | **I7** |

## Failure modes to watch

1. **Drafter loops incompatible** — the README states the drafter "taps those
   looped layers" and our DSpark is July-vintage. S0 exists to catch this before
   it costs a model load.
2. **Acceptance rate near zero** — speculation then *adds* cost (draft + verify).
   If acceptance < ~0.5 the honest result is "no gain, revert".
3. **VRAM pressure** — 0.4 GB drafter + draft KV on top of 10.8/10.1 GiB loaded.
   Watch `memory.free` (not `memory.used`) and be ready to shrink draft ngl.
4. **Prose regression** — the upstream +68 % came with a −17 % prose cell. Report
   both, never the average.
