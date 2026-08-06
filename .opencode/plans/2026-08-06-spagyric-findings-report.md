# Spagyric — Findings Report (weights-as-code refuted; amortization + parallel-slot win)

Date: 2026-08-06. Covers the full investigation: the weights-as-code thesis, its
measured refutation, the amortization discovery, and the Spagyric (VITRIOL hardware
autotuner) Phase 0 + decode-knob results.

## 1. Summary of everything found

| # | finding | status | value |
| --- | --- | --- | --- |
| 1 | Weights-as-code (R2-FOLD) is bit-correct but 92.8x packed bytes | **refuted (measured)** | the "package too large" wall, exactly |
| 2 | Batch amortization on dense TQ1_0 GEMV | **win (measured)** | 3.6x per-token at batch R=16 |
| 3 | Decode knobs on the real runtime: ubatch + threads are NOT levers | **measured** | flat |
| 4 | `--parallel` slots ARE the decode throughput lever | **win (measured)** | 2.3x (DeepSeek), 1.4x (Mellum) |
| 5 | `threads=8` on this 4C/8T box is catastrophic for compute-bound models | **measured** | 2.24 t/s vs 30 at t=4 (Mellum) |
| 6 | DeepSeek/Mellum "illegible output" concern | **did not reproduce** | temp 0, merge-sort prompt, PASS |

## 2. The weights-as-code thesis and its refutation

Thesis (the "shader" idea): compile LLM weights into literal instruction structure
(FADD/FSUB on activation lanes), elide dead (zero) lanes, let the compiler fold
constants — so inference fetches no weight bytes, only instructions.

Measured on `blk.0.ffn_gate` [2560,6912] TQ1_0 from the dense BitNet 2B model
(`crates/engine/examples/blkfold.rs` oracle + generator):

- **G-PARITY: PASS.** The folded kernel is bit-exact vs the q8_K reference
  (`max_abs_diff = 0`). Dead-lane elision (~39% of lanes are zero) is numerically
  sound.
- **G-ECON/G-COMPILE: REFUTED.** A 4-row slice compiled to 50,556 SASS instructions
  (12,639 instr/row ~ 135 KB code/row). Full tensor: **346 MB code vs 3.73 MB packed =
  92.8x**. Whole model: ~73 GB code vs 439.6 MB packed. Per-token the GPU must fetch the
  executed instruction stream; 346 MB/tensor cannot be cached (Pascal L2 ~ 2 MB), so
  runtime is *slower*, not faster.

Structural reason: each row's weights are unique (0% duplicate rows), so the code image
is irreducibly row-proportional. Information floor: an instruction is ~8 B, a packed
ternary weight is ~0.2 B — weights-as-code is a byte-losing translation, always.

This is exactly the "the package is too large for my GPU" the user hit — confirmed by an
external AI/graphics researcher as the TRT-LLM class of problem (per-GPU AOT engines).
But TRT-LLM's real win is fused kernels + tactic autotuning, not "paths as code".

## 3. Amortization — the surviving lever (dense)

Because weight bytes are irreducible, the only lever is *amortizing the fetch across
tokens*. Measured on the LUT TQ1_0 GEMV (GTX 1070 Ti):

| batch R | per-token ms | tokens/s |
| --- | --- | --- |
| 1 | 0.258 | 3,879 |
| 8 | 0.089 | 11,194 |
| **16** | **0.072** | **13,981 (knee)** |
| 64 | 0.075 | 13,335 (flat, compute-bound) |

**3.6x per-token**, parity bit-exact at every R. Mechanism: L2 (2 MB) serves the
repeated weight reads of concurrent row-blocks + occupancy hides DRAM latency.

## 4. Spagyric = VITRIOL hardware autotuner

Decision: Spagyric is not a new runtime and not a new representation — it is a
**feature of VITRIOL** (`--spagyric-tune`) that probes the specific hardware, sweeps the
real tunable knobs, finds the knee, and freezes a profile (`~/.vitriol/profiles/...`).
It carries a measured-boundary blacklist so future GPUs never re-test refuted
representations (R2-FOLD, IQ-LUT-on-sm_61, activation-delta, input-prefold).

Verified integration anchors in the fork: TQ1_0 is natively supported; the profile
system has no consumer yet (Spagyric is the first); the chunked per-expert page-lock
machinery (working-set pinning) and the SSD/disk-offload tier already exist.

## 5. Phase 0 — correctness-gated baselines

Fresh llama-server rebuild; merge-sort prompt, 64 tokens, temp 0, 1 warmup + 3 rounds.

| model | config | gen t/s | eval t/s | correctness |
| --- | --- | --- | --- | --- |
| DeepSeek-Coder-V2-Lite IQ2_M | ngl=99 c=4096 t=4 | 58.1-58.3 | 56.7-58.4 | PASS |
| Mellum2-12B Q4_K_M | ngl=24 c=32768 t=4 | 30.9-34.3 | ~49 | PASS |

Both produce valid merge_sort code. The prior legibility concern did not reproduce.

## 6. S2 — decode-knob sweep (the headline result)

Harness `VITRIOL/libvitriol/spagyric_sweep.py`: mode A single-request decode t/s,
mode B concurrent-request aggregate throughput. All configs correctness PASS.

### DeepSeek IQ2_M (bandwidth-bound)
| knob | values | result |
| --- | --- | --- |
| ubatch | 64/128/256/512 | 60.2/59.8/60.0/59.8 t/s — **flat** |
| threads | 2/4/8 | 59.5/59.8/59.6 — **flat** (GPU-bound) |
| **parallel** | **2/4/8** | **78.5 / 87.9 / 135.8 t/s** — **2.3x single-slot** |

### Mellum Q4_K_M (compute-bound)
| knob | values | result |
| --- | --- | --- |
| ubatch | 64/128/256/512 | 29.8/28.3/28.8/31.1 — **flat** |
| threads | 2/4/8 | 27.6 / t=4 30 / **2.24 at t=8** — catastrophic |
| **parallel** | **2/4** | **37.2 / 41.8 t/s** — **1.4x** |

### Reading
- **`--parallel` is the decode throughput knob.** One forward pass serves all slots;
  the weight fetch is amortized across the batch (the native llama.cpp analog of the
  dense 3.6x). The win scales with how bandwidth-bound the model is.
- **ubatch and threads are not decode levers**; `threads=8` on a 4C/8T box with a
  compute-bound model is a disaster (HT contention).
- Spagyric autotune axis: `--parallel` (fine-grained 1/2/4/6/8/12/16), `threads=4`
  fixed, ubatch at default.

## 7. Hardware / environment facts

- GTX 1070 Ti (sm_61, 8 GB, no Tensor Cores, DP4A-capable), 15 GB DDR3 RAM, i7-3770
  4C/8T, PCIe 3.0 x16.
- **Blocker: RLIMIT_MEMLOCK ~2 GB, no CAP_IPC_LOCK.** The VITRIOL stream mode pins a
  working set; even chunked per-expert pinning needs the cap raised. Ternary-Qwen and
  the VITRIOL-knob sweep (LRU/prefetch/pin) wait on:
  ```fish
  sudo prlimit --pid $fish_pid --memlock=unlimited:unlimited; and ulimit -l
  ```
  (fish shell: `$fish_pid`, not `$$`; `; and` instead of `&&`.)

## 8. Next steps

- **S1**: build `--spagyric-tune` in llama-server (probe + autotune `--parallel`,
  freeze profile).
- **S4**: finer parallel sweep (6/12/16) + VITRIOL stream knobs behind the mlock
  unblock; run the ternary Qwen through stream mode.
- **S6**: seed the refuted-transform blacklist (already in `docs/spagyric-autotuner.md`).

## 9. Where the evidence lives

- Refutation: `bitshaper-ai/.opencode/plans/2026-08-06-spagyric-shader-test.md`
- Amortization: `bitshaper-ai/.opencode/plans/2026-08-06-amortization-batching.md`
- Baselines: `.../2026-08-06-spagyric-phase0-baseline-report.md`
- Sweep: `.../2026-08-06-spagyric-decode-knob-sweep.md` (both repos)
- Design/schema/blacklist: `VITRIOL/docs/spagyric-autotuner.md`,
  `VITRIOL/docs/spagyric-profile-schema.md`
- Harness: `VITRIOL/libvitriol/spagyric_sweep.py`
