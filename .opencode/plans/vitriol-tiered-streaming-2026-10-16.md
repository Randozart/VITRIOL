# Tiered expert streaming: VRAM / pinned-RAM arena / Optane (Rust-first)

**Created**: 2026-10-16 21:20
**Status**: approved (user: "pragmatic, 10 t/s should be doable with a tight
configuration", new code in Rust)
**Supersedes**: the remap-based approach in `dma-stream-revival-2026-10-16.md`
(Addendum C verdict: remap corrupts on Nemotron, both arms, both languages).
That doc's wiring fixes (RC1-RC4) remain landed and load-bearing.

## Design contract (user)

Expert weights are immutable -> three read-only cache tiers, no coherence:

| tier | medium | capacity (start) | role |
|---|---|---|---|
| VRAM | LRU pool per GPU | auto (free - reserve), ~3-5 GiB/dev | per-token reads, D2D |
| pinned RAM arena | `cuMemHostAlloc`, slice-granular LRU | ~12-16 GiB | CE H2D per miss, registered -> direct DMA, no staging |
| Optane | raw-zone store (38 GiB, mkfs'd) | cold set | **async promotions into arena** (predictor/miss-driven), never critical path |

After warmup the working set stabilizes in VRAM+RAM (~65-70% of bytes,
>=90% of accesses under expert skew); Optane traffic = background
replenishment. That is what makes 10 t/s arithmetic close:
1 GB/token at 10 t/s = 10 GB/s sustained - only pinned/VRAM delivers it.

## Architecture

- Weight mover = the upstream ggml sched MoE branch
  (ggml-backend.cpp:1690-1760): gathers only routed experts into a device
  `input_cpy`. Proven correct on this box (the pre-wrap 1.3 t/s runs).
- Transport = vitriol Copy Engine channel (vitriol_copy_engine.cpp: dedicated
  CE stream + pinned mapped bounce + fence, cuMemcpyDtoDAsync path).
- Serve hook = inside ggml-cuda.cu `set_tensor_async`/`set_tensor`: if the
  source pointer falls inside a registered vitriol expert region, delegate to
  Rust: VRAM hit -> D2D from slot; arena hit -> CE H2D from registered slice;
  miss -> store fill -> arena -> H2D (first touch sync, amortized by
  predictor promotions).
- The in-op pointer remap (ggml_cuda_mul_mat_id route block + needs_sync
  override) is DISABLED by default (`VITRIOL_REMAP=1` re-enters, loudly
  guarded). Without it CUDA graphs are legal again for these ops.
- Wrap procs stay: they are what places experts host-side under mmap=on,
  which is the sched branch's precondition. Zero-copy, RSS small.

## Language split (user: prefer Rust for new code)

- NEW: `vitriol-stream` crate gains `arena.rs` (pinned slice LRU) + a
  `vs_serve_set` FFI entry (registry lookup -> VRAM/arena/store decision ->
  CE transport) + predictor promotions. All policy/transport in Rust.
- C++ shrinks to: the ~5-line hook in ggml-cuda.cu, env-gating/removal of the
  remap block, and binding registration calls (existing vs_* ABI).
- Existing C++ kept: wrap procs, store reader (mkfs format), engagement guard.

## Performance ladder (each rung oracle-gated, independently shippable)

| rung | content | target t/s | state |
|---|---|---|---|
| L0 | CPU-MoE (mode=off) | 0.75 | exists, correct |
| L1 | sched-fed stream, remap off, graphs on | ~1.3 (measure) | this session |
| L2 | graphs verification | measure | this session |
| L3 | arena + CE serve hook | 4-6 | next session |
| L4 | VRAM LRU serve + predictor promotions, budgets from measured hit rates | 8-12 | then |
| L5 | hot-first placement, budget tuning | 10+ | knob work |

## Correctness gates (every rung, non-negotiable)

17x23=391 oracle; greedy determinism n=3 (fixed prompt, temp 0); output
coherence; then 3x256 bench; 30-min soak before any "working" verdict.
Engagement guard stays; failures loud.

## Kill criteria (pragmatism)

- S2 arena+CE fails oracle after a day of honest debug -> hold L1/L2
  (working machine), post-mortem arena before retry.
- L4 hit rate <50% with tuned budgets -> report ceiling, hold best rung.
- Box never without a correct Nemotron: L0 profile always one
  `vitriol config load` away.

## Stages

- S0 (30 min): GGML_SCHED_DEBUG=2 capture on a corrupting (remap-on) decode:
  split layout, which expert ops sched-fed, input_cpy size reality.
- S1 (1 h): remap env-gated off + needs_sync gate; profile
  `nemotron35-super-vl-stream` (mode=stream, mmap=on, ncmoe=88, no store);
  L1/L2 protocol; commit.
- S2 (session): arena.rs + CE serve hook; oracle gate; >=3 t/s target; commit.
- S3 (session): VRAM LRU serve + predictor + budgets from hit rates; soak;
  commit.
- S4: docs (Addendum D), optional Rust port of remaining C++ paths.

## Provenance

No new third-party code. Fork-owned files, Apache-2.0 OR MIT. Rust crate
extends existing vitriol-stream (same license, PROVENANCE headers kept).
