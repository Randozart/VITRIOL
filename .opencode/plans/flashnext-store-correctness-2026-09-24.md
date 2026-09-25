# Flash-Next Store Correctness and Validation Plan

Date: 2026-09-24 09:02 CEST. Status: in progress.

## Objective

Qualify `VITRIOL_STORE=1` as the cold expert-weight source for
Qwen3.8-Flash-Next-UD-Q3_K_XL on 2x RTX 3060. Correctness and deterministic
output equivalence must pass before any performance or depth claim.

## Current evidence

Source log: `/tmp/opencode/armB_store4.log`.

- Arm B loaded on `127.0.0.1:8302` and reported healthy.
- Lazy buffer was store-backed: 53,247 MiB pageable mmap.
- 144 expert tensor bases registered to store descriptors.
- 1,057 expert fills completed with zero `store fill ... FAIL` messages.
- Fill sizes were heterogeneous: 107 x 1,740,800 B, 948 x 921,600 B,
  2 x 627,200 B.
- No inference acceptance result exists yet. Current output is quarantined.

## Blocking defect

`vitriol-cuda-integration.cpp` uses a fixed 1,024-entry `g_locked_slices`
array. Insert uses unchecked post-increment at line 584. The 1,057-fill load
exceeded capacity and overwrote adjacent global state, including store
registry metadata. `armB_store4.log:2223` resolves a corrupted TID (`215`)
after a correct TID `294` fill. This is memory corruption, not a performance
issue.

Additional correctness risks:

- `store_read_expert` reads padded `slice_bytes`, while integration compares
  the result with `data_bytes`.
- Arbitrary tensor `nb02` is not proven suitable for O_DIRECT destination.
- Store handle and tensor bindings are global, not owned by their buffer.
- Loader skips GGUF reads even when exact store binding fails.
- Lazy allocator continues with an empty store-backed buffer if `store_open`
  fails.
- Eager store mode does not call the fill hook.
- Generic loader has a direct unresolved dependency on a CUDA-library symbol.
- Current host-registration eviction can retain unbounded pinned pages.

## Implementation sequence

### Stage 1 - Memory-safe bounded lock state

1. Replace fixed lock array with bounded dynamic ownership state.
2. Key entries by buffer owner, tensor base, expert index, and exact stride.
3. Enforce both byte budget and entry capacity before insertion.
4. Never post-increment or index before bounds validation.
5. Preserve `mlock` and CUDA registration lifetimes safely; no unregister while
   prior CUDA work can reference the range.
6. Release state on buffer destruction after device work is quiescent.

Gate: stress more than 1,024 unique slices; no overwrite, sanitizer issue, CUDA
error, or stale binding.

### Stage 2 - Buffer-owned store state

1. Keep each store handle in its `vitriol_buffer_context`.
2. Bind tensors to that buffer's store and descriptor.
3. Unregister all tensor bindings before closing the store.
4. Reject store-backed allocation if store open or descriptor validation fails.
5. Remove global assumptions about one store handle across devices/buffers.

Gate: two-buffer lifecycle test; no dangling handle or cross-buffer TID.

### Stage 3 - Exact O_DIRECT expert reads

1. Validate descriptor TID, expert index, `n_experts`, tensor `nb02`,
   `data_bytes`, and `slice_bytes` before skipping or reading.
2. Require `nb02 == data_bytes` for current packed expert layout.
3. Read padded slot into 512-byte-aligned RAII scratch storage.
4. Copy exactly `data_bytes` into the tensor destination.
5. Return/compare exact real bytes; count requested bytes, actual bytes,
   failures, and elapsed read time.
6. Validate async path separately or disable it until alignment-safe.

Gate: mixed geometry, non-512 `data_bytes`, invalid TID, invalid expert, and
unaligned destination tests pass.

### Stage 4 - Loader/backend boundary

1. Skip GGUF `read_raw` only after successful exact store binding.
2. On registration failure, return failure to model loading; do not leave
   tensor storage uninitialized.
3. Replace direct CUDA symbol dependency with a default-safe callback or
   registration API usable by CPU-only builds.
4. Invoke store fill whenever store-backed, independently of lazy locking.
5. Keep output cache disabled for equivalence and ladder gates. Fix its
   256-expert bound before enabling it.

Gate: CPU-only link test and CUDA loader smoke test pass.

### Stage 5 - Build and host tests

1. Build affected CUDA/server targets.
2. Build and run existing `vitriol-store-test` harness.
3. Extend existing harness; do not add an unrelated upstream test framework.
4. Run contract-enforce build for host VITRIOL translation units.
5. Run available lint/typecheck commands, or record that none are configured.

Gate: clean build, all store tests PASS, no new warnings.

### Stage 6 - Arm B shallow output gate

Shared launch geometry:

```text
-ngl 99 -ts 12,12 --main-gpu 0 -ub 64 -fa on
-c 512 --parallel 1 --slots -t 8
```

1. Kill stale `llama-server` processes before launch.
2. Rebuild `llama-server`, then reapply `cap_ipc_lock` to executable and
   `libllama-server-impl.so`.
3. Launch arm B with `VITRIOL_MODE=stream`, `VITRIOL_MAX_LOCKED_MB=8192`,
   `VITRIOL_STORE=1`, and `VITRIOL_STORE_PATH=/dev/nvme0n1p3`.
4. Wait for health and exact tensor bindings.
5. Run deterministic greedy 32-token completion.
6. Require coherent output, valid descriptor geometry, more than 1,024 safe
   fills, and zero store/read/CUDA failures.

STOP on mismatch, crash, wrong TID, silent fallback, or unbounded lock state.

### Stage 7 - Arm A versus arm B equivalence

1. Use same binary, model, launch argv, prompt, seed, and greedy sampling.
2. Change only store source environment between arms.
3. Capture completion text and token IDs.
4. Require exact token and text equality.
5. Embed full argv, env fingerprint, timestamps, counters, and outputs here.

STOP on any mismatch. No depth benchmark runs after failed equivalence.

### Stage 8 - Depth ladder

Only after Stage 7 passes. Test filled depths 13K, 26K, and 36K. For each arm:

- exact-fill via `/tokenize`;
- record prefill time and filled token count;
- run one warmup plus 3 x 64-token decode measurements;
- record predicted tokens/second, prompt tokens/second, VRAM on both GPUs,
  store bytes, block-device sectors, and observed PCIe traffic when available;
- watch host RSS, swap, `VmSwap`, lock bytes, and registration count.

Do not claim a depth unless filled token count is reported.

## Safety and repository rules

- Preserve unrelated uncommitted `ggml-cuda.cu` `VITRIOL_LRU_DBG` changes.
- Do not stage or modify unrelated untracked files.
- No force push, schema migration, partition change, or D2b work.
- Default mmap path remains unchanged and flag-gated store mode stays opt-in.
- Every launch and result must include full flag provenance.
- Commit only after explicit user approval for each stage.

## Deliverables

- This plan with anchored status after every gate.
- Store test output and exact commands.
- Arm A/B equivalence artifacts.
- Per-depth mmap/store comparison table and attribution.
- Known limitations and final PASS/FAIL verdict.

## Anchored status

- 2026-09-24 09:02 CEST: plan written. P0 lock-table overflow confirmed from
  1,057-fill arm B log. Benchmark gate closed. Implementation starts at Stage 1.
- 2026-09-24 22:45 CEST: direct-to-VRAM implemented and build passes.
  Standalone store test `ALL PASS`. Arm B shallow output gate remains FAIL:
  `[21134, 11]` versus arm A `[271, 26108]`. All copied expert bytes and VRAM
  readbacks are exact; first-layer routing matches. FFN input hashes differ,
  moving the remaining investigation before routed FFN weight use. Depth ladder
  remains blocked. Daily `vitriol-server.service` is stopped and disabled during
  testing.

## 2026-09-24 direct-to-VRAM implementation status

Implemented and tested a direct-to-VRAM store path:

- canonical `VITRIOL_STORE_SOURCE=gguf|store`;
- routed experts are bound exactly, but `source=store` leaves host bytes
  untouched;
- per-device 16-slot pinned bounce ring, 512-byte aligned;
- Optane O_DIRECT read into bounce, then bounce-to-VRAM H2D;
- bounce and LRU slots guarded by CUDA events;
- LRU slot sized to the largest bound expert (`1,740,800` bytes for
  Flash-Next);
- no per-slice host `mlock`, `cudaHostRegister`, or store-mode host writes.

Standalone store test result: `ALL PASS`.

Direct arm B launch used `VITRIOL_MAX_LOCKED_MB=8192`, `-ngl 99`,
`-ts 12,12`, `-c 512`, `-ub 64`, `-fa on`, `--no-warmup`, one slot. Model
bound 144 routed tensors. The first deterministic two-token request returned
`[21134, 11]` (`" Answer,"`) instead of arm A's `[271, 26108]`
(`"\n\nFour"`).

Diagnostics for this run:

- 8,043 direct fills, zero store failures;
- all 8,043 CPU bounce hashes matched GGUF byte-for-byte;
- VRAM D2H readback hashes matched bounce hashes;
- first-layer expert routing matched the GGUF arm exactly;
- therefore CPU storage, Optane read, bounce, H2D, and VRAM contents were
  correct;
- matched FFN probes showed that the activation tensor entering the first
  compared FFN call already differed between GGUF and store arms
  (`src1hash` differed), so divergence occurs before the routed FFN weights
  are used;
- temporary FFN instrumentation was removed after diagnosis.

Operational finding: after reboot, the daily `vitriol-server.service` restart
loop repeatedly killed experimental servers through its launcher. The unit was
stopped and disabled for this test cycle. It must be explicitly re-enabled or
its model path fixed before restoring the daily driver.

## 2026-09-24 pre-direct diagnostic addendum

Fresh-process probes narrowed the remaining defect to the store fill
architecture, not expert bytes.

- Arm A (GGUF) repeatedly emitted tokens `[271, 26108, 13, 248044]`.
- Store arm repeatedly diverged at token two, usually `[271, 248068, ...]`.
- The original fixed 1,024-entry lock table was replaced by bounded dynamic
  state. Arm B then completed 10,586 fills with zero failures and clean
  shutdown.
- The store file was compared against GGUF routed tensors and matched.
- CPU-side fill hashes matched store/GGUF bytes.
- H2D source hashes matched CPU fill hashes.
- VRAM D2H readback hashes matched H2D source hashes.
- For the valid matched control, every overlapping `(tid, expert)` runtime
  hash matched. All 885 store-arm-only used slices were independently
  verified byte-for-byte against GGUF.
- Changing routing/kind masks, one layer, LRU size, direct O_DIRECT versus
  scratch, and pre-fill CUDA synchronization did not restore equivalence.
- GGUF-loaded-then-refilled store mode still diverged, so rewriting mutable
  host expert slices is the remaining architectural hazard.

Decision: use direct-to-VRAM for store mode. The loader will bind routed
tensors but leave their host bytes untouched. On an LRU miss, VITRIOL will
O_DIRECT-read into a 512-aligned pinned bounce ring slot, copy that slot into
the VRAM LRU slot, and guard both bounces and VRAM slots with events. Store
mode will not call per-slice `mlock`, `cudaHostRegister`, or write into the
anonymous expert tensor. LRU slots will be sized to the largest bound expert
so Flash-Next's 1,740,800-byte `ffn_down` tensors cannot bypass the cache and
read uninitialized host memory.
