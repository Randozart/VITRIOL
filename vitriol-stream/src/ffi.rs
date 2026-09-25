//! C ABI for the C++ shim. Every entry is `extern "C"`, never unwinds
//! (panic = abort is set crate-wide, and each entry validates inputs), and
//! returns 0/-1-style codes mirroring the C++ semantics.

use crate::lru::{config, Lru};
use crate::store::{path_from_c_ptr, Store};
use std::ffi::{c_char, c_void};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

const MAX_DEVICES: usize = 16;

// Raw CUDA handles (streams/events/device ptrs) are only used from the
// single ggml compute thread; the Mutex is the sync point.
unsafe impl Send for State {}
unsafe impl Sync for State {}

struct State {
    bindings: HashMap<usize, u32>, // tensor_base -> tid
    max_expert_bytes: usize,
    lrus: Vec<Option<Lru>>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            bindings: HashMap::new(),
            max_expert_bytes: 0,
            lrus: (0..MAX_DEVICES).map(|_| None).collect(),
        })
    })
}

fn store() -> &'static OnceLock<Arc<Store>> {
    static S: OnceLock<Arc<Store>> = OnceLock::new();
    &S
}

/// `kind` per store.h: 0=ffn_up, 1=ffn_gate, 2=ffn_down, 3=shexp.
/// The matcher mirrors vitriol_name_is_expert/store_kind_from_name; shexp
/// tensors are NOT bound (excluded there too).
fn kind_of(name: &str) -> Option<u32> {
    if name.contains("ffn_up_exps") {
        Some(0)
    } else if name.contains("ffn_gate_exps") {
        Some(1)
    } else if name.contains("ffn_down_exps") {
        Some(2)
    } else {
        None
    }
}

fn layer_of(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("blk.")?;
    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if num.is_empty() || !rest.starts_with(&format!("{num}.")) {
        return None;
    }
    num.parse().ok()
}

///---------------- ABI ----------------

#[no_mangle]
pub extern "C" fn vs_abi_version() -> u32 {
    crate::ABI_VERSION
}

/// Read VITRIOL_* env once. Returns 0 on success.
#[no_mangle]
pub extern "C" fn vs_init() -> i32 {
    let _ = config();
    0
}

/// Open the global store. 0 on success, -1 on failure.
#[no_mangle]
pub extern "C" fn vs_open_store(path: *const c_char) -> i32 {
    let Some(p) = path_from_c_ptr(path) else { return -1 };
    match Store::open(&p) {
        Ok(s) => {
            let _ = store().set(Arc::new(s));
            if std::env::var_os("VITRIOL_VERBOSE").is_some() {
                eprintln!("VITRIOL(rust): store opened ({p})");
            }
            0
        }
        Err(e) => {
            eprintln!("VITRIOL(rust): store open failed: {e:?}");
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn vs_tensor_count() -> u32 {
    store().get().map(|s| s.n_tensors()).unwrap_or(0)
}

/// Bind tensor_base -> tid for (layer, kind) with full geometry checks.
/// Returns 1 on success, 0 on failure (mirrors C++ bool).
#[no_mangle]
pub extern "C" fn vs_bind_tensor_named(
    tensor_base: *mut c_void,
    name: *const c_char,
    tensor_bytes: usize,
    row_bytes: usize,
) -> i32 {
    let Some(name) = path_from_c_ptr(name) else { return 0 };
    let Some(kind) = kind_of(&name) else { return 0 };
    let Some(layer) = layer_of(&name) else { return 0 };
    let Some(s) = store().get() else { return 0 };
    let Some(tid) = s.find_tensor(layer, kind) else { return 0 };
    let Some(d) = s.desc(tid) else { return 0 };
    if d.n_experts == 0
        || d.data_bytes == 0
        || d.slice_bytes < d.data_bytes
        || row_bytes != d.data_bytes as usize
        || d.n_experts as usize * d.data_bytes as usize != tensor_bytes
    {
        return 0;
    }
    let mut st = state().lock().unwrap();
    st.max_expert_bytes = st.max_expert_bytes.max(d.data_bytes as usize);
    if let Some(prev) = st.bindings.get(&(tensor_base as usize)) {
        return (*prev == tid) as i32;
    }
    st.bindings.insert(tensor_base as usize, tid);
    if std::env::var_os("VITRIOL_VERBOSE").is_some() {
        println!(
            "VITRIOL(rust): store reg {name} -> tid {tid} (base {:p}, {tensor_bytes} B)",
            tensor_base
        );
    }
    1
}

#[no_mangle]
pub extern "C" fn vs_max_expert_bytes() -> usize {
    state().lock().unwrap().max_expert_bytes
}

/// Forward of vitriol_lru_ensure. Returns the device pointer, 0 for
/// "uncached fallback", or usize::MAX on a protocol violation (caller
/// aborts).
#[no_mangle]
pub extern "C" fn vs_lru_ensure(
    tensor_base: *const c_void,
    expert_idx: i32,
    expert_data: *const c_void,
    expert_size: usize,
    compute_stream: *mut c_void,
) -> usize {
    let cfg = config();
    let sres = |st: &mut State| -> Result<usize, String> {
        let dev = crate::driver::ctx_device().map_err(|e| {
            let mut cur: usize = 0;
            let gc = unsafe { crate::driver::cu_ctx_get_current(&mut cur) };
            format!("{e} [ctx_current={gc:#x} cur={cur:#x}]")
        })?;
        if st.lrus.get(dev as usize).map_or(true, |l| l.is_none()) {
            let lru = Lru::init(dev, expert_size, st.max_expert_bytes, cfg.store_device)?;
            st.lrus[dev as usize] = Some(lru);
        }
        let store_pair = if cfg.store_device {
            let s = store().get().ok_or("store fill but store not open")?;
            let tid = *st
                .bindings
                .get(&(tensor_base as usize))
                .ok_or("store H2D has no exact tensor binding")?;
            Some((s.as_ref(), tid))
        } else {
            None
        };
        let lru = st.lrus[dev as usize].as_mut().unwrap();
        lru.ensure(
            tensor_base,
            expert_idx,
            expert_data,
            expert_size,
            compute_stream,
            store_pair,
            cfg.store_device,
        )
    };

    let mut st = state().lock().unwrap();
    match sres(&mut st) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("VITRIOL(rust): {e}");
            usize::MAX // sentinel: caller aborts
        }
    }
}

#[no_mangle]
pub extern "C" fn vs_lru_mark_compute_done(vram_ptr: usize, stream: *mut c_void) -> i32 {
    let mut st = state().lock().unwrap();
    for l in st.lrus.iter_mut().flatten() {
        if let Err(e) = l.mark_compute_done(vram_ptr, stream) {
            eprintln!("VITRIOL(rust): mark_compute_done: {e}");
            return -1;
        }
    }
    0
}

/// Out-parameter form (FFI-safe): pass non-null pointers to receive counts.
#[no_mangle]
pub extern "C" fn vs_lru_stats(
    hits: *mut u64,
    misses: *mut u64,
    evictions: *mut u64,
) -> i32 {
    let st = state().lock().unwrap();
    let (mut h, mut m, mut e) = (0u64, 0u64, 0u64);
    for l in st.lrus.iter().flatten() {
        let s = l.stats();
        h += s.hits;
        m += s.misses;
        e += s.evictions;
    }
    unsafe {
        if !hits.is_null() {
            *hits = h;
        }
        if !misses.is_null() {
            *misses = m;
        }
        if !evictions.is_null() {
            *evictions = e;
        }
    }
    0
}

#[no_mangle]
pub extern "C" fn vs_print_stats() {
    let st = state().lock().unwrap();
    for (dev, l) in st.lrus.iter().enumerate() {
        let Some(l) = l else { continue };
        let s = l.stats();
        println!("=== VITRIOL(rust) Statistics (GPU {dev}) ===");
        println!("LRU Hits: {}", s.hits);
        println!("LRU Misses: {}", s.misses);
        println!("LRU Evictions: {}", s.evictions);
    }
}
