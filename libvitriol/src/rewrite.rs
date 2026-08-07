//! Size-preserving GGUF rewrite — the weight-surgery write path.
//!
//! A GGUF file is `[header + metadata + tensor-info] [tensor payloads]` with
//! payloads at recorded offsets. Rewriting a masked tensor IN PLACE (same byte
//! size, same quant format) keeps every offset valid, so the header never needs
//! re-serialization and untouched tensors stay byte-identical.
//!
//! This is the database-write layer of the LARQL "model is the database" idea:
//! `plan()` indexes the tensors, `copy_and_edit()` writes a new file with
//! same-size payload replacements. f16/f32 masking is exact; quantized masking
//! (iq2_s/iq4_nl) is a follow-up — quantized tensors are byte-copied untouched.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::gguf::tensor_size_bytes;

/// A parsed tensor payload location.
#[derive(Debug, Clone)]
pub struct TensorSlot {
    /// Tensor name (e.g. `blk.0.ffn_gate.weight`).
    pub name: String,
    /// GGML type enum value.
    pub ggml_type: i32,
    /// Byte offset of the payload in the file.
    pub offset: u64,
    /// Payload size in bytes.
    pub size: u64,
}

/// The parse result: tensor payload layout of a GGUF file.
#[derive(Debug)]
pub struct RewritePlan {
    /// Byte offset where the first tensor payload begins (header end).
    pub header_end: u64,
    /// Total source file size.
    pub file_len: u64,
    /// Tensor payload slots, in file order.
    pub tensors: Vec<TensorSlot>,
}

impl RewritePlan {
    /// The slot for a tensor name, if present.
    pub fn find(&self, name: &str) -> Option<&TensorSlot> {
        self.tensors.iter().find(|t| t.name == name)
    }
}

/// One same-size payload replacement: write `bytes` over tensor slot `index`.
#[derive(Debug, Clone)]
pub struct Edit {
    /// Index into `RewritePlan::tensors`.
    pub index: usize,
    /// Replacement payload bytes; must equal the slot's size exactly.
    pub bytes: Vec<u8>,
}

/// Parse the GGUF header + tensor-info, returning the payload layout.
pub fn plan(path: &Path) -> anyhow::Result<RewritePlan> {
    use anyhow::Context;
    let mut f = File::open(path).with_context(|| format!("cannot open: {}", path.display()))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        anyhow::bail!("not a GGUF file");
    }
    let _version = read_u32(&mut f);
    let tensor_count = read_u64(&mut f);
    let kv_count = read_u64(&mut f);

    for _ in 0..kv_count {
        let _key = read_str(&mut f);
        let tval = read_i32(&mut f);
        skip_val(&mut f, tval);
    }

    let mut tensors = Vec::with_capacity(tensor_count as usize);
    for _ in 0..tensor_count {
        tensors.push(read_tensor_slot(&mut f));
    }

    let header_end = f.stream_position().with_context(|| "stream_position")?;
    let file_len = f.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(RewritePlan {
        header_end,
        file_len,
        tensors,
    })
}

/// Read one tensor-info entry: (name, ggml_type, offset, size).
fn read_tensor_slot(f: &mut File) -> TensorSlot {
    let name = read_str(f);
    let nd = read_u32(f);
    let mut dims = Vec::with_capacity(nd as usize);
    for _ in 0..nd {
        dims.push(read_i64(f));
    }
    let ggml_type = read_i32(f);
    let offset = read_u64(f);
    let size = tensor_size_bytes(ggml_type, &dims);
    TensorSlot {
        name,
        ggml_type,
        offset,
        size,
    }
}

/// Copy `src` to `dst` byte-for-byte, then overwrite each edited payload in
/// place. Every edit must be exactly the slot's size. Returns bytes written.
pub fn copy_and_edit(
    src: &Path,
    dst: &Path,
    plan: &RewritePlan,
    edits: &[Edit],
) -> anyhow::Result<u64> {
    fs::copy(src, dst)?;
    let mut f = File::options()
        .write(true)
        .open(dst)
        .map_err(|e| anyhow::anyhow!("open {}: {e}", dst.display()))?;
    for edit in edits {
        let slot = plan
            .tensors
            .get(edit.index)
            .ok_or_else(|| anyhow::anyhow!("edit index {} out of range", edit.index))?;
        anyhow::ensure!(
            edit.bytes.len() as u64 == slot.size,
            "edit for '{}' size {} != slot size {}",
            slot.name,
            edit.bytes.len(),
            slot.size
        );
        f.seek(SeekFrom::Start(slot.offset))?;
        f.write_all(&edit.bytes)?;
    }
    Ok(plan.file_len)
}

/// Zero a random `ratio` fraction of an f16 tensor's payload elements.
/// Returns a same-size replacement. Deterministic when `seed` is fixed; picks
/// unique indices so the zeroed fraction is exact.
pub fn mask_f16(payload: &[u8], ratio: f64, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    let mut out = payload.to_vec();
    let n = payload.len() / 2;
    let to_zero = (n as f64 * ratio.clamp(0.0, 1.0)) as usize;
    let mut picked = std::collections::HashSet::new();
    let mut guard = 0u64;
    while picked.len() < to_zero && guard < (n as u64 + 1) * 8 {
        let idx = (rng.next() % n.max(1) as u64) as usize;
        if picked.insert(idx) {
            out[idx * 2] = 0;
            out[idx * 2 + 1] = 0;
        }
        guard += 1;
    }
    out
}

/// Zero a random `ratio` fraction of an f32 tensor's payload elements.
pub fn mask_f32(payload: &[u8], ratio: f64, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    let mut out = payload.to_vec();
    let n = payload.len() / 4;
    let to_zero = (n as f64 * ratio.clamp(0.0, 1.0)) as usize;
    let mut picked = std::collections::HashSet::new();
    let mut guard = 0u64;
    while picked.len() < to_zero && guard < (n as u64 + 1) * 8 {
        let idx = (rng.next() % n.max(1) as u64) as usize;
        if picked.insert(idx) {
            out[idx * 4..idx * 4 + 4].fill(0);
        }
        guard += 1;
    }
    out
}

/// True when a GGML type can be masked exactly by this module (raw fp types).
pub fn maskable(ggml_type: i32) -> bool {
    matches!(ggml_type, 0 | 1) // f32, f16
}

/// SplitMix64 — deterministic seedable PRNG for mask selection.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
}

fn read_u32(f: &mut File) -> u32 {
    let mut b = [0u8; 4];
    let _ = f.read_exact(&mut b);
    u32::from_le_bytes(b)
}

fn read_i32(f: &mut File) -> i32 {
    let mut b = [0u8; 4];
    let _ = f.read_exact(&mut b);
    i32::from_le_bytes(b)
}

fn read_u64(f: &mut File) -> u64 {
    let mut b = [0u8; 8];
    let _ = f.read_exact(&mut b);
    u64::from_le_bytes(b)
}

fn read_i64(f: &mut File) -> i64 {
    let mut b = [0u8; 8];
    let _ = f.read_exact(&mut b);
    i64::from_le_bytes(b)
}

fn read_str(f: &mut File) -> String {
    let len = read_u64(f) as usize;
    let mut buf = vec![0u8; len];
    if len > 0 {
        let _ = f.read_exact(&mut buf);
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Skip a GGUF metadata value of the given type tag.
fn skip_val(f: &mut File, tval: i32) {
    match tval {
        0 | 1 | 7 => {
            let _ = f.read_exact(&mut [0u8; 1]);
        }
        2 | 3 => {
            let _ = f.read_exact(&mut [0u8; 2]);
        }
        4..=6 => {
            let _ = f.read_exact(&mut [0u8; 4]);
        }
        10..=12 => {
            let _ = f.read_exact(&mut [0u8; 8]);
        }
        8 => {
            let len = read_u64(f) as usize;
            let _ = f.read_exact(&mut vec![0u8; len]);
        }
        9 => {
            let et = read_i32(f);
            let cnt = read_u64(f) as usize;
            for _ in 0..cnt {
                skip_val(f, et);
            }
        }
        _ => {
            let _ = f.read_exact(&mut [0u8; 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_f16_payload(values: &[f32]) -> Vec<u8> {
        let mut out = Vec::new();
        for v in values {
            out.extend_from_slice(&f16_bits(*v).to_le_bytes());
        }
        out
    }

    fn f16_bits(v: f32) -> u16 {
        // minimal f32->f16 (rounded) for tests
        let bits = v.to_bits();
        let sign = (bits >> 16) & 0x8000;
        let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
        let man = (bits >> 13) & 0x3ff;
        if exp >= 0x1f {
            return (sign | 0x7c00) as u16;
        }
        if exp <= 0 {
            return sign as u16;
        }
        (sign | ((exp as u32) << 10) | man) as u16
    }

    /// Build a minimal valid GGUF with one tensor, returning the file bytes.
    fn build_minimal_gguf() -> Vec<u8> {
        // magic(4) version(4) tensor_count(8) kv_count(8) — no KV, one tensor
        let mut out = Vec::new();
        out.extend_from_slice(b"GGUF");
        out.extend_from_slice(&3u32.to_le_bytes());
        out.extend_from_slice(&1u64.to_le_bytes()); // 1 tensor
        out.extend_from_slice(&0u64.to_le_bytes()); // 0 kv
                                                    // tensor info: name "t", ndims=1, dims=[8], type=f16(1), offset
        let name = b"t";
        out.extend_from_slice(&(name.len() as u64).to_le_bytes());
        out.extend_from_slice(name);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&8i64.to_le_bytes());
        out.extend_from_slice(&1i32.to_le_bytes()); // f16
                                                    // payload offset (computed after this block) — placeholder then fix
        let off_pos = out.len();
        out.extend_from_slice(&0u64.to_le_bytes());
        // payload: 8 f16 values
        let payload = write_f16_payload(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        let offset = out.len() as u64;
        out[off_pos..off_pos + 8].copy_from_slice(&offset.to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn plan_parses_minimal_gguf() {
        let p = std::env::temp_dir().join("rewrite_plan.bin");
        std::fs::write(&p, build_minimal_gguf()).unwrap();
        let plan = plan(&p).unwrap();
        assert_eq!(plan.tensors.len(), 1);
        assert_eq!(plan.tensors[0].name, "t");
        assert_eq!(plan.tensors[0].ggml_type, 1);
        assert_eq!(plan.tensors[0].size, 16);
        assert!(plan.header_end <= plan.tensors[0].offset);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn copy_and_edit_preserves_everything_else() {
        let src = std::env::temp_dir().join("rewrite_src.bin");
        let dst = std::env::temp_dir().join("rewrite_dst.bin");
        let original = build_minimal_gguf();
        std::fs::write(&src, &original).unwrap();
        let plan = plan(&src).unwrap();
        // zero the whole payload (mask all 8 f16 -> same size)
        let mask = mask_f16(&original[plan.tensors[0].offset as usize..], 1.0, 7);
        assert_eq!(mask.len(), 16);
        assert!(mask.iter().all(|&b| b == 0));
        copy_and_edit(
            &src,
            &dst,
            &plan,
            &[Edit {
                index: 0,
                bytes: mask,
            }],
        )
        .unwrap();
        let rewritten = std::fs::read(&dst).unwrap();
        assert_eq!(rewritten.len(), original.len());
        // header identical
        let off = plan.tensors[0].offset as usize;
        assert_eq!(&rewritten[..off], &original[..off]);
        // payload zeroed
        assert!(rewritten[off..].iter().all(|&b| b == 0));
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }

    #[test]
    fn mask_f16_zeroes_exact_fraction() {
        let payload = write_f16_payload(&(0..64).map(|i| i as f32).collect::<Vec<_>>());
        let masked = mask_f16(&payload, 0.5, 42);
        let zeros = masked.chunks(2).filter(|c| c[0] == 0 && c[1] == 0).count();
        assert_eq!(zeros, 32);
    }

    #[test]
    fn maskable_classification() {
        assert!(maskable(0)); // f32
        assert!(maskable(1)); // f16
        assert!(!maskable(20)); // iq4_nl
        assert!(!maskable(22)); // iq2_s
    }

    #[test]
    fn wrong_size_edit_rejected() {
        let src = std::env::temp_dir().join("rewrite_bad.bin");
        let dst = std::env::temp_dir().join("rewrite_bad2.bin");
        std::fs::write(&src, build_minimal_gguf()).unwrap();
        let plan = plan(&src).unwrap();
        let r = copy_and_edit(
            &src,
            &dst,
            &plan,
            &[Edit {
                index: 0,
                bytes: vec![0u8; 4],
            }],
        );
        assert!(r.is_err());
        let _ = std::fs::remove_file(&src);
        let _ = std::fs::remove_file(&dst);
    }
}
