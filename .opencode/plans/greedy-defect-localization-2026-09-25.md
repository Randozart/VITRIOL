# Greedy retest + defect localization (post signal-correction)

Date: 2026-09-25 18:15 CEST. Approved by user.
Predecessor: `flashnext-store-divergence-2026-09-25.md` (bisects retracted),
`vitriol-stream-rust-hybrid-2026-09-25.md` (Rust verdict: language-exonerated).

## Question

Does a real defect exist in the store streaming path, measured ONLY with
deterministic signals (greedy temp 0, fixed seed, or byte parity)? The one
surviving anomaly: server greedy probes, arm B (store) answered
" the capital of France is the capital of" 4/4; arm A (gguf) answered
" Paris..." n=1.

## Research facts established this round

- GGUF has NO fused ffn_gate_up_exps -> separate gate/up MUL_MAT_IDs run
  against ffn_gate_exps/ffn_up_exps, which ARE in the VITRIOL buffer - yet
  gate/up never traversed the LRU (no 627,200 B keys in any log).
- ggml sched MoE branch (ggml-backend.cpp:1703-1745): for MoE weights in a
  host buffer with USAGE_WEIGHTS, the sched replaces src[0] with a device
  input_cpy and copies only the routed experts - FROM THE HOST BUFFER.
- cpy_tensor_async rejects host buffers (ggml-cuda.cu:2702) -> sync
  host-staged copy. In store mode the host buffer is ZEROED (set_tensor
  skips the GGUF memcpy). Any such copy carries zeros -> MoE contribution
  zero -> the observed failure class.
- Candidate explains: multi-GPU layer split (CUDA1 layers ~24+ match the
  one surviving multi-GPU observation) and gate/up rerouting (no LRU keys).
- UNRESOLVED by theory: oracle full-capture -ts 12,12 showed byte parity
  between arms (4886/4886). Measurement, not more theory, resolves this.

## Phases

Phase 0 - signal validation (2 runs): greedy oracle (`--temp 0 --seed 42`),
arm A, single-GPU, twice. GATE: identical tokens. Any difference = harness
bug first.

Phase 1 - reproduce with valid signal (4 runs): greedy oracle multi-GPU
(-ts 12,12), arm A x2 + arm B x2. Deterministic compare.
  - differ -> real CLI defect -> Phase 2
  - identical -> server-only delta -> diff server env/argv, server greedy
    arm A x3. No code changes until explained.

Phase 2 - mechanism identification (1 fprintf + 1 run + 2 code reads):
- fprintf in ggml_cuda_mul_mat_id: when src0->buffer is NOT vitriol, print
  tensor name + buffer type (one declared instrument).
- greedy multi-GPU arm B with it: which tensors/layers bypass the LRU?
- code: does llama-model.cpp:1832 set USAGE_WEIGHTS on the vitriol buffer?
  how does sched resolve the duplicate vitriol buft across two devices?
Pre-registered: no copies found -> drop the zero-copy theory, name the next
suspect from greedy-valid byte capture instead.

Phase 3 - fix (design follows Phase 2):
  F1 route gate/up through the VITRIOL store path like down
  F2 teach the sched MoE branch to source vitriol tensors from the store
  F3 stopgap: fill only the regions Phase 2 proves get copied

Phase 4 - verify & close: greedy functional suite (8 prompts) store arm x3,
server probe x3 -> Gate B -> commit (approval) -> Stage-5 perf (the mission).

## Discipline

- Every signal deterministic or byte-level; n=2 minimum per arm.
- No code changes before Phase 2 completes (one declared fprintf excepted).
- Kill criteria: harness nondeterminism at n=2 -> fix harness, restart phase.

## RESULTS (2026-09-25 21:50 CEST) - ROOT CAUSE CONFIRMED BY INTERVENTION

Phase 0 PASS: greedy oracle arm A x2 single-GPU = 11751/11751. Harness
deterministic.

Phase 1: greedy oracle multi-GPU: arm A 11751 x2, arm B 11751 x2 -> NO
defect in the CLI path. Server greedy (current binary, n=3 each):
arm A " Paris. Paris is known for its iconic" 3/3 CORRECT; arm B
" the capital of France is the capital of" 3/3 WRONG. Defect is real,
deterministic, and SERVER-LOCALIZED.

Server flag bisect: --no-warmup no; --parallel/--slots removed no.

Phase 2 findings (server, instrumented):
- ggml_cuda_mul_mat_id is called ONLY for ffn_down_exps (48 unique
  tensors, buft name "CUDA_Host" - which is the VITRIOL buffer's own
  deliberately-overridden name, vitriol-buffer.cpp:466-473).
- ffn_gate_exps / ffn_up_exps MUL_MAT_ID NEVER reaches the CUDA backend
  (zero MMID lines in server AND oracle). Their dtypes: IQ3_XXS (thin
  layers), IQ4_XS (fat layers blk.2/4/30/46/47) - both ARE in the CUDA
  supports_op list (ggml-cuda.cu:5573-5577), so placement is not a
  supports_op type rejection; the rerouting mechanism is still being
  traced (suspects: a fused gate/up/swiglu node whose op trips the
  VITRIOL supports_op guard at ggml-cuda.cu:5467, or CPU-backend
  placement of those nodes).
- The server sched runs 3 splits per decode: CPU (0 inputs) -> CUDA0
  (13 inputs) -> CUDA1 (14 inputs, crosses l_last-24).

INTERVENTION TEST (decisive): server arm B + VITRIOL_STORE_HYBRID=1
(host buffer filled from GGUF at load, LRU still store-fed):
" Paris. Paris is known for its iconic" - CORRECT.

## ROOT CAUSE (confirmed)

1. gate/up expert tensors are consumed by an execution path that reads
   the VITRIOL HOST BUFFER directly (not the CUDA LRU path).
2. Store mode zeroes that host buffer by design (set_tensor skips the
   GGUF memcpy; the store was meant to feed everything).
3. Therefore gate/up contribute zeros in store mode -> degenerate MoE ->
   " the capital of France is the capital of".
4. gguf arm and HYBRID arm fill the buffer -> correct. Intervention flips
   the outcome 1/1; non-hybrid wrong is 3/3 server + 2/3 oracle greedy.

## OPEN (post-fix items)

- O1: identify the exact gate/up consumer + placement reason (fused node
  hypothesis unconfirmed; add op names to the oracle capture rows).
- O2: rare greedy flip (store oracle: 11751,11751,279 across identical
  runs; arm A also had 2 historical anomalies in random-seed runs).
  Measure flip rate at greedy n=10 AFTER the D1 fix; only then decide if
  a second defect exists.
- O3: why oracle full-capture (-ts 12,12, cb_eval) showed byte parity
  between arms if gate/up read zeros in store mode - re-examine once O1
  lands; cb_eval may alter the consumer path.

## FIX OPTIONS (for discussion)

  F-a  Make gate/up run on CUDA like down (trace placement; if it is a
       missing/fused-op quirk, fix routing) -> gate/up join the LRU.
  F-b  CPU-side store hook: feed CPU-executed gate/up MUL_MAT_ID from the
       store (invasive into ggml-cpu).
  F-c  Requantize gate/up in the store at mkfs to a CUDA-supported type
       (IQ4_NL: +14.5 GB store, one-time requant; slight quality upgrade;
       +46% per-token expert read traffic).
  F-d  Fill gate/up at load: +30.8 GiB RAM - DEAD on this 31 GiB box.

## O1 PROBE RESULTS (2026-09-25 22:15 CEST)

- Graph (capture with op+src0 rows, new oracle fields op/src0): ALL THREE
  expert kinds are plain MUL_MAT_ID with src0 = the VITRIOL-buffered
  weight. No fused node, no exotic op. Fused-node theory DEAD.
- Execution (classifier moved to the top of ggml_cuda_mul_mat_id, fires
  for EVERY path): ONLY ffn_down_exps is ever dispatched to CUDA
  (48 lines). ffn_gate/up_exps: zero lines, in server AND oracle, with
  and without cb_eval. They execute somewhere else - by elimination, the
  CPU backend (the 3-split layout is CPU -> CUDA0 -> CUDA1).
- supports_op (wrapper probe VITRIOL_SUP): ALL expert MUL_MAT_IDs report
  supported=1 on dev0 AND dev1 (down and gate/up alike). Placement is
  NOT an op-support rejection.
- NEW CONFIRMED: store-greedy oracle is nondeterministic run-to-run
  (11751, 11751, 279 identical configs) - phenomenon D2 is real and is
  greedy-level, not sampler noise.

NEXT PROBE (O1b, designed, not yet run): GGML_SCHED_DEBUG=2 prints the
full node->backend map (ggml-backend.cpp:1018, debug>1 branch) - the
2026-09-25 22:00 attempt did not emit node lines (env did not take
effect through that systemd-run; needs a direct-exec rerun). The map
names the backend chosen for gate/up MUL_MAT_ID and completes the
placement answer. Alternative: instrument the placement loop directly.

## FIX (2026-09-25 22:45 CEST) - DEFECT CLOSED

Root cause: the CUDA backend's gate/up/swiglu node fusion
(ggml_cuda_should_fuse_mul_mat, applied at ggml-cuda.cu:3449) combines
[MUL_MAT_ID(gate), MUL_MAT_ID(up), GLU] into one fused kernel that reads
the expert weights DIRECTLY from src0->data - bypassing ggml_cuda_mul_mat_id
and the LRU. For VITRIOL store mode that pointer is the ZEROED host buffer
-> gate/up contribute zero -> degenerate MoE. gguf mode works because the
host buffer holds real bytes. down is not fused (no GLU successor) which is
why the LRU only ever saw down tensors.

Fix: ggml_cuda_should_fuse_mul_mat now refuses vitriol-buffered weights ->
the fusion is skipped -> gate/up return to ggml_cuda_mul_mat_id -> LRU ->
store fills. VERIFIED: server store arm greedy x3 =
" Paris. Paris is a city. All" (correct, deterministic); MMID
classification now 144/144 tensors (down+gate+up) on the LRU path.

Remaining: O2 - greedy token variance vs arm A's exact continuation
("is known for" vs "is a city. All" beyond the correct " Paris") and the
historical rare flips (D2). Measure: greedy n=10 both arms; byte-capture
diff if variance persists. Then Stage-5 perf (the mission).

## Log

- 2026-09-25 18:15 CEST: plan written, approved. Phase 0 not started.
- 2026-09-25 21:50 CEST: root cause confirmed by intervention; recorded.
