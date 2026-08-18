# C1b: Parallel-Scan Gated Delta-Net Kernel for Qwen3.8-27B MTP Verify

Date: 2026-08-18 20:30
Branch: `vitriol-mellum2`
Goal: remove the MTP-verify serial-scan wall (14 → target ~18-31 t/s) by replacing the
serial `for t in n_tokens` recurrence in `gated_delta_net_cuda` with a parallel-scan
formulation for small n_tokens (MTP verify batch).

## Background / Why

- qwen35 (Qwen3.8-27B): 64 layers, every 4th is full-attention, **48 are Gated Delta Net** (recurrent).
- MTP verify batches n_draft+1 tokens (n_tokens≈6) through all 48 recurrent layers.
- `fused_gdn_ch=true` (default, llama-context.cpp:192) routes n_tokens>1 through the
  **fused serial-loop kernel** `gated_delta_net_cuda` (gated_delta_net.cu:57 `for t in n_tokens`).
- Measured: verify(5) = 308ms vs single-forward 91ms = **3.4×**. 48 serial layers × 6 tokens
  = the wall. Config levers exhausted (Phase A). No upstream help (Phase B).

## The Recurrence (non-KDA path — qwen35's)

Per token t, per state column col (one thread owns a column across all S_v rows):

```
kv[col]     = Σ_i S[i][col] · k[i]                    // scalar per col, warp row-reduce
delta[col]  = (v[col] − g·exp · kv[col]) · beta        // g scalar (non-KDA)
S_new[i][col] = g·S[i][col] + k[i] · delta[col]        // rank-1 state update
attn[col]   = Σ_i S_new[i][col] · q[i]                 // warp row-reduce
```

### Why it's chunkable (affine)
delta depends on S only through the scalar kv = S·k. Therefore the state map is affine:

```
S' = g·(I − β·k·kᵀ)·S + β·k·vᵀ  =  A·S + B
```
- A = g·(I − β·k·kᵀ): identity minus rank-1 (low-rank), scales S
- B = β·k·vᵀ: rank-1 outer product

Chunk composition: S_chunk_end = (A_last···A_first)·S_chunk_start + (composed B). So within a
chunk all tokens compute local (A_t, B_t) in parallel, then compose transitions serially.

## Design

### Dispatch (non-invasive, default-auto)
- `n_tokens == 1` → existing fused AR path (unchanged).
- `n_tokens > 1` → new parallel-scan kernel.
- Toggle env `GGML_CUDA_GDN_SCAN`:
  - `auto` (default) → parallel-scan when n_tokens>1, serial otherwise
  - `off` → always serial (current behavior, A/B reference)
  - `on` → always parallel-scan
  - Missing/unset → auto.

### Kernel structure (parallel-scan, small n_tokens)
Grid/block layout reused from current kernel: grid `(H_v, n_seqs, S_v/num_warps)`,
block `(warp_size, 4)`, each thread owns one S_v column across rows (rows_per_lane).

Per block (head h, seq s):
1. **Local pass (parallel over t)** — all n_tokens compute, no cross-token dep:
   - kv_t[col]   = Σ_i S_0[i][col]·k_t[i]   (uses carry-in S_0 — the ONLY serial input)
   - local A_t, B_t (affine coeffs) OR local delta_t assuming S_0=0
   - local attn_t[col] = Σ_i S_local_t[i][col]·q_t[i]
2. **Serial composition over t** (n_tokens steps, cheap):
   - fold carry S_0 through A_t/B_t to get true S_t per position
   - final output attn needs true S_t: attn_t[col] = Σ_i S_t[i][col]·q_t[i]
3. **Write** final state (S after last token) + all attn outputs.

Note: because delta_t = (v − g·kv)·β and kv_t depends on the *true* prior state, the
parallel-scan must separate the "zero-state contribution" from the "carry propagation".
Exact math worked out at implementation with a CPU/GPU reference diff.

### Correctness / rigour (non-negotiable)
1. Keep serial kernel intact as reference. New kernel behind selector.
2. Extend `tests/test-backend-ops.cpp` `test_gated_delta_net`:
   - add n_seq_tokens = 6 (MTP verify shape), head_size = 128, v_repeat matching qwen35 (48/16)
   - KDA and non-KDA, n_seqs = 1 and 2
   - run BOTH serial and scan kernels via toggle, assert bitwise/tolerance-equal (~1e-5 f32)
3. Run `test-backend-ops` gated-delta-net cases before/after. Must pass.
4. Only after unit green: full-model benchmark at 131K+MTP.

### Performance target
- verify(5): 308ms → target ≤ ~110-150ms (single-forward + small overhead)
- end-to-end: 14.09 → realistic 18-24 t/s (theoretical ~31, Pascal fp32 caps)

## Files
- `ggml/src/ggml-cuda/gated_delta_net.cu` — new parallel-scan kernel + dispatch + env toggle
- `ggml/src/ggml-cuda/gated_delta_net.cuh` — unchanged (same entry point)
- `tests/test-backend-ops.cpp` — add n_seq_tokens=6 + serial/scan A/B test

## PROVENANCE
Independent implementation of affine chunk-composition for the delta-net recurrence.
Prior art consulted: the fork's own `build_delta_net_chunking` graph path (GPL, in-tree) for
the decay/composition math; no upstream CUDA kernel exists (upstream gla/delta-net kernels
are serial). No code copied from external repos. Header added per project convention.

## Sequencing
1. Write plan file ✓ (this)
2. Read full current kernel + confirm exact recurrence/layout
3. Implement parallel-scan kernel + dispatch + env toggle
4. Extend test harness (n_seq_tokens=6, serial vs scan A/B)
5. Build test-backend-ops, run gated-delta-net cases — must pass
6. Full rebuild + sudo vitriol setup + benchmark 131K+MTP
7. Update findings + write kernel-engineering report (Phase D)

## Risks
- Chunk-composition correctness (mitigate: reference diff in unit test, keep serial path)
- f32 numeric drift across scan vs serial (tolerance ~1e-5; if drift > tol, refine order)
- Pascal fp32-bound caps ceiling (chunking removes serialization, not per-element cost)
- Small n_tokens (6) may not fully amortize; if scan slower than serial for 6, keep serial
  for small n_tokens and use scan only at larger (this is what the auto-dispatch is for)

## Safety
- User may keep running the current server during write + build (CPU-only build, no GPU
  contention; running binary unaffected by disk replacement).
- Server stopped ONLY at step 6 (killall + relaunch with new binary).
