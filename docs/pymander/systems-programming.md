# Systems Programming Doctrine

Hand-authored atomic knowledge for the systems-programming domain. Each `##`
section is one atomic node; the pre-node prose is domain header and skipped.

## Memory Safety Rules

Rust borrow checker guarantees memory safety at compile time; no data races
in safe code. Interior mutability (Cell/RefCell/Mutex) trades compile-time
guarantees for runtime checks. Unsafe code must document its invariants.

## Zero-Copy I/O

Prefers buffers owned by the driver and passed by reference; avoids memcpy in
the hot path. mlock/cudaHostRegister pins host memory so DMA can read it
directly — see VITRIOL's expert-offload path.

## Ternary Weights

Executing ternary {-1, 0, +1} weights by lookup table instead of multiply:
decode a packed 2-bit block into signs, look up the precomputed product with
the activation, accumulate. No FP multiply per weight in the hot loop.

## Context Sliding Window

When the KV cache overflows, shift the window (--context-shift): keep the
recent turns, drop the oldest, and reuse the prefix via --cache-reuse so the
pinned prefix stays cached. Injected context must survive the shift.
