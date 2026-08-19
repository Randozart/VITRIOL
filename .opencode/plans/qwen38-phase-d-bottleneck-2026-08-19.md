# Phase D: Locating the Real MTP Verify Wall (C1b Parallel-Scan Falsified)

Date: 2026-08-19 07:30
Branch: `vitriol-mellum2` (llama.cpp submodule), working tree instrumentation
Status: measurement campaign in progress

## Result: C1b parallel-scan delta-net kernel is FALSIFIED

The MTP verify wall (308 ms) is NOT the serial delta-net recurrence.

### Evidence (env-gated `cudaEvent` timing, `GGML_CUDA_GDN_PROFILE=1`)

test-backend-ops isolated op (no graph capture), RTX 3060:

| shape (S_v=128, kda=0) | ms/call |
|---|---|
| H=8,  n_tokens=1 | 0.728 / 0.478 (warm) |
| H=8,  n_tokens=6 | 0.728 |
| H=48, n_tokens=1 | 0.473 |
| H=48, n_tokens=6 | 0.532 |

- Real model (graph-captured, live server): delta-net = **0.016-0.036 ms/layer** at
  n_tokens=2 (MTP hook pairs). 48 layers ≈ **1-2 ms** per pass. Negligible.
- Even at worst-case isolated cost (0.532 ms × 48 = 25.5 ms), delta-net is **≤8%** of the
  308 ms verify.

Key scaling fact: n_tokens 1→6 costs only **+12%** (0.473 → 0.532 ms). The serial `for t`
loop is essentially free; the kernel is launch/occupancy bound, and 6 tokens cost ~same as 1.

### Arithmetic against the cycle

- Verify(6 tokens) = 308 ms = **51 ms/token amortized** — already better than 6 × 91 ms
  single-forward (546 ms). Batching works.
- Compute floor at 6 tokens ≈ 20-40 ms (FFN ~7-10 ms + attention + delta-net ~2 ms).
- → **~270 ms of verify is NOT matmul compute.** Structural, config-insensitive cost
  (matches flat Phase A: every ts/MTP/ubatch lever stuck in 13.5-14 t/s band).

### Leading hypothesis for the ~270 ms

Cross-GPU per-layer synchronization: tensor split (ts 24,12) across two different-arch
GPUs (sm_86 + sm_61) over PCIe gen3 — one host-sync + buffer copy per layer × 64 layers.
Alternative/contributing: MTP rollback machinery (`llama_memory_seq_cp`/`seq_rm`, spec
checkpoint), and/or graph replay overhead.

## [DEC] measurement — the MTP draft-length-1 bug (found + fixed)

The [DEC] timer revealed every verify was `n_tokens=2` (1 sampled + 1 draft) at ~97.5 ms,
**never 6** despite `--spec-draft-n-max 5`. Root cause:

- `common_speculative_state_mtp::draft()` chains drafts: k=0 seeds from ctx_tgt `t_h_pre_norm`,
  k>=1 seeds from ctx_dft `t_mtp_out`.
- The dense `qwen35_mtp` graph (`src/models/qwen35-mtp.cpp`) set `res->t_h_pre_norm` but
  **omitted `res->t_mtp_out`** → `get_t_mtp_out()` returned null → loop broke after k=0.
- The MoE variant (`qwen35moe-mtp.cpp:236-237`) sets BOTH — so dense was simply buggy.

Fix (working tree): `res->t_mtp_out = cur;` added in `qwen35-mtp.cpp`.

### Post-fix draft sweep (winner config, 120 tokens, GGML_CUDA_GDN_PROFILE=1)

| `--spec-draft-n-max` | t/s | draft accepted | notes |
|---|---|---|---|
| pre-fix (5, draft len forced 1) | 14.37 | 39/39 (100%) | baseline |
| 5 | 9.01 | 83/174 (48%) | deep chained drafts decay |
| 3 | 11.25 | 80/117 (68%) | |
| 2 | 12.85 | 75/88 (85%) | |
| 1 | 13.45 | 59/59 (100%) | = baseline (noise band 13.5-14.4) |

Verified verify-cost scaling: n_tokens 2→6 costs 97.5→169.8 ms (+64%) — sub-linear, batching
works. The wall is NOT verify size; it is chained-draft quality + MTP-head decode cost
(~8 ms/ctx_dft decode × drafts). `p_min=0.75` (top_k=1) gates chained drafts hard (avg
1.29 drafts/cycle at n_max=2).

Conclusion: **draft length 1 is the economic optimum for this model's MTP head** (trunk-seeded
depth-1 is excellent, chained depth>=2 drifts). Keep the `t_mtp_out` fix (correct, aligns dense
with MoE) but set `--spec-draft-n-max 1`. Existing AGENTS.md/config profiles using n=5 will
REGRESS post-fix (n=5 → ~9 t/s) unless lowered.

### Structural overhead remains the real wall

Per cycle (n_max=1): verify(2) = ~97.5 ms for ~1.7 useful tokens (~57 ms/token) vs compute
floor ~10-15 ms for 2 tokens → **~85 ms of structural overhead per verify** (cross-GPU
per-layer sync + graph replay). This, not delta-net and not draft depth, is the lever for
pushing beyond 14 t/s. Next phase: whole-layer-per-GPU placement / sync reduction / graph
node fusion.

## Files touched (working tree, uncommitted)

- `ggml/src/ggml-cuda/gated_delta_net.cu` — env-gated timing (graph-capture-safe).
- `tests/test-backend-ops.cpp` — verify-shape cases (H=48 S_v=128 n_tokens 1/6, kda 0/1) → 22/22 pass.
- `tools/server/server-context.cpp` — decode-level `[DEC]` timing.
- `src/models/qwen35-mtp.cpp` — `res->t_mtp_out = cur;` (dense MTP draft chaining fix).

## Decision

Do NOT implement parallel-scan. Keep the `t_mtp_out` fix + `--spec-draft-n-max 1`. The lever
for >14 t/s is the structural verify overhead (cross-GPU sync / graph replay), target of the
next phase.