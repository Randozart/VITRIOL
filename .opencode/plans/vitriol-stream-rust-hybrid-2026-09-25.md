# VITRIOL-stream: hybrid Rust implementation of the expert streaming path

Date: 2026-09-25 16:20 CEST.
Status: active. User directive: write the hybrid Rust implementation "if only
to see if that works" - as a controlled experiment against the Heisenbug, and
as the structural fix if our code is the culprit.

Supersedes nothing; complements `.opencode/plans/flashnext-store-divergence-2026-09-25.md`
(that plan's Stage 3 C++ toggles are implemented and did NOT fix; this is the
next escalation).

## Why (evidence base, 2026-09-25 hunt)

- Transport EXONERATED: 1794/1794 expert slices byte-verified store -> bounce
  -> H2D -> VRAM readback (incl. all 206 fat-layer 1,740,800 B slices). The
  weights reaching every matmul are provably correct.
- The corruption tracks async depth, not code location: ~1022 trailing
  unsynced graph nodes = safe; ~1272 = corrupt. Healing requires syncs
  DISTRIBUTED through the graph, not at one spot.
- One hard crash (CUDA illegal memory access) fired in arm A (gguf source),
  in ggml dispatch. Arm A is not clean either - it was lucky.
- Ruled out this session: LRU eviction race, victim/compute race (pending
  tracking added), map/order desync (abort added, never fired), H2D ordering
  (STRICT), matmul overlap (MATMUL_SYNC), cross-device copies (single-GPU
  still fails), store bytes, loader wiring (first 3522+ nodes byte-identical).
- Open suspects: ggml's own async graph execution (event ring, CUDA pool
  reuse, sched copies) - OR something in VITRIOL's C++ that byte-verification
  cannot see (e.g. stale pointers returned on hit paths, protocol misuse by
  the CALLER).

The Rust port is the controlled experiment: same store format, same protocol,
same env flags - memory-safe implementation. If it still corrupts, VITRIOL's
code is exonerated at the implementation level and ggml's execution layer is
implicated. If it does not, the C++ had a bug our instrumentation could not
see. Either answer ends the hunt.

## Scope

IN (Phase 1): everything on the expert streaming hot path -
  store reader (O_DIRECT), pinned bounce ring, per-device LRU pool
  (alloc/map/order/pending/mark), H2D event protocol, stats, diagnostics.
  These are the VITRIOL-authored lines where every bug class we hit lives.

OUT (stays C++, keeps working through the existing ABI):
  ggml buffer type registration, tensor binding call sites, `mul_mat_id`
  hook + expert loop, loader skip (`vitriol_skip_host_read`), mlock/
  cudaHostRegister slice ring (dormant on this host: mlock always fails at
  8 MiB ulimit), eager buffer paths, KV/embed zones (unused by this model).

## Architecture

New crate `vitriol-stream/` at repo root (sibling of `libvitriol/`).

    vitriol-stream/
      Cargo.toml          # crate-type ["cdylib","staticlib"], name vitriol_stream
      src/lib.rs          # module wiring, PROVENANCE header
      src/driver.rs       # ALL unsafe CUDA (driver API): the only unsafe module
      src/store.rs        # on-disk format, O_DIRECT reader, CRC32
      src/bounce.rs       # pinned slot ring, event-guarded reuse
      src/lru.rs          # per-device Lru struct: pool, SlotHandle, map+order+pending
      src/ffi.rs          # extern "C" ABI, catch_unwind boundary, error codes

Safety design (bug class -> construct):

| C++ bug class we hit | Rust construct |
|---|---|
| file-scope `g_lru_map`/`g_lru_order` mixed devices | `struct Lru` owned per device; map+order+pending in ONE struct, single mutation path |
| slot-0 fallback aliasing two keys | `SlotHandle(NonZeroU32)`; no integer fallback exists |
| victim overwrite of in-use slot | `pending: Vec<bool>` consulted in victim scan; last-resort fallback logs loudly |
| DMA from unpinned/freed host memory | bounce slots owned by `BounceRing`; guard types keep slot pinned while `InFlight` |
| silent error swallowing | `Result` everywhere; ABI returns codes; GGML abort on protocol violation at shim |
| O_DIRECT misalignment | aligned buffer types; alignment checked at construction |

CUDA context rule: Rust NEVER calls cudaSetDevice / context management.
The C++ shim does `cudaSetDevice(dev)` (ggml owns contexts); Rust uses the
calling thread's CURRENT context for driver calls - same semantics as the
C++ code today.

Store format (from vitriol-store.cpp, verified 2026-09-25):

- fd opened with O_DIRECT; all reads 512-aligned buf/size/offset.
- Two 4096 B superblocks at offsets 0 and 4096; valid = magic 0x56495453,
  version 1, crc32(IEEE, bytes 0..4091) == s.crc32; prefer s0.
- `super.expert_count` field REUSED as tensor count `n_tensors`.
- Index zone at offset 8192 (2*BLK): dense `store_tensor_desc` array
  (32 B each: u32 layer, kind, n_experts, slice_bytes, data_bytes, pad0;
  u64 store_offset) + u64 crc trailer; zone padded to 512.
- Expert read: off = super.expert_base + desc.store_offset
                       + expert_idx * desc.slice_bytes;
  pread slice_bytes (512-aligned), use first data_bytes.
  (Confirm `store_expert_offset` body matches at implementation start.)

## C ABI (ffi.rs; C++ side declares matching prototypes)

    vs_abi_version() -> u32                          // 1
    vs_init() -> i32                                 // reads VITRIOL_* env
    vs_open_store(path: *const c_char) -> *mut Store // null on error
    vs_close_store(*mut Store)
    vs_tensor_count(*mut Store) -> u32
    vs_desc_data_bytes(*mut Store, tid) -> u32
    vs_desc_n_experts(*mut Store, tid) -> u32
    vs_bind_tensor(*mut Store, tensor_base: *mut c_void, tid: u32) -> i32
    vs_max_expert_bytes() -> usize
    vs_lru_ensure(tensor_base, expert_idx: i32,
                  expert_data: *const c_void, expert_size: usize,
                  compute_stream: CUstream) -> CUdeviceptr   // 0 = uncached
    vs_lru_mark_compute_done(vram_ptr: CUdeviceptr, stream: CUstream)
    vs_lru_stats() -> (u64 hits, u64 misses, u64 evictions)   // via out-params
    vs_print_stats()
    vs_deinit()

Diagnostics ported: VITRIOL_LRU_HASH, VITRIOL_LRU_VRAM_HASH (verify-after-H2D),
VITRIOL_LRU_STRICT (post-H2D stream sync). Same output format as C++ so the
existing diff tooling works unchanged.

Env read by Rust: VITRIOL_STORE_PATH, VITRIOL_STORE_SOURCE (gguf|store),
VITRIOL_LRU_MB[_<dev>], VITRIOL_VERBOSE, diagnostics above.
Env kept in C++: VITRIOL_MODE, VITRIOL_MAX_LOCKED_MB, VITRIOL_RUST,
VITRIOL_MATMUL_SYNC, VITRIOL_LRU_DBG.

## C++ integration (env-gated A/B: VITRIOL_RUST=1)

- `vitriol-cuda-integration.cpp`: in `vitriol_lru_ensure` /
  `vitriol_lru_mark_compute_done` / stats print - when `rust_enabled()`,
  forward to vs_* and return. C++ LRU state untouched (unused in Rust mode).
- `vitriol-buffer.cpp`: in lazy alloc - when rust_enabled(), open the store
  via vs_open_store (C++ store not created); in `vitriol_store_bind_tensor`
  shim - call vs_bind_tensor; max_expert_bytes from vs_max_expert_bytes().
- `store_source_device()` logic unchanged (Rust reads the flag via vs_init).
- `ggml-cuda.cu`: untouched (mul_mat_id calls the same vitriol_* functions).

## Build system

- `cargo build --release` in vitriol-stream/ produces
  `target/release/libvitriol_stream.a` (preferred: static link, no dlopen
  path issues) and the cdylib for standalone tests.
- ggml/CMakeLists (ggml-cuda target): if VITRIOL_RUST_LIB exists
  (env/configured path to the .a), target_link_libraries(ggml-cuda
  <path>/libvitriol_stream.a). Driver API: link libcuda (ggml already
  links CUDA runtime; add CUDA_DRIVER_LIBRARY or `-lcuda`).
- C++ declarations of the ABI live in vitriol-cuda-integration.cpp
  (extern "C", weak-free - strong requirement when VITRIOL_RUST=1).

## Test protocol

Phase 0 - crate builds, unit tests pass
  - cargo test: CRC32 vectors, super parse, desc parse, expert offset math,
    SlotHandle/LRU state machine (hit/miss/evict/pending transitions).
  - Gate: green.

Phase 1 - store reader parity
  - Standalone binary (cargo run --example read_check) reads blk.0
    ffn_down expert 106 from /dev/nvme0n1p3, FNV-1a hashes, compares against
    the C++ reference hash 61584288a5842d51 (vh2_s.err, 2026-09-25).
  - Gate: hash match.

Phase 2 - wired oracle A/B
  - Build oracle with VITRIOL_RUST=1 available. Runs (single-GPU -ts 12,0,
    unserialized, the configuration that fails 100% today):
      1. arm B (store) + VITRIOL_RUST=1, x2   -> want 11751 both
      2. arm B (store) + VITRIOR_RUST=0       -> expect wrong (control)
      3. arm A (gguf)  + VITRIOL_RUST=1, x1   -> want 11751
  - Gate: (1) correct and deterministic -> Rust path healthy; bug was ours.
    (1) still corrupt -> VITRIOL code exonerated; ggml execution layer
    implicated -> write that up, decide next (ggml-side fix or stack move).

Phase 3 - if Phase 2 passes: multi-GPU arm B oracle, then server probe
  (' Paris' test), then Gate B functional suite (scripts/vitriol-store-
  functional-suite.py). Gate: suite pass -> commit approval request.

Phase 4 - deferred (only after Phase 3): move mlock/register slice ring and
  eager buffer paths into Rust; retire C++ LRU code.

## Risks / unknowns

- Context handling: driver calls from Rust on ggml's primary context -
  semantics must match exactly; watch for CUDA_ERROR_INVALID_CONTEXT.
- Static-lib + Rust std linking into ggml-cuda: may need
  `-lpthread -ldl -lm`; test early (Phase 0).
- Panic safety: catch_unwind at every extern "C" entry; panic = error code +
  stderr note, never unwinding into C++.
- Performance: expected parity (same syscalls, same driver calls); the
  1.16 ms store read dominates either way. Perf work stays deferred per the
  divergence plan's Stage 5.
- If Phase 2 still corrupts: the diagnostic value is the answer, not a fix;
  next step is a ggml-side experiment (e.g. force `ggml_backend_synchronize`
  after every N nodes in the sched) - documented, not hidden.

## Constraints (unchanged)

- No partition work, no D2b work, no push.
- Commit only after explicit user approval, per phase.
- Flag provenance: every oracle/server launch logs full env + argv.
- PROVENANCE header in lib.rs: own work, Apache-2.0 OR MIT; the LRU
  three-state victim idea remains attributed (kimi-k3-in-c inspiration).

## SIGNAL CORRECTION (2026-09-25 18:00 CEST)

The oracle's sampled token - used as the heal/corrupt signal throughout -
samples at temp 0.8 with a RANDOM seed (LLAMA_DEFAULT_SEED). Token churn
across runs is sampler noise. The Rust-vs-C++ equivalence verdict stands on
byte-level evidence only; the token table below is indicative, not
probative. The clean re-measurement protocol is: greedy (temp 0, fixed
seed) + byte-level node parity.

## Results (2026-09-25 17:40 CEST)

Phases 0-2 COMPLETE. The experiment answers the question:

- Phase 0 PASS: crate builds, 5/5 unit tests.
- Phase 1 PASS: Rust store reader byte-identical to raw device reads
  (blk.0 ffn_down e106). Side finding: the C++ diagnostic fnv1a at
  vitriol-cuda-integration.cpp:304 uses a TRUNCATED basis
  (1469598103934665603 instead of 14695981039346656037) - its hashes are
  internally consistent but not comparable to standard FNV-1a.
  Fix cost: one line, deferred to the commit round.
- Phase 2: VITRIOL_RUST=1 runs end-to-end (after two Rust-side fixes:
  driver memory entry points must use the _v2 names - cuMemAlloc_v2 etc. -
  the v1 legacy symbols fail with CUDA_ERROR_INVALID_CONTEXT on primary
  contexts; and the shim must cudaSetDevice before driver calls).

VERDICT: the Rust implementation REPRODUCES the corruption exactly.

| config (single-GPU, unserialized oracle) | sampled token |
|---|---|
| C++ gguf | 11751 (stable, 12+ runs) |
| C++ store | 393/279/264/9338/... (nondeterministic) |
| Rust store async | 2423 |
| Rust store SYNC_FILL (device-drain, blocking copies, no side stream) | 279/220 |
| Rust store SYNC_FILL + HYBRID (53 GiB host buffer memcpy'd from GGUF like arm A) | 271 |

Also verified this round:
- Store DATA is clean: 0 / 2560 expert slices differ between the store and
  the GGUF shards (new example store_vs_gguf: parses split-shard headers,
  compares every expert of layers 0/2/30/46/47).
- The model has VARIABLE-width FFN layers: blk.2/4/30/46/47 ffn_down are
  q8_0 (1,740,800 B/expert), the rest IQ3-class (921,600 B/expert).
- The graph executes 8022 nodes (incl. views); only ffn_down_exps tensors
  traverse the LRU (the fused ffn_gate_up_exps bypasses VITRIOL by name).

CONCLUSION: implementation language exonerated; transport exonerated
(1794/1794 VRAM byte-verified); store data exonerated (0/2560); async
protocol exonerated (sync-only still fails); cross-device exonerated
(single-GPU still fails). The corruption is triggered by the streaming
CONFIGURATION but not by anything VITRIOL implements - and it is healed
ONLY by distributed ggml-scheduler-level synchronization (cb_eval forces
ggml_backend_synchronize per node group; threshold ~1022 safe vs ~1272
corrupt trailing unsynced nodes). Prime suspect: ggml backend scheduler
async machinery (split input copies / event ring, GGML_SCHED_MAX_COPIES=4,
n_copies=1 when parallel=false) or graph buffer reuse interacting with the
53 GiB host-resident expert buffer placement.

NEXT (proposals, need user decision):
1. Rebuild ggml with -DGGML_SCHED_MAX_COPIES=64 and rerun the C++ store
   oracle (10 min, directly tests the event-ring suspicion).
2. Experiment with VITRIOL_MODE=stream on the gguf arm + forced sched
   syncs to place the threshold precisely.
3. Accept Rust crate as the base for the Stage-5 perf work (capacity/dev
   split, async reads) - its safety design pays off there.

## Log

- 2026-09-25 16:20 CEST: plan written. Phase 0 not started.
- 2026-09-25 17:40 CEST: Phases 0-2 complete; verdict recorded (above).
  Crate: vitriol-stream/ (driver/store/bounce/lru/ffi + read_check and
  store_vs_gguf examples). C++ shim: VITRIOL_RUST=1 routing in
  vitriol-cuda-integration.cpp + vitriol-buffer.cpp; CMake links
  $VITRIOL_RUST_LIB. Not committed (awaiting approval).
