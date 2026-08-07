# Systems Programming Doctrine

Hand-authored atomic knowledge for the systems-programming domain (VITRIOL's
own domain: machine-native LLM inference). Each `##` section is one atomic
node; the pre-node prose is domain header and skipped.

## Memory Safety Rules

Rust borrow checker guarantees memory safety at compile time; no data races
in safe code. Interior mutability (Cell/RefCell/Mutex) trades compile-time
guarantees for runtime checks. Unsafe code must document its invariants.

## Zero-Copy I/O

Prefers buffers owned by the driver and passed by reference; avoids memcpy in
the hot path. mlock / cudaHostRegister pins host memory so DMA can read it
directly — see VITRIOL's expert-offload path.

## Ternary Weights

Executing ternary {-1, 0, +1} weights by lookup table instead of multiply:
decode a packed 2-bit block into signs, look up the precomputed product with
the activation, accumulate. No FP multiply per weight in the hot loop.

## Context Sliding Window

When the KV cache overflows, shift the window (--context-shift): keep the
recent turns, drop the oldest, and reuse the prefix via --cache-reuse so the
pinned prefix stays cached. Injected context must survive the shift.

## Concurrency and Locks

Serializing writers with a single mutex avoids sqlite "database is locked".
Hold the write lock across the ENTIRE write, not just the connection fetch —
concurrent threads otherwise stall up to busy_timeout.

## Deterministic Iteration

Hash iteration order varies per process; any map iteration that produces
output (IR, layout, ordering-dependent artifacts) MUST be sorted by key.

## Numerical Rigor

Quantized/float-heavy kernels: enforce mixed-type arithmetic explicitly,
never rely on silent bitcasts. Check NaN/Inf at every FFI/GPU/kernel
boundary and verify quantized output against a reference at a print boundary,
never == on floats.

## Formats Generalize

A kernel written for one weight format (e.g. TQ1_0) must let another
ternary/low-bit format use it without a rewrite. Push format knowledge to
configuration or a spec, not into a special case name.