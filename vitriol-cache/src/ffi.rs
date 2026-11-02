// VITRIOL stack-pull S5: C ABI shim over the policy primitives.
//
// The C++ side (server_prompt_cache) owns the KV state blobs; this crate is
// the lookup/eviction POLICY brain. Handles are opaque u64 values chosen by
// the caller. The `vc_*` engine functions wrap the radix prefix index;
// `vc_arc_*` exposes the ARC policy standalone (future disk-tier eviction
// order). All entry points are panic-free at the boundary.
//
// PROVENANCE: interface shape mirrors vitriol-stream's vs_* ABI (fork-owned,
// Apache-2.0 OR MIT); policy provenance lives in radix.rs / arc.rs.

use std::ffi::c_int;

use crate::arc::Arc;
use crate::radix::{RadixTree, NO_VALUE};

/// Prefix-index engine: radix trie over token sequences.
pub struct VcEngine {
    radix: RadixTree,
}

/// Standalone ARC instance (disk-tier eviction order).
pub struct VcArc {
    arc: Arc,
}

const VC_OK: c_int = 0;
const VC_ERR: c_int = -1;
const VC_MISS: c_int = 0;
const VC_HIT: c_int = 1;

impl VcEngine {
    pub fn new(capacity_hint: usize) -> Self {
        let _ = capacity_hint;
        VcEngine {
            radix: RadixTree::new(),
        }
    }
}

#[no_mangle]
pub extern "C" fn vc_new(_capacity_hint: usize) -> *mut VcEngine {
    Box::into_raw(Box::new(VcEngine::new(_capacity_hint)))
}

/// # Safety
/// `engine` must be a pointer returned by `vc_new`, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn vc_free(engine: *mut VcEngine) {
    if !engine.is_null() {
        drop(Box::from_raw(engine));
    }
}

/// # Safety
/// `engine` valid; `tokens` points to `n` u64 token ids (or is null iff n=0).
#[no_mangle]
pub unsafe extern "C" fn vc_insert(
    engine: *mut VcEngine,
    tokens: *const u64,
    n_tokens: usize,
    value: u64,
    out_old: *mut u64,
) -> c_int {
    if engine.is_null() || (tokens.is_null() && n_tokens > 0) || out_old.is_null() {
        return VC_ERR;
    }
    let engine = &mut *engine;
    let slice = std::slice::from_raw_parts(tokens, n_tokens);
    *out_old = NO_VALUE;
    if let Some(old) = engine.radix.insert(slice, value) {
        *out_old = old;
    }
    VC_OK
}

/// # Safety
/// `engine` valid; out params writable.
#[no_mangle]
pub unsafe extern "C" fn vc_match(
    engine: *mut VcEngine,
    tokens: *const u64,
    n_tokens: usize,
    out_value: *mut u64,
    out_depth: *mut usize,
) -> c_int {
    if engine.is_null()
        || (tokens.is_null() && n_tokens > 0)
        || out_value.is_null()
        || out_depth.is_null()
    {
        return VC_ERR;
    }
    let engine = &mut *engine;
    let slice = std::slice::from_raw_parts(tokens, n_tokens);
    match engine.radix.match_prefix(slice) {
        Some((value, depth)) => {
            *out_value = value;
            *out_depth = depth;
            VC_HIT
        }
        None => VC_MISS,
    }
}

/// # Safety
/// `engine` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_touch(engine: *mut VcEngine, value: u64) -> c_int {
    if engine.is_null() {
        return VC_ERR;
    }
    (*engine).radix.touch(value);
    VC_OK
}

/// # Safety
/// `engine` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_pin(engine: *mut VcEngine, value: u64) -> c_int {
    if engine.is_null() {
        return VC_ERR;
    }
    if (*engine).radix.pin(value) {
        VC_OK
    } else {
        VC_MISS
    }
}

/// # Safety
/// `engine` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_unpin(engine: *mut VcEngine, value: u64) -> c_int {
    if engine.is_null() {
        return VC_ERR;
    }
    (*engine).radix.unpin(value);
    VC_OK
}

/// # Safety
/// `engine` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_remove(engine: *mut VcEngine, value: u64) -> c_int {
    if engine.is_null() {
        return VC_ERR;
    }
    if (*engine).radix.remove(value) {
        VC_OK
    } else {
        VC_MISS
    }
}

/// # Safety
/// `engine` valid; `out_values` writable for `out_cap` u64s; `out_len` writable.
/// Evicts least-recently-hit unpinned registrations until at most
/// `max_entries` remain (or no more evictable), filling `out_values`.
#[no_mangle]
pub unsafe extern "C" fn vc_evict(
    engine: *mut VcEngine,
    max_entries: usize,
    out_values: *mut u64,
    out_cap: usize,
    out_len: *mut usize,
) -> c_int {
    if engine.is_null() || out_len.is_null() || (out_values.is_null() && out_cap > 0) {
        return VC_ERR;
    }
    let engine = &mut *engine;
    let evicted = engine.radix.evict_lru(max_entries);
    let n = evicted.len().min(out_cap);
    for (i, v) in evicted.iter().take(n).enumerate() {
        out_values.add(i).write(*v);
    }
    *out_len = n;
    VC_OK
}

/// # Safety
/// `engine` valid; `out_len` writable.
#[no_mangle]
pub unsafe extern "C" fn vc_len(engine: *mut VcEngine, out_len: *mut usize) -> c_int {
    if engine.is_null() || out_len.is_null() {
        return VC_ERR;
    }
    *out_len = (*engine).radix.len();
    VC_OK
}

// ---- standalone ARC (disk-tier eviction order, not wired yet) ----

/// # Safety
#[no_mangle]
pub extern "C" fn vc_arc_new(capacity: usize) -> *mut VcArc {
    Box::into_raw(Box::new(VcArc {
        arc: Arc::new(capacity),
    }))
}

/// # Safety
/// `arc` must be a pointer returned by `vc_arc_new`, not yet freed.
#[no_mangle]
pub unsafe extern "C" fn vc_arc_free(arc: *mut VcArc) {
    if !arc.is_null() {
        drop(Box::from_raw(arc));
    }
}

/// # Safety
/// `arc` valid; `out_evicted` writable (may be null to ignore the victim).
#[no_mangle]
pub unsafe extern "C" fn vc_arc_touch(arc: *mut VcArc, key: u64, out_evicted: *mut u64) -> c_int {
    if arc.is_null() {
        return VC_ERR;
    }
    let victim = (*arc).arc.touch(key);
    if !out_evicted.is_null() {
        out_evicted.write(victim.unwrap_or(NO_VALUE));
    }
    VC_OK
}

/// # Safety
/// `arc` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_arc_forget(arc: *mut VcArc, key: u64) -> c_int {
    if arc.is_null() {
        return VC_ERR;
    }
    (*arc).arc.forget(key);
    VC_OK
}

/// # Safety
/// `arc` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_arc_pin(arc: *mut VcArc, key: u64) -> c_int {
    if arc.is_null() {
        return VC_ERR;
    }
    (*arc).arc.pin(key);
    VC_OK
}

/// # Safety
/// `arc` valid.
#[no_mangle]
pub unsafe extern "C" fn vc_arc_unpin(arc: *mut VcArc, key: u64) -> c_int {
    if arc.is_null() {
        return VC_ERR;
    }
    (*arc).arc.unpin(key);
    VC_OK
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_insert_match_evict_roundtrip() {
        unsafe {
            let e = vc_new(0);
            let toks = [1u64, 2, 3, 4];
            let mut old: u64 = 0;
            assert_eq!(vc_insert(e, toks.as_ptr(), 4, 100, &mut old), VC_OK);
            assert_eq!(old, NO_VALUE);

            let mut value: u64 = 0;
            let mut depth: usize = 0;
            assert_eq!(
                vc_match(e, toks.as_ptr(), 4, &mut value, &mut depth),
                VC_HIT
            );
            assert_eq!((value, depth), (100, 4));

            // replace: old handle comes back
            vc_insert(e, toks.as_ptr(), 4, 101, &mut old);
            assert_eq!(old, 100);

            // evict to zero frees the (unpinned) registration
            let mut out = [0u64; 8];
            let mut n = 0usize;
            assert_eq!(vc_evict(e, 0, out.as_mut_ptr(), 8, &mut n), VC_OK);
            assert_eq!(n, 1);
            assert_eq!(out[0], 101);
            assert_eq!(
                vc_match(e, toks.as_ptr(), 4, &mut value, &mut depth),
                VC_MISS
            );

            vc_free(e);
        }
    }

    #[test]
    fn engine_pins_block_eviction() {
        unsafe {
            let e = vc_new(0);
            let a = [7u64];
            let b = [9u64];
            let mut old: u64 = 0;
            vc_insert(e, a.as_ptr(), 1, 1, &mut old);
            vc_insert(e, b.as_ptr(), 1, 2, &mut old);
            assert_eq!(vc_pin(e, 2), VC_OK);

            let mut out = [0u64; 8];
            let mut n = 0usize;
            vc_evict(e, 0, out.as_mut_ptr(), 8, &mut n);
            assert_eq!((n, out[0]), (1, 1)); // only the unpinned one
            assert_eq!(vc_evict(e, 0, out.as_mut_ptr(), 8, &mut n), VC_OK);
            assert_eq!(n, 0); // pinned refuses

            vc_free(e);
        }
    }

    #[test]
    fn null_args_are_errors_not_crashes() {
        unsafe {
            assert_eq!(
                vc_insert(
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    0,
                    1,
                    std::ptr::null_mut()
                ),
                VC_ERR
            );
            let e = vc_new(0);
            let mut v = 0u64;
            let mut d = 0usize;
            assert_eq!(vc_match(e, std::ptr::null(), 3, &mut v, &mut d), VC_ERR);
            vc_free(std::ptr::null_mut());
            vc_free(e);
        }
    }

    #[test]
    fn arc_roundtrip() {
        unsafe {
            let a = vc_arc_new(2);
            let mut ev = 0u64;
            vc_arc_touch(a, 1, &mut ev);
            vc_arc_touch(a, 2, &mut ev);
            vc_arc_touch(a, 3, &mut ev);
            assert_eq!(ev, 1); // LRU evicted
            vc_arc_free(a);
        }
    }
}
