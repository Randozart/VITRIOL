# E3 — cb_eval Oracle + Parity Ladder: Results Report

Date: 2026-09-01 (runs 17:50-18:10 local)
Plan: `mining-experiment-master-plan-2026-09-01.md`

---

## 1. What was built

- **`llama.cpp/tools/vitriol-oracle/vitriol-oracle.cpp`** — captures every
  non-view graph node (name, type, shape, raw bytes) of one decode via the
  public `cb_eval` callback (`include/llama.h:390`). Output: `<prefix>.bin`
  + JSONL `<prefix>.idx`. Env: `ORACLE_OUT` (prefix), `ORACLE_NO_CB=1`
  (disable capture). Registered in `tools/CMakeLists.txt`.
- **`tools/vitriol-oracle/diff.py`** — node-aligned comparison
  (aligns by name+occurrence: backends fuse ops differently), L0
  byte-exact / L1 cos>=0.999 per node, `--perturb=FILE:NODE:BIT` gate.
- **Provenance headers** per repo licensing rule (inspiration: OurobourOS
  bitnet-rs harness, own work; public llama.cpp API).

## 2. Gate results (Qwen3.8-9B-Q6_K, prompt 5 tok, build-ku2)

| gate | result |
|---|---|
| CPU determinism (2 runs, t=4) | 989/989 nodes byte-exact, token 11751 both |
| CPU thread-count (t=1 vs t=4) | 989/989 byte-exact (CPU is thread-count bit-stable) |
| CUDA determinism (2 runs, dev0) | 989/989 byte-exact |
| Perturbation gate (flip bit 17 of node 500) | detected: node flagged, maxabs 1.5e-5 quantified |
| CUDA-vs-CPU parity | 989/989 structurally matched; 20/989 nodes diverge cos 0.997-0.9999; greedy token EQUAL (11751) |

## 3. Interpretation of the CUDA-vs-CPU divergence

- First divergence appears in q6_k matmul outputs (CPU float dequant-dot vs
  CUDA int8 mmq rounding), amplified over 27 layers + GatedDeltaNet
  recurrence; worst cos 0.9972 (ffn_swiglu-1). No structural mismatch.
- Cross-backend graphs differ structurally on some archs (bge q8_0: CUDA
  321 vs CPU 297 nodes, in-place vs copy) - the name-alignment in diff.py
  handles this; per-node byte-equality across backends is not a meaningful
  gate, only same-backend A/B is byte-comparable.
- **Ladder calibration**: cos>=0.999 per node is the right gate for f32/f16
  graphs and same-backend A/Bs. For quantized-weight cross-backend diffs
  the production contract is L2 (greedy equality) + trend review of the
  per-node report. Documented in `docs/parity-ladder.md`.

## 4. Bonus find: SIGFPE crash in upstream fit machinery (fixed)

**Symptom**: any binary SIGFPE-crashes at init when the GPU is nearly full
AND `-ngl 0` (or generally under the memory-pressure fit path). Repro:
daily server running (10.6/12.3 GiB dev0 used) then
`llama-cli -m <9B> -ngl 0 -st -n 1` -> exit 136.

**Root cause** (`common/fit.cpp`, new upstream fit machinery in this merge):
`sum_projected_model / std::min(uint32_t(mparams->n_gpu_layers), hp_ngl)` at
line 408 -> `min(0, hp_ngl) == 0` -> idiv by zero (crash PC verified in core
dump: `idiv %rdi`). Two further degenerate denominators guarded in the same
file: `mem_high[id] - mem[id]` (fit step-size interpolation, 2 sites) and
`n_ctx_max - n_ctx_min_total` / `sum_projected_used - min_ctx` (context
interpolation; guarded branch falls into the existing else-log).

**Patch**: `common/fit.cpp` (uncommitted, this session) - all three sites
guard denominator <= 0. **Verified**: crash repro now exits 0; non-degenerate
path math identical (guard only changes the zero case). Upstream-report
candidate.

## 5. Hypothesis scoreboard

| hypothesis | verdict |
|---|---|
| H3 oracle reduces kernel-change verification to scripted diff | **PASS** - capture+diff < 60 s end-to-end; deterministic gates green; injected divergence caught |

## 6. Reuse

- E2 (LUT GEMV) and E7 (dp4a TQ) parity gates now use this tool:
  same-backend capture before/after kernel change must stay byte-exact.
- Oracle doubles as a graph-structure inspector for qwen35 (node names list
  available in `.idx`).

## 7. Raw data

- Captures: `/tmp/opencode/oracle-{cpu1,cpu2,cpu1t,cuda,cuda2}.{bin,idx}`
  (transient; regenerate via recipe above)
- Diff logs: `/tmp/opencode/oracle-diff-{cuda,threads,bge}.log`
- Crash cores: coredumpctl `llama-cli` SIGFPE 17:48-17:52
