// PROVENANCE: own work (Apache-2.0 OR MIT). Re-implementation of the
// VITRIOL expert streaming path in safe Rust; on-disk store format and the
// H2D event protocol mirror ggml/src/ggml-cuda/vitriol-store.{h,cpp} and
// vitriol-cuda-integration.cpp (same project). The three-state victim
// selection idea is attributed there: inspiration - kimi-k3-in-c
// (Apache-2.0; re-derived, not copied).
//
// Design goal: every bug class the C++ path exhibited is prevented by
// construction - per-device `Lru` instead of file-scope globals, typed
// `SlotHandle` instead of raw ints with a 0 fallback, pending-tracking
// integral to victim selection, aligned O_DIRECT buffers by type, loud
// failures everywhere (panic = abort).

pub mod bounce;
pub mod driver;
pub mod ffi;
pub mod lru;
pub mod store;

/// Bump when the C ABI changes. The C++ shim checks this.
pub const ABI_VERSION: u32 = 1;

/// IEEE CRC32 (poly 0xEDB88320, init/final xor), matching C++ store_crc32.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

pub fn align_up(v: usize, a: usize) -> usize {
    (v + a - 1) / a * a
}
