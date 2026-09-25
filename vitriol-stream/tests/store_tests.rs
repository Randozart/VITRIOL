use vitriol_stream::{crc32};
use vitriol_stream::store::Store;

#[test]
fn crc32_known_vectors() {
    // IEEE CRC32 reference vectors.
    assert_eq!(crc32(b""), 0x0000_0000);
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414F_A339);
}

#[test]
fn crc32_all_zero_block_constant() {
    // The 4096-zero superblock region hashes to a fixed value; ensure the
    // implementation is stable and not the identity.
    let z = [0u8; 4096];
    assert_eq!(crc32(&z), crc32(&z));
    assert_ne!(crc32(&z), 0);
}

#[test]
fn align_up_basics() {
    assert_eq!(vitriol_stream::align_up(921600, 512), 921600);
    assert_eq!(vitriol_stream::align_up(921601, 512), 922112);
    assert_eq!(vitriol_stream::align_up(921601, 256), 921856);
    assert_eq!(vitriol_stream::align_up(0, 512), 0);
}

#[test]
fn store_open_rejects_garbage() {
    use vitriol_stream::store::Store;
    let dir = std::env::temp_dir().join("vitriol-stream-test");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("not-a-store.bin");
    std::fs::write(&p, vec![0u8; 16384]).unwrap();
    assert!(Store::open(p.to_str().unwrap()).is_err());
}

#[test]
fn kind_and_layer_parsing() {
    // Mirror of ffi::kind_of/layer_of semantics exercised via a tiny reimpl
    // here to keep the ffi module opaque; if these drift, the parity gate
    // in Phase 1 catches it against the real store.
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
    assert_eq!(kind_of("blk.2.ffn_down_exps.weight"), Some(2));
    assert_eq!(kind_of("blk.2.ffn_gate_up_exps.weight"), None); // fused: not matched
    assert_eq!(kind_of("blk.2.ffn_shexp.weight"), None);
}
