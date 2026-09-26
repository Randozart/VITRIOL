# Gate B close + Stage-5 perf — 2026-09-26

Predecessor: `greedy-defect-localization-2026-09-25.md` (defect closed:
gate/up/swiglu CUDA fusion bypassed the LRU and read the zeroed host buffer;
fixed in inner `aacdc7138` — fusion now refuses vitriol-buffered weights;
verified server store arm greedy 3/3 correct, MMID classification 144/144).

User directives (binding):
- **Correctness is the gate. We do NOT need precise determinism.** Token
  parity between arms/runs is not a pass criterion; degenerate output is.
- Hold off restoring `vitriol-server.service` until after perf work.
- Measure sampling-independent signals: greedy (`temp 0, top_k 1, seed`)
  or byte parity. No sampler-noise conclusions (2026-09-25 retraction).
- Commit after user approval per stage.

## Stage 1 — Gate B (correctness only)
1. Launch store server, C++ backend (same config as the fix verification:
   `VITRIOL_MODE=stream VITRIOL_STORE=1 VITRIOL_STORE_PATH=/dev/nvme0n1p3
   -ngl 99 -ts 12,12 -c 512 -ub 64 -fa on`).
2. `scripts/vitriol-store-functional-suite.py` ×3. Pass = every prompt
   yields a correct/meaningful answer each run. Token drift OK; loops,
   garbage, refusals not.
3. If any wrong answer: oracle byte-diff bisect (capture now trustworthy —
   all 144 expert tensors flow the LRU path). O2 flip-rate probe only if
   an actual wrong answer appears.

## Stage 2 — Land
4. Commit oracle capture rows (inner; `tools/vitriol-oracle/vitriol-oracle.cpp`
   +15/−3 uncommitted), update outer submodule pointer. Keep O1
   instrumentation (print-once per unique key, negligible).

## Stage 3 — HELD (user): service restore deferred to post-Stage-4.

## Stage 4 — Stage-5 perf (ranked)
5. **LRU capacity + device split.** Working set 1440 slices/token (48L ×
   10 experts × 3 kinds, ~1.0 GiB) vs pool 2048 MB = 1233 slots →
   guaranteed thrash (31,540 evictions/suite-run). Now with the fusion
   fix, gate/up stream too — same working set, exactly as budgeted.
   a. Fix `vitriol_cuda_get_extra_bufts` ignoring `dev` (all MoE on GPU0).
   b. Split pool across both cards (~6.3 + 8.7 GiB free → ~6–8k slots).
   c. Verify: evictions → 0 over the suite; t/s before/after.
6. **Async/batched store reads** (Rust base): 1.16 ms/read latency-bound
   (~0.8 GB/s of multi-GB/s Optane). Batch per-expert up/gate/down reads,
   overlap with compute.
7. **Predictor prefetch**: `vitriol_predictor_prefetch` plumbing exists;
   strong temporal locality in MoE routing.

Rejected (do not re-litigate): size-priority, kind-residency, whole-layer
pinning — see predecessor plan §Deferred.

## Log
- 2026-09-26: plan written. Stage 1 starting.
- 2026-09-26 late (session 3): TRIGGER ISOLATED - THE SHAPE CHANGE.
  Server, hybrid, fresh: S1..S4 = FOUR different 12-token prompts in a
  row: ALL CORRECT (same-shape chains never collapse, content
  irrelevant). S5 = first 20-token prompt: COLLAPSE. S6/S7 = the old
  12-token shape: ALSO DEAD - the first new-shape request poisons the
  whole process PERMANENTLY. Every historical death = a request whose
  token count differed from all earlier requests; every pass = same
  shape (fix-verification probes x3, long48/64, S-chain).
  Additional eliminations: -fa off (no effect), VITRIOL_POOL_RESET=1
  (no effect), upstream bug (plain -ngl 0 CPU server AND -ngl 4 GPU
  server both handle request 2 correctly).
  ORACLE IS BLIND: two-shot oracle (shot A 12tok + shot B 20tok with 3
  decode steps each, KV cleared between) - every segment's result_output
  byte-IDENTICAL between after-A and solo-B (only stale-memory diffs in
  unconsumed lanes: 95 floats at hidden dim 2559, GET_ROWS tails).
  cb_eval's synchronous per-node eval masks the bug - oracle unusable
  for this defect (re-confirms the old "cb_eval masking" note).
## BREAKTHROUGH (session 4): the cliff is n_tokens >= 15/16 - fused GDN CH
  - A1 (20 tok) as the FIRST request on a FRESH server: DEAD immediately.
    There was never a "shape change" requirement - every earlier "first
    request" was <= 12 tokens.
  - Boundary bisected with first-request probes on fresh servers:
    14 tok ALIVE, 16 tok DEAD (17, 18, 20 DEAD). Cliff at 15/16.
  - 16 = llama-context.cpp:58 llm_fused_op_gdn_ch_probe.n_tokens_per_seq.
    cparams.fused_gdn_ar = fused_gdn_ch = true HARDCODED (llama-context.cpp
    234-235), auto_fgdn = false -> the fused GDN chunked op is enabled
    UNPROBED. At n_tokens >= 16 the model builder emits the CHUNKED GDN
    fused op -> collapse. Server-only: the ORACLE escapes because cb_eval
    forces unfused per-node graphs; PLAIN escapes (its fused path is
    inert/different). Reinterprets ALL prior evidence consistently.
  - NEXT STEP (one-liner): force cparams.fused_gdn_ch = false (test first,
    maybe ar too) for the vitriol server and rerun F1->A1->suite. If Gate B
    passes: gate the fused GDN ops off whenever vitriol buffers are active
    (or fix the actual fused-GDN-kernel/sched interaction; secondary).
  - Server init logs contain NO resolve_fused_ops lines (all auto_* false
    for GDN; only DSV4-HC probes appear in oracle logs) -> no device check
    ever ran for the fused GDN ops.

  PREVIOUS NEXT LEADS (superseded but kept for depth):
  1. The sched/gallocr REALLOC that a new n_tokens triggers: cudaMalloc
     /cudaFree of compute buffers while g_lru_stream still has queued
     DMAs; extra_bufts handling on graph rebuild. Instrument buffer
     alloc/free + LRU activity around the first new-shape request.
  2. Server-side logits hash per request (no cb_eval) to find the first
     divergent request state; then bisect WITHIN the server using
     VITRIOL_MATMUL_SYNC + trace (probes exist and are cb-free).
  3. ggml_cuda_mul_mat_id sync loop scratch sizes (ids_buf_dev,
     src1_sorted, dst_sorted scale with ne12) - check pool_alloc reuse
     across the shape change.
- 2026-09-26 evening: NEW DEFECT (blocks Gate B) — "request-2 collapse".
  Suite: req1 correct, every later DIFFERENT-prompt request = token-0 spam
  (tokens [0,0,0...]); SAME-prompt repeats always correct. Plain server
  (-ngl 4, no VITRIOL env) is CLEAN. Failure reproduces in: stream+store,
  hybrid (filled host buffer), MATMUL_SYNC, NO_GRAPHS, LRU_MB=2560,
  LRU_MB=4096, single-GPU, after two real fixes (below). NOT reproduced:
  NO_LRU=1 (pool off → host-read garbage, but stable/deterministic —
  expected).

  ELIMINATED (each verified, do NOT re-test):
  - Store data: 0/73,728 slices differ vs GGUF (store_vs_gguf now
    exhaustive: all 48 layers x up/gate/down x 512 experts).
  - Host buffer: VITRIOL sentinel (golden bytes captured at load, pure-CPU
    memcmp at every ensure) — 0 divergences across requests.
  - LRU fills: 3-way VHASH (expected vs copy_source vs VRAM, after
    cuStreamSynchronize) — 0 violations over 122k fills.
  - Slots at read time: VITRIOL_LRU_READBACK (sync D2H at kernel use) —
    0 bad in the combined run. (Earlier READBACK zeros = artifact of
    racing the async DMA; legacy-stream semantics made SOME readbacks
    wait, hence the contradictory earlier runs.)
  - src0 identity: 144 stable pointers (MMID_PTR probe, cudaPointerGet-
    Attributes); sched selective-copy does NOT rewrite src0 (the
    "ffn_moe_gate-0" names are sched node names, not copies).
  - CUDA graphs (VITRIOL_NO_GRAPHS=1 kill-switch added), output cache
    (env-gated off), predictor prefetch (env-gated off), cross-device
    (single-GPU dies), capacity/evictions (4096MB pool still dies).

  CONCLUSION: expert weight delivery is PROVEN byte-perfect end-to-end,
  yet output is garbage for req2+ only. The corruption is NOT the expert
  weights — look at: (a) the ids/router path, (b) src1_sorted/dst_sorted
  scratch + pool_alloc reuse across graph rebuilds, (c) the OTHER nodes'
  behavior with a vitriol buffer present (supports_op guard? buffer
  usage flags? bad_padding_clear logic in ggml-cuda.cu:110 — MMQ padding
  NaN note in ggml-backend.cpp copy_experts), (d) a clean full-GPU
  control: NONE EXISTS (model only fits via store) — consider a
  small-quant model download for a true control.
  NOTE: "LRU Hits: 0" stats are UNRELIABLE (identical numbers across
  different workloads — counter/print bug, itself worth fixing).

  FIXES LANDED THIS SESSION (uncommitted):
  - REAL BUG 1 (alias): ensure/prefetch allocated slot = lru_map.size()
    which after ANY eviction aliases a new key onto a still-occupied
    slot → silent weight corruption. Fixed with g_lru_next_fresh_slot
    monotonic counter (both sites). Not the req2 trigger (predates req2
    pattern) but a genuine landmine — KEEP.
  - REAL BUG 2 (in-flight evict): victim-scan fallback blindly took
    lru_order.back() when every slot's last-read event was unfired →
    overwrite race with queued kernels. Fixed: cuStreamSynchronize
    (compute stream) then take LRU. KEEP.
  - Diagnostic envs added: VITRIOL_NO_GRAPHS, VITRIOL_NO_LRU,
    VITRIOL_LRU_READBACK, VITRIOL_LRU_TRACE[_BASE|_EXPERTS],
    VITRIOL sentinel (in trace), VHASH 3-way verdicts (SRC_BAD/VRAM_BAD,
    prints only violations), MMID_PTR probe, store_vs_gguf exhaustive.

  NEXT SESSION: start at (c) bad_padding_clear / MMQ padding — read
  ggml-cuda.cu:110 usage + ggml-backend.cpp copy_experts padding note;
  the LRU slice gives the kernel EXACTLY nb02 bytes with NO zeroed
  padding after it (upstream zero-fills padding for MMQ); check whether
  the per-expert mul_mat for slice tensors reads past nb02. Then (b)
  scratch pool reuse. Then (a) ids.
