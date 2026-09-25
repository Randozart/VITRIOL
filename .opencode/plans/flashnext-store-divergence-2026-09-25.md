# Flash-Next Store Divergence Localization Plan

Date: 2026-09-25 01:10 CEST.
Status: active.
Scope: find and fix why `VITRIOL_STORE_SOURCE=store` produces degenerate
generation while `VITRIOL_STORE_SOURCE=gguf` is coherent, on the same binary,
model, argv, and sampling.

Supersedes the "next diagnostic should dump tolerance-aware activation
statistics" note in `.opencode/plans/flashnext-store-functional-2026-09-24.md`
(that note is retained as history).

## Constraint recap

- No partition work, no D2b work, no push.
- Commit only after explicit user approval, per stage.
- Flag provenance mandatory on every launch.
- Depth ladder and perf gates remain blocked until functional gates pass.

## Evidence base

### Arms differ in exactly three ways

Both arms run with `VITRIOL_MODE=stream`, `VITRIOL_MAX_LOCKED_MB=8192`,
`VITRIOL_STORE=1`, `VITRIOL_STORE_PATH=/dev/nvme0n1p3`, `VITRIOL_VERBOSE=1`.
Verbose output confirms `Output Cache: none`, `Expert Pinning: none`,
`Predictor: none`, so only:

| # | arm A `source=gguf` | arm B `source=store` | site |
|---|---|---|---|
| 1 | GGUF `read_raw` fills host expert bytes | skipped, host stays zero | `src/llama-model-loader.cpp:1700` gated by `vitriol_skip_host_read` at `vitriol-cuda-integration.cpp:571-579` |
| 2 | `mlock` + `cudaHostRegister` per slice | skipped | `ggml-cuda.cu:2224` guard |
| 3 | `vitriol_lru_ensure` H2D from `expert_data` (host) | H2D from `bounce->aligned` | `vitriol-cuda-integration.cpp:1386` |

Sort path, LRU, event protocol, device placement are shared.

### Ruled out by tensor inventory

Parsed all three GGUF shards (1224 tensors total):

- exactly **144** names contain `ffn_{up,gate,down}_exps`, all `.weight`,
  48 layers x 3 kinds. No `.scale`, `.bias`, or `shexp` name collides.
- `store_kind_from_name` (`vitriol-cuda-integration.cpp:443-449`) and mkfs
  `expert_kind` (`vitriol-store-mkfs.cpp:55-61`) use identical ordering, so no
  kind swap between index and lookup.
- 192 `shexp` tensors and 36 `ssm_dt.bias` tensors are excluded from both
  matchers in the worktree (the `shexp` branch was removed).
- `g_store_binding_failed` (`vitriol-cuda-integration.cpp:440`) is written at
  `:575` and never read — dead flag, no telemetry for a failed bind.

### Findings from `/tmp/opencode/functional_store.out`

- Exactly one `lazy buffer` line (53247 MiB) and one `LRU pool` line
  (`GPU 0`). Span arithmetic over the 144 registered base pointers confirms
  all tensors live in one mapping.
- `vitriol_cuda_get_extra_bufts` (`vitriol-buffer.cpp:503-520`) ignores `dev`
  and returns one static list for every device → all expert weights land in
  device-0's buffer → **every `MUL_MAT_ID` executes on GPU 0**. Not
  arm-asymmetric, so not the correctness bug, but it explains the ~4x
  slowdown.
- `Store Read ms: 63474` over 54,529 fills = **1.16 ms per Optane read**,
  synchronous inside the compute path.
- LRU: 1233 slots vs ~1152 entries touched per token → 53,296 evictions;
  the working set sits at the pool edge.

### Latent bugs in `vitriol_lru_ensure` (`vitriol-cuda-integration.cpp:1289-1405`)

1. `g_lru_map` / `g_lru_order` are file-scope globals (`:83-85`) while pools,
   `g_lru_slot_size`, `g_lru_num_slots` are per-device arrays (`:61-63`).
   `slot = (int)g_lru_map.size()` (`:1347`) and victim selection over
   `g_lru_order` (`:1351-1357`) mix devices as soon as a GPU-1 pool exists.
   Currently dormant.
2. `slot = (eit != g_lru_map.end()) ? eit->second : 0;` (`:1357`) silently
   falls back to slot 0 on order/map desync → two keys alias one slot →
   silent weight corruption. Must abort instead.
3. Victim selection treats `cuEventQuery(slot_event) == CUDA_SUCCESS` (`:1354`)
   as "slot is safe to overwrite", but the same event object is also the H2D
   completion record. Window: after a cache hit enqueues
   `cuStreamWaitEvent(cstream, ev)` (`:1330`) and before
   `vitriol_lru_mark_compute_done` re-records it (`ggml-cuda.cu:2250`), the
   event still reads SUCCESS for an H2D → eviction can overwrite a slot while
   a consumer matmul is pending.

### Evidence-quality caveat

The `src1hash` divergence claim cannot be re-verified: the temporary FFN
instrumentation was removed and never committed (`git log -S src1hash` is
empty). Treat it as a hypothesis, not a fact, until Stage 1 reproduces it.

## Stage 0 - Controls, zero code

1. Launch arm A with `VITRIOL_STORE_SOURCE=gguf`, identical argv and env, on
   an isolated port. Run `scripts/vitriol-store-functional-suite.py` against
   it. This control was never run; without it "store is broken" is unproven.
2. One deterministic single completion per arm on a chosen probe prompt;
   confirm divergence still reproduces on the current post-cleanup binary.
   That prompt becomes the Stage 1 oracle input.
3. Store arm single completion under `CUDA_LAUNCH_BLOCKING=1` — free race
   probe, no rebuild.

Gate: arm A suite result recorded; divergence reproduced on current binary.

## Stage 1 - Definitive localization

4. `cmake --build build --target llama-vitriol-oracle` (binary is stale,
   2026-09-08).
5. Run both arms with the Stage 0 probe prompt, identical env,
   `ORACLE_OUT=/tmp/opencode/oracle_{gguf,store}`. Apply `setcap` to the
   oracle binary too if `VITRIOL_MAX_LOCKED_MB=8192` is kept.
6. `python3 tools/vitriol-oracle/diff.py A.idx A.bin B.idx B.bin` → record the
   first divergent node in graph order (name, type, shape, cos, maxabs).
7. Run arm A twice and diff against itself to establish the determinism noise
   floor, so non-determinism is not mistaken for divergence.

Known caveats: `capture_cb` skips views, so the `reshape_3d` src1 node itself
will not appear (its producer `hc_mixed-<il>` will); `cb_eval` inserts a
synchronous D2H per node and can mask timing-dependent defects.

Gate: a named first-divergent node, or an explicit "parity under oracle"
result.

## Stage 2 - Interpret and target

- **Divergence before the first `MUL_MAT_ID`** → loader/buffer issue. D2H-diff
  non-expert tensors between arms at load: `token_embd`, attention weights,
  `hc_ffn_*`, `hc_inject_*`.
- **Divergence at or after the first `MUL_MAT_ID`** → expert path. Hash the
  LRU slot *immediately before* `ggml_cuda_mul_mat` (compute-time, not
  post-hoc), gated by `VITRIOL_LRU_DBG`, and compare the same
  `(tensor, expert)` across arms.
- **Parity under oracle** → do not conclude "no bug"; `cb_eval` serializes.
  Go to Stage 3.

Gate: narrowed to one of loader / expert data / timing.

## Stage 3 - Race isolation (only if Stage 1 shows parity)

8. Add `VITRIOL_LRU_STRICT=1` with three independent toggles, enabled one at
   a time:
   - (a) `cuStreamSynchronize` on the LRU stream after H2D;
   - (b) `cuStreamSynchronize` on the compute stream after each expert matmul;
   - (c) always evict true LRU, never skip via `cuEventQuery`.
   Whichever toggle restores parity names the defect.
9. Fix the three latent bugs regardless (defensive, low risk):
   - abort on a missing `evict` key instead of slot 0;
   - make `g_lru_map` / `g_lru_order` per-device with a per-device free list,
     so slot assignment never reads a cross-device `map.size()`;
   - track `compute_pending[slot]`, set when `vitriol_lru_ensure` returns a
     cache hit, cleared in `vitriol_lru_mark_compute_done`, and excluded from
     victim selection.

Gate: store arm produces coherent output on the probe prompt.

## Stage 4 - Re-gate

10. Rebuild, then reapply `setcap cap_ipc_lock=+ep` on `build/bin/llama-server`
    and `build/bin/libllama-server-impl.so` (a rebuild wipes capabilities).
11. Rerun Gate A counters and the Gate B suite for 3 rounds; append full argv,
    env fingerprint, counters, and outputs to
    `.opencode/plans/flashnext-store-functional-2026-09-24.md`.
12. Request commit approval for the stage(s) that pass.
13. Afterwards: fix the missing model path, re-enable and restart
    `vitriol-server.service`.

Gate: Gate A + Gate B pass; daily driver restored.

## Deferred -> Stage 5 - Post-gate performance (folded 2026-09-25 14:10 CEST)

Correctness gates first. Then, ranked by expected value:

1. **LRU capacity + device split.** Working set is 1440 slices/token
   (48 layers x 10 experts x 3 kinds, ~1.0 GiB). Pool is 2048 MB = 1233
   slots < working set -> guaranteed thrash every token (31,540 evictions
   in one suite run). Fix `vitriol_cuda_get_extra_bufts` ignoring `dev`
   (all MoE forced onto GPU 0 today, ~4x loss) and split the pool across
   both cards (~6.3 + 8.7 GiB free -> ~6-8k slots >> 1440). Eviction -> 0;
   streaming becomes first-touch-only. No policy needed.
2. **Async / batched store reads.** 1.16 ms per ~0.9 MB read is
   latency-dominated (~0.8 GB/s vs multi-GB/s Optane). Issue the
   up/gate/down reads of one expert in parallel or batch adjacent slices;
   overlap with compute.
3. **Predictor prefetch.** `Predictor: none` today; plumbing exists
   (`vitriol_predictor_prefetch`). MoE routing has strong temporal
   locality -> pre-stream next-step experts during current compute.

Rejected: size-based prioritization ("keep large experts local"). Expert
slices are uniform per kind (down 921,600 B, up/gate 627,200 B; IQ blocks
are uniform), and cache economics cancel size anyway (miss cost and hit
benefit both scale with slice bytes; frequency is the only signal).
Rejected: kind-based residency (token needs up+gate+down together).
Rejected: whole-layer pinning (~1.1 GB/layer for ~6% of streams).

## Log

- 2026-09-25 01:10 CEST: plan written after read-only investigation. Stage 0
  not yet started.
- 2026-09-25 11:15-14:10 CEST: Stage 0/1 executed.
  - Arm A control FAILED 8/8 on first run -> later shown to be a memory
    artifact: arm A writes the full 53 GiB expert buffer, box has 31 GiB
    RAM; RssAnon 12.7 MB / VmSwap 55 GB, 0.15 tok/s. Not a correctness
    regression.
  - Fresh healthy-memory A/B: arm A " Paris...", arm B degenerate loop.
    Defect IS store-specific.
  - Oracle (serialized): 4886/4886 nodes byte-exact between arms, both
    sample 11751. Arm A self-noise floor: byte-exact. Defect is
    timing-dependent; cb_eval masking confirmed.
  - Unserialized oracle: gguf=11751 stable, store varies
    (393/279/264/9338...). Nondeterminism = corrupted compute.
  - Ruled out: LRU eviction race (3700-slot pool, Evictions: 0, server
    still wrong), CUDA_LAUNCH_BLOCKING=1 (no change), H2D completion
    ordering (new VITRIOL_LRU_STRICT=1 toggle: store still wrong), store
    read failures (0), loader/buffer wiring (first 3522+ nodes byte-exact).
  - Window-capture bisect (new ORACLE_IDX_LO/HI): brackets [5851,5900) and
    (6750,7000] were reported - **INVALIDATED 2026-09-25 18:00 CEST**: the
    bisect signal was the oracle's sampled token, which samples at temp 0.8
    with a RANDOM seed (LLAMA_DEFAULT_SEED -> std::random_device,
    llama-sampler.cpp:341). Token churn across runs is sampler noise, not
    corruption. All brackets from that session are retracted. The only
    sampling-independent divergences on record: (a) full-capture runs are
    byte-IDENTICAL between arms (4886/4886 nodes); (b) server greedy probes
    (temp 0): arm B " the capital of France is the capital of" 4/4 vs
    arm A " Paris..." n=1.
  - Leading hypothesis: cross-device (ggml sched) race exposed by store
    path timing, not caused by store bytes. VITRIOL_LRU_HASH A/B built to
    confirm source bytes identical (not yet run).
  - .clangd added (clang 22 lacks P2900): strips -fcontract*, defines
    pre() away for clangd only; verified clean parse via clangd --check.
