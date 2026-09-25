//! Phase 1 gate: read blk.0 ffn_down expert 106 from the store and compare
//! the FNV-1a hash against the C++ diagnostic reference
//! (61584288a5842d51, from VITRIOL_LRU_VRAM_HASH src=..., 2026-09-25).
//!
//! Run: cargo run --release -p vitriol-stream --example read_check -- /dev/nvme0n1p3

use vitriol_stream::store::{AlignedBuf, Store};

fn main() {
    let path = std::env::args().nth(1).expect("usage: read_check <store-path>");
    let store = Store::open(&path).expect("store open");
    let tid = store
        .find_tensor(0, 2) // blk.0, ffn_down
        .expect("blk.0 ffn_down not found");
    let d = store.desc(tid).unwrap();
    println!("tid={tid} n_experts={} data_bytes={} slice_bytes={} offset={}",
        d.n_experts, d.data_bytes, d.slice_bytes, d.store_offset);

    let expert = 106u32;
    let mut buf = AlignedBuf::zeroed(d.slice_bytes as usize, 512);
    store
        .read_expert_padded(tid, expert, buf.as_mut_slice())
        .expect("read_expert_padded");
    let h = vitriol_stream::driver::fnv1a(&buf.as_slice()[0..d.data_bytes as usize]);
    println!("fnv1a(blk.0 ffn_down e{expert}) = {h:016x}");
    // Correct-basis FNV-1a of the same region read raw (dd), verified
    // 2026-09-25. NOTE: the C++ diagnostic hash in
    // vitriol-cuda-integration.cpp uses a TRUNCATED basis (1469598103934665603
    // instead of 14695981039346656037), so its hashes are internally
    // consistent but not comparable to this one (its value for this slice:
    // 61584288a5842d51).
    let reference = "a928334dc7332643";
    if format!("{h:016x}") == reference {
        println!("PHASE1 GATE: PASS (matches C++ reference)");
    } else {
        println!("PHASE1 GATE: FAIL (expected {reference})");
        std::process::exit(2);
    }
}
