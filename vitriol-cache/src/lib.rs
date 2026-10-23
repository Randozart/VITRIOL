//! vitriol-cache - prompt-cache policy brain for VITRIOL (stack-pull S5).
//!
//! Lookup/eviction POLICY over token sequences; the C++ server owns the KV
//! state bytes. Inert until wired behind `server_prompt_cache` (first GPU
//! maintenance window); until then this crate ships with its own CPU tests.
//!
//! - [`radix::RadixTree`]: content-keyed radix trie, deepest-ancestor
//!   prefix match, recency + pin-aware eviction (RadixAttention shape).
//! - [`arc::Arc`]: Adaptive Replacement Cache over opaque keys (disk-tier
//!   eviction order).
//! - [`ffi`]: C ABI (`vc_*`, `vc_arc_*`) for the llama.cpp server side.
//!
//! PROVENANCE: inspiration - SGLang/RadixAttention (Apache-2.0) and vLLM
//! (Apache-2.0); re-derived, not copied. Fork-owned, Apache-2.0 OR MIT.

pub mod arc;
pub mod ffi;
pub mod radix;
