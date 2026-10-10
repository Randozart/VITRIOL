# cuda-oxide spike: adopt/reject decision

Provenance record: `docs/provenance/cuda-oxide-spike.md`.

- **Date:** 2026-10-10 21:xx
- **Verdict:** TOOLCHAIN ADOPTED for experiments; NO ENGINE KERNELS yet.
- Crate: `vitriol-gpu-rs/` (Apache-2.0 OR MIT, PROVENANCE headers).

## What was proven (RTX 3060 dev1, sm_86, driver 615.71, CUDA toolkit 13.4)

1. Full pipeline: Rust -> MIR -> Pliron IR -> LLVM IR -> PTX (`vitriol_gpu_rs.ptx`,
   `.target sm_86`) -> driver JIT -> execution. Nightly-2026-08-28 toolchain,
   rustc-codegen-cuda backend built from the pinned vendor commit (bafffe4).
2. `scale` hello kernel: parity 1 Mi elems; launch+sync overhead ~4-5 us per
   tiny launch (100 launches 396-2414 us depending on host load).
3. `probe_dot` kernel (q[H,D] . k[N,H,D] -> [N,H], the lull probe-scoring
   shape): parity exact vs CPU at N=16384/H=8/D=64; naive in-thread reduction
   measures 4-8.5 GMAC/s (host VRAM contention inflates variance; the kernel
   is intentionally naive - no smem tiling, no vector loads).
4. compute-sanitizer memcheck: 0 errors on both bins.

## Gotchas hit (documented for future use)

- `load_kernel_module` expects the CRATE name with underscores
  (`vitriol_gpu_rs`), not the package name with hyphens.
- Generated typed-module type is `kernels::LoadedModule` (not `Kernels`).
- `ctx.default_stream()` returns `Arc<CudaStream>`; typed launches take
  `&CudaStream` directly (no `.as_ref()`).
- Artifact emission lands in the crate root only when the codegen backend
  path actually runs (watch for the RUSTC-CODEGEN-CUDA banner; standalone
  projects build the backend from the pinned commit on first run).
- `default-run` manifest key needed once a second bin exists.
- The `[workspace] exclude = ["vendor"]` line is load-bearing: without it the
  outer workspace claims the vendored nested workspaces and breaks
  `edition.workspace` inheritance.

## Adopt/reject

- ADOPT as the experimental GPU-kernel toolchain (capability proven end to
  end, isolated from the engine; nvcc/ggml build untouched).
- REJECT immediate engine integration: VITRIOL has zero custom CUDA kernels
  because ggml already covers the ops we use (probe scoring runs on GPU via
  ggml graph ops). A hand kernel must beat ggml's tuned CUDA ops to earn a
  place; the naive 8.5 GMAC/s spike is ~3 orders below that bar and was not
  optimized. Revisit only if an op ggml lacks becomes load-bearing (e.g. a
  fused probe-score+evict-select pass that today round-trips scores to host).
- `cutile-rs` (needs CUDA 13.2+, sm_80+) remains available in the same vendor
  tree if tile-level DSL work ever becomes relevant.

## Reproduce

    cd vitriol-gpu-rs
    CUDA_VISIBLE_DEVICES=1 cargo oxide run              # hello scale
    CUDA_VISIBLE_DEVICES=1 cargo oxide run --bin probe_dot
    CUDA_VISIBLE_DEVICES=1 cargo oxide sanitize --bin probe_dot --tool memcheck
