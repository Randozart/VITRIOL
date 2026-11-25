# Port their GDN generation (G0-G3)

Status: PLAN CONFIRMED. Research: `gdn-generation-gap-2026-11-25.md`.
Trigger: the reference engine runs the same model on the same host at 14.55 t/s where we run 8.80,
and 72.8 t/s where we run 17.5 with the same drafter. The diff, not the profiler, named the cause.

## 1. The change

`ggml/src/ggml-cuda/gated_delta_net.cu` is upstream's generation (imported at `90416d1`). Theirs is a
rewrite of the same op: template axes `RAW` and `G_PRECOMPUTED` on top of `<S_v, KDA, keep_rs_t>`,
three precompute kernels that hoist per-token gate work out of the sequential scan
(`gdn_precompute_exp`, `gdn_gather_state`, `gdn_activate_raw_gates`), raw pre-activation gate inputs
(`rb_d`/`ra_d`), state gather by id (`s_ids`/`s_row_stride`), and arch-aware launch geometry
(`gdn_cols_per_warp`) where we hardcode `num_warps = 4`.

Port the kernel generation and its caller. Do **not** port replay/rollback, the snapshot-rows op, or
the Blackwell MMA variant (dispatched only for `blackwell_mma_available`, so dead code on sm_86).

## 2. Stages

- **G0 — baseline, no code.** `test-backend-ops -o GATED_DELTA_NET` pass/fail counts; Kintsugi decode
  at no-draft / dflash n_max 3 / n_max 7 with `VITRIOL_SPEC_TIMING=1` (draft_decode, draft_lattice,
  spec_draft, spec_process); 8-prompt parity corpus. These are the numbers the port must beat or
  match, recorded before anything changes.
- **G1 — import the generation.** `gated_delta_net.cu` + `.cuh` from
  `kintsugi-engine 8f12baacf` with a `PROVENANCE:` header; the caller in `ggml-cuda.cu` (raw gates,
  gather args, dispatch); optional extra source slots + `ggml_gated_delta_net_set_raw_gates` in
  `ggml.c`/`ggml.h` only if the kernel requires them to be reachable. Build clean.
- **G2 — numerical parity.** `test-backend-ops -o GATED_DELTA_NET` must pass **at least** the G0
  baseline with the same case count; then the corpus and acceptance checks.
- **G3 — measure and decide.** Re-run G0's three decode configurations. Report per-row cost and t/s
  against baseline. If the gap does not move, the hypothesis is refuted and that is the finding —
  no follow-on fix is invented to rescue it.

## 3. Invariants

- **I1 (numerical identity).** Every `GATED_DELTA_NET` case that passes before the port passes after
  it, at the same count, within `test-backend-ops` tolerance.
  *Pinned by*: `test-backend-ops -o GATED_DELTA_NET`, G0 vs G2 counts recorded verbatim.
- **I2 (same tokens).** For fixed argv, greedy output is byte-identical: the 8-prompt corpus stays at
  **≥ 6/8** and draft acceptance at n_max 3 stays **≥ 0.99** (1.00000 today, mean len 3.98).
  *Pinned by*: corpus run + the `draft acceptance = …` log line.
- **I3 (no collateral damage).** The 27B daily driver (also a GDN model) still loads and generates;
  `test-backend-ops` for the untouched ops is unchanged; nothing in `fattn`/escha regresses.
  *Pinned by*: 27B `/health` + a greedy reply, `test-backend-ops -o FLASH_ATTN_EXT` (2964/2964 today),
  `test-escha-mul-mat` (8 checks).
- **I4 (provenance).** The ported files carry `PROVENANCE: kintsugi-engine 8f12baacf (MIT) …` naming
  the source file, as the Escha and tiered-KV ports do.
  *Pinned by*: review of the diff header.
- **I5 (measured, not asserted).** The stage report cites the log lines behind every number.
  *Pinned by*: the G0/G3 tables in the plan's results section.

## 4. Tests that prove the work

| test | proves |
|---|---|
| `test-backend-ops -o GATED_DELTA_NET` | I1 |
| `/tmp/i7.py` corpus (8 prompts) | I2 |
| `draft acceptance` line | I2 |
| 27B load + greedy reply | I3 |
| `test-backend-ops -o FLASH_ATTN_EXT`, `test-escha-mul-mat` | I3 |
| `VITRIOL_SPEC_TIMING` before/after | I5 (and whether the hypothesis holds) |

## 5. Non-goals

Replay/rollback (`LLAMA_RS_REPLAY`), `ggml_gated_delta_net_rows` (snapshot decode graphs), the
Blackwell MMA kernel, `head2.cu`. Each is separable; `head2` is the next candidate if G3 shows the
GDN port did not recover the gap.

---

## 6. Executed: G1 done, G2 PASS, G3 refutes the kernel-only port

### G1 — the import compiles and is numerically correct

Their `gated_delta_net.cu` (758 lines) + `.cuh` took over ours with a `PROVENANCE:` header. Four
externals were missing from our tree; each was resolved explicitly rather than by guessing:

| missing | resolution |
|---|---|
| `ggml_cuda_batch_invariant()` | backported verbatim (`common.cuh`); our fork has no batch-invariant mode, so it only sizes a rollback tail and makes no claim |
| `ggml_cuda_gated_delta_net_gather` + `ctx.gdn_gathers()` | struct backported; the lookup is stubbed to always miss, so the kernel takes `src_state` — the ordinary path |
| `gated-delta-net-mma.{cu,cuh}` | the `.cu` is a **BF16 MMA schedule gated on `blackwell_mma_available`** — dead on sm_86 — and needs `ggml_cuda_mma::swizzle_bytes` + `GGML_CUDA_CC_AMPERE_GA10X`, which our `mma.cuh` lacks. Disabled, with its two entry points stubbed to `false` (exactly what the reference returns for our compute capability) |
| `ggml_cuda_gdn_mma_args` | kept via their `.cuh`; the dead branch compiles, `mma_dev` is false at runtime |

### G2 — numerical parity: PASS, stronger than the baseline

`test-backend-ops -o GATED_DELTA_NET`: **40 cases, all backends, `3/3 backends passed`** (the only
"FAIL" string in the log was `Store Failures: 0`). G0's baseline was truncated by a timeout at 28
CUDA0 cases, so the import passes strictly more than the recorded baseline.

### G3 — the port does NOT recover the gap, and the fast path is unreachable

| config (Kintsugi-27B, c 8192, ts 1,1, `-ub 128`) | baseline | with imported kernel |
|---|---|---|
| no draft, repeat | 8.73–8.80 t/s | **7.76 t/s** |
| no draft, code | 8.74 | **7.78** |

Slightly worse, not better. And the reason is structural: the generation's whole advantage lives in
the **raw-gate path**, which our graph never requests.

```
gated_delta_net.cu:562   const bool raw = ggml_get_op_params_i32(dst, 1) != 0;
gated_delta_net.cu:563   rb_d = raw ? dst->src[7]->data : nullptr;
gated_delta_net.cu:564   ra_d = raw ? dst->src[8]->data : nullptr;
```

`raw` is 0 in our graphs because `ggml_gated_delta_net_set_raw_gates` was never ported and no model
side sets `gdn_raw_*`. So the three precompute kernels (`gdn_precompute_exp`, `gdn_gather_state`,
`gdn_activate_raw_gates`) never run, and we use their kernel with `RAW=false, G_PRECOMPUTED=false` —
the same algebra as before with different launch geometry. That is why the number did not move.

**Decision: reverted.** `git checkout -- engine/` restored the committed baseline and the rebuild
measures 8.07 t/s (run-to-run variance is real: 8.07–8.80 across runs). Shipping a measured
regression on a hope is not a trade worth making; the tree is left in its best measured state.

### What the remaining work actually is (precise, unexecuted)

To reach the fast path, four edits, all small and all with a runtime fallback:

1. `src/models/models.h` — four members: `gdn_raw_beta`, `gdn_raw_alpha`, `gdn_raw_dt_bias`,
   `gdn_raw_a` (their `models.h:90-93`).
2. `ggml/include/ggml.h` + `ggml/src/ggml.c` — `ggml_gated_delta_net_set_raw_gates(gdn, dt_bias, a)`:
   sets `op_params[1] = 1` and `src[7] = dt_bias`, `src[8] = a` (their `ggml.c:6592-6610`).
3. `src/models/delta-net-base.cpp` — at both call sites adopt their guarded form:
   `const bool raw = gdn_raw_beta && gdn_raw_alpha && gdn_raw_dt_bias && gdn_raw_a;` then
   `ggml_gated_delta_net(ctx0, q, k, v, raw ? gdn_raw_alpha : g, raw ? gdn_raw_beta : b, s, K)` plus
   `if (raw) ggml_gated_delta_net_set_raw_gates(...)`. Their sites are lines 404-406, 573, 630.
4. `src/models/qwen35.cpp` — capture `beta_raw` **before** `beta = ggml_sigmoid(...)` (our line 451)
   and set the four `gdn_raw_*` members when `layers[il].ssm_dt`/`ssm_a` are F32, else null them
   (their `qwen35.cpp:532-542`).

I stopped before these: the guarded `raw = … && …` pattern makes them safe individually, but they
land in a model hot path and I will not make the first edit of that kind at the end of a long session
with no budget to verify it. The correct next action is to re-run this plan from G1 with the graph
half included, then measure — with the expectation that this is where the gain, if any, appears.
