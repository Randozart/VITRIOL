# cuda-oxide spike crate — provenance

Date: 2026-10-10.

## What

`vitriol-gpu-rs/` — spike crate validating the NVIDIA cuda-rust /
cuda-oxide toolchain (pure-Rust SIMT kernels compiled to PTX by the
`cargo-oxide` rustc backend) on this host, plus a probe-shaped
batched dot-product kernel with CPU parity, bench, and
compute-sanitizer runs.

## Kind

`inspiration` — NVlabs/cuda-oxide (Apache-2.0), vendored at pinned commit
`bafffe4` under `vitriol-gpu-rs/vendor/` (gitignored; reclone +
`cargo oxide run` rebuilds the backend from the pinned rev). What was
learned, not copied:

- single-source host+device authoring (`#[cuda_module]` / `#[kernel]`,
  `DisjointSlice`, `thread` intrinsics, typed launches with closure
  capture)
- the compilation pipeline (Rust -> MIR -> Pliron IR -> LLVM IR -> PTX ->
  driver JIT) and the `cargo oxide build/run/sanitize` workflow
- toolchain pinning discipline (nightly-2026-08-28 + rustc-dev)

The two kernels (`scale`, `probe_dot`), the parity/bench harness, and all
host code are VITRIOL-original. Verdict + gotchas:
`.opencode/plans/cuda-oxide-spike-verdict-2026-10-10.md`.

## License position

Vendored dependency stays Apache-2.0 (upstream). VITRIOL crate carries
`Apache-2.0 OR MIT`. No upstream code was copied into VITRIOL files.
