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

use crate::gguf::{tensor_size_bytes, GGML_TYPE_TABLE};

/// The block size (bytes) of a super-block quant whose f16 scale `d` sits at
/// block offset 0 (so a zero scale zeroes every value — dequant = scale × code),
/// or None for unsupported types. Block byte-size equals the GGUF type size for
/// block formats.
pub fn block_size(ggml_type: i32) -> Option<usize> {
    match ggml_type {
        16..=23 => GGML_TYPE_TABLE
            .iter()
            .find(|t| t.enum_val == ggml_type)
            .map(|t| t.type_size as usize),
        _ => None,
    }
}

/// A parsed tensor payload location.
#[derive(Debug, Clone)]
pub struct TensorSlot {
    /// Tensor name (e.g. `blk.0.ffn_gate.weight`).
    pub name: String,
    /// GGML type enum value.
    pub ggml_type: i32,
    /// Logical shape (GGUF dims).
    pub ne: Vec<i64>,
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

/// Read one tensor-info entry into a slot.
fn read_tensor_slot(f: &mut File) -> TensorSlot {
    let name = read_str(f);
    let nd = read_u32(f);
    let mut ne = Vec::with_capacity(nd as usize);
    for _ in 0..nd {
        ne.push(read_i64(f));
    }
    let ggml_type = read_i32(f);
    let offset = read_u64(f);
    let size = tensor_size_bytes(ggml_type, &ne);
    TensorSlot {
        name,
        ggml_type,
        ne,
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

/// True when a GGML type can be masked exactly by this module: raw fp types
/// (element-level) and the super-block formats whose f16 scale sits at block
/// offset 0 (block-level zeroing — dequant = scale × code, so a zero scale
/// zeroes every value in the block).
pub fn maskable(ggml_type: i32) -> bool {
    matches!(ggml_type, 0 | 1) || block_size(ggml_type).is_some()
}

/// Zero the f16 scale (bytes 0..2) of a random `ratio` fraction of blocks in a
/// quantized payload. Every dequantized value in a zero-scale block becomes 0
/// (dequant = scale × code) — exact, size-preserving, no block decode needed.
pub fn mask_quantized(payload: &[u8], ratio: f64, block_size: usize, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64::new(seed);
    let mut out = payload.to_vec();
    if block_size == 0 || payload.len() < block_size {
        return out;
    }
    let n_blocks = payload.len() / block_size;
    let to_zero = (n_blocks as f64 * ratio.clamp(0.0, 1.0)) as usize;
    let mut picked = std::collections::HashSet::new();
    let mut guard = 0u64;
    while picked.len() < to_zero && guard < (n_blocks as u64 + 1) * 8 {
        let idx = (rng.next() % n_blocks.max(1) as u64) as usize;
        if picked.insert(idx) {
            let base = idx * block_size;
            out[base] = 0;
            out[base + 1] = 0;
        }
        guard += 1;
    }
    out
}

/// The expert-drop operation on one FFN weight tensor (bundled to keep the
/// entry point under the param-count gate).
pub struct DrossEdit<'a> {
    /// Encoded tensor payload.
    pub payload: &'a [u8],
    /// GGML type enum value.
    pub ggml_type: i32,
    /// Logical shape (GGUF dims).
    pub ne: &'a [i64],
    /// Total expert count (the MoE pool).
    pub n_expert: u64,
    /// Hidden size per expert.
    pub n_ffn_expert: u64,
    /// Dross (never-fired) expert ids, sorted.
    pub dross: &'a [u32],
}

impl DrossEdit<'_> {
    /// Zero every block fully contained within a dross expert's element range.
    ///
    /// The tensor is 2-D `[a, b]` where one dim equals
    /// `n_expert × n_ffn_expert` (the expert-major dim) and the other is the
    /// hidden dim. Blocks tile the contiguous dim (ne[0]). A block is zeroed
    /// only when its entire element range lies inside one dross expert's rows,
    /// so blocks straddling an expert boundary are kept — conservative, never
    /// corrupts an active expert's rows.
    ///
    /// Returns the masked payload and the number of blocks zeroed.
    pub fn apply(&self) -> Option<(Vec<u8>, u64)> {
        let Self {
            payload,
            ggml_type,
            ne,
            n_expert,
            n_ffn_expert,
            dross,
        } = *self;
        if ne.len() < 2 {
            return None;
        }
        let blk = block_size(ggml_type)? as i64;
        if blk <= 0 {
            return None;
        }
        let a = ne[0];
        let b = ne[1];
        let expert_total = (n_expert * n_ffn_expert) as i64;
        // Determine which dim is expert-major.
        let expert_dim = if a == expert_total {
            0
        } else if b == expert_total {
            1
        } else {
            return None;
        };
        let mut out = payload.to_vec();
        let mut zeroed = 0u64;
        // Element-index of a block given its byte position.
        // Column-major: element(i, j) is at i + j*a. Bytes-per-element is
        // fractional for block quants (e.g. iq2_s = 82/256 B/el), so use f64.
        let bytes_per_el = payload.len() as f64 / (a * b) as f64;
        let elems_per_block = (blk as f64 / bytes_per_el) as i64;
        if elems_per_block <= 0 {
            return None;
        }
        let n_blocks = payload.len() / blk as usize;
        for bi in 0..n_blocks {
            let byte_off = (bi * blk as usize) as i64;
            let el_start = (byte_off as f64 / bytes_per_el) as i64;
            let el_end = el_start + elems_per_block;
            if el_end > a * b {
                continue;
            }
            // Map the block's element range to expert-major indices.
            let (em_start, em_end) = if expert_dim == 0 {
                (el_start, el_end)
            } else {
                // expert-major on dim 1: element range spans a fixed j (= i/a)
                let j_start = el_start / a;
                let j_end = (el_end - 1) / a;
                if j_start != j_end {
                    continue; // block crosses rows — conservative skip
                }
                (j_start, j_end + 1)
            };
            if in_dross(em_start, em_end, n_ffn_expert, dross) {
                let base = (bi * blk as usize) as usize;
                out[base] = 0;
                out[base + 1] = 0;
                zeroed += 1;
            }
        }
        Some((out, zeroed))
    }
}

/// True when the `[s, e)` element range lies entirely within one dross expert's
/// rows.
fn in_dross(s: i64, e: i64, n_ffn_expert: u64, dross: &[u32]) -> bool {
    if e <= s {
        return false;
    }
    let exp_size = n_ffn_expert as i64;
    let expert_of_start = s / exp_size;
    let expert_of_end = (e - 1) / exp_size;
    if expert_of_start != expert_of_end {
        return false; // spans two experts
    }
    dross.binary_search(&(expert_of_start as u32)).is_ok()
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
        assert!(maskable(20)); // iq4_nl
        assert!(maskable(22)); // iq2_s
        assert!(maskable(16)); // iq2_xxs
        assert!(!maskable(10)); // q2_K (scale not at offset 0)
        assert_eq!(block_size(20), Some(18));
        assert_eq!(block_size(22), Some(82));
        assert_eq!(block_size(16), Some(66));
    }

    #[test]
    fn mask_dross_zeroes_expert_rows_only() {
        // iq4_nl (blk=32 elements). ne=[64, 1], n_expert=2, n_ffn=32 =>
        // expert_total=64 on dim0. Block0=[0,32)=expert0, block1=[32,64)=expert1.
        let ne = [64i64, 1i64];
        let mut payload = Vec::new();
        for _ in 0..2 {
            payload.push(0x01); // nonzero scale low
            payload.push(0x3C); // nonzero scale high
            payload.extend_from_slice(&[0xAB; 16]);
        }
        let (masked, zeroed) = DrossEdit {
            payload: &payload,
            ggml_type: 20,
            ne: &ne,
            n_expert: 2,
            n_ffn_expert: 32,
            dross: &[0],
        }
        .apply()
        .unwrap();
        assert_eq!(zeroed, 1);
        // block0 (expert0) scale zeroed, block1 (expert1) untouched
        assert_eq!(masked[0], 0);
        assert_eq!(masked[1], 0);
        assert_eq!(masked[18], 0x01);
        assert_eq!(masked[19], 0x3C);
    }

    #[test]
    fn mask_dross_skips_spanning_blocks() {
        // ne=[8, 4], n_expert=4, n_ffn=2 => expert_total=8 on dim0.
        // Single 32-el block spans experts 0..3 -> kept (conservative).
        let ne = [8i64, 4i64];
        let mut payload = vec![0x01u8, 0x3C];
        payload.extend_from_slice(&[0xAB; 16]);
        let (masked, zeroed) = DrossEdit {
            payload: &payload,
            ggml_type: 20,
            ne: &ne,
            n_expert: 4,
            n_ffn_expert: 2,
            dross: &[0],
        }
        .apply()
        .unwrap();
        assert_eq!(zeroed, 0);
        assert_eq!(masked[0], 0x01);
    }

    #[test]
    fn mask_quantized_zeroes_blocks_via_scale() {
        // two iq4_nl blocks (18 bytes each) with nonzero scales
        let mut payload = Vec::new();
        for b in 0..4 {
            payload.push(0x00); // scale low
            payload.push(0x3C + b as u8); // scale high (nonzero f16)
            payload.extend_from_slice(&[0xAB; 16]);
        }
        let masked = mask_quantized(&payload, 0.5, 18, 1);
        assert_eq!(masked.len(), payload.len());
        // exactly 2 blocks' scales zeroed; other bytes untouched
        let zeroed_scales = (0..4)
            .filter(|&b| masked[b * 18] == 0 && masked[b * 18 + 1] == 0)
            .count();
        assert_eq!(zeroed_scales, 2);
        // untouched blocks keep original data
        for b in 0..4 {
            if masked[b * 18] != 0 || masked[b * 18 + 1] != 0 {
                assert_eq!(
                    &masked[b * 18 + 2..b * 18 + 18],
                    &payload[b * 18 + 2..b * 18 + 18]
                );
            }
        }
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
