//! W0 value census: decode a GGUF tensor's payload and compute structural
//! statistics (dead-lane fraction, magnitude, crude entropy).
//!
//! This is the measurement floor for the Officina DESCRIBE/CENSUS ops and for
//! pruning decisions (a dead-lane-heavy tensor is a pruning candidate). Values
//! are decoded block-by-block from the file at the tensor's recorded offset;
//! only f32, f16, q8_0, and q4_0 blocks are decoded (the block layouts are
//! simple and stable); other types report `unsupported` rather than guessing.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::gguf::TensorEntry;

/// Structural statistics for one tensor.
#[derive(Debug, Clone)]
pub struct Census {
    /// Number of decoded values.
    pub elements: u64,
    /// Fraction of values that are exactly zero (dead lanes).
    pub zero_fraction: f64,
    /// Mean absolute value.
    pub abs_mean: f64,
    /// Maximum absolute value.
    pub abs_max: f64,
    /// Crude Shannon entropy (bits) over an 8-bin magnitude histogram.
    pub entropy_bits: f64,
    /// Values actually decoded (may be capped by `sample_cap`).
    pub sampled: u64,
    /// True when the tensor's type is not decodable by this census.
    pub unsupported: bool,
}

/// How many values to decode at most (large tensors are sampled).
const SAMPLE_CAP: u64 = 262_144;

/// Run the W0 census over `entry` in `path`.
pub fn census_tensor(path: &Path, entry: &TensorEntry) -> std::io::Result<Census> {
    let mut f = File::open(path)?;
    f.seek(SeekFrom::Start(entry.offset))?;
    let n_elems: u64 = entry.shape.iter().copied().map(|d| d.max(0) as u64).product();
    match entry.ggml_type {
        0 => census_raw(&mut f, n_elems, 4, read_f32),
        1 => census_raw(&mut f, n_elems, 2, read_f16),
        8 => census_q8_0(&mut f, n_elems),
        2 => census_q4_0(&mut f, n_elems),
        _ => Ok(Census {
            elements: n_elems,
            zero_fraction: 0.0,
            abs_mean: 0.0,
            abs_max: 0.0,
            entropy_bits: 0.0,
            sampled: 0,
            unsupported: true,
        }),
    }
}

/// Census for raw (f32/f16) tensors.
fn census_raw<F>(f: &mut File, n: u64, width: u64, read: F) -> std::io::Result<Census>
where
    F: Fn(&mut &[u8]) -> std::io::Result<f32>,
{
    let mut acc = StatsAccum::new(n);
    let cap = n.min(SAMPLE_CAP);
    let mut buf = vec![0u8; (cap as usize).saturating_mul(width as usize)];
    f.read_exact(&mut buf)?;
    let mut cursor: &[u8] = &buf;
    while acc.count < cap {
        let v = read(&mut cursor)?;
        acc.push(v);
    }
    Ok(acc.finish())
}

/// Census for q8_0 blocks (2-byte f16 scale + 32 int8).
fn census_q8_0(f: &mut File, n: u64) -> std::io::Result<Census> {
    let mut acc = StatsAccum::new(n);
    let cap = n.min(SAMPLE_CAP);
    let blocks = cap.div_ceil(32);
    let mut b = [0u8; 34];
    for _ in 0..blocks {
        f.read_exact(&mut b)?;
        let scale = f16_to_f32(u16::from_le_bytes([b[0], b[1]]));
        for i in 0..32 {
            let v = scale * (b[2 + i] as i8 as f32);
            acc.push(v);
        }
    }
    Ok(acc.finish())
}

/// Census for q4_0 blocks (2-byte f16 scale + 16 bytes of nibbles).
fn census_q4_0(f: &mut File, n: u64) -> std::io::Result<Census> {
    let mut acc = StatsAccum::new(n);
    let cap = n.min(SAMPLE_CAP);
    let blocks = cap.div_ceil(32);
    let mut b = [0u8; 18];
    for _ in 0..blocks {
        f.read_exact(&mut b)?;
        let scale = f16_to_f32(u16::from_le_bytes([b[0], b[1]]));
        for i in 0..16 {
            let hi = ((b[2 + i] >> 4) as i32) - 8;
            let lo = ((b[2 + i] & 0xF) as i32) - 8;
            acc.push(scale * hi as f32);
            acc.push(scale * lo as f32);
        }
    }
    Ok(acc.finish())
}

/// Accumulator for the running census statistics.
struct StatsAccum {
    n: u64,
    count: u64,
    zeros: u64,
    abs_sum: f64,
    abs_max: f64,
    hist: [u64; 8],
}

impl StatsAccum {
    fn new(n: u64) -> Self {
        Self {
            n,
            count: 0,
            zeros: 0,
            abs_sum: 0.0,
            abs_max: 0.0,
            hist: [0; 8],
        }
    }

    fn push(&mut self, v: f32) {
        self.count += 1;
        let a = v.abs();
        if a == 0.0 {
            self.zeros += 1;
        }
        self.abs_sum += a as f64;
        if a as f64 > self.abs_max {
            self.abs_max = a as f64;
        }
        let bin = if a == 0.0 {
            0
        } else {
            ((a as f64).log10().clamp(-8.0, 8.0) / 2.0 + 4.0) as usize
        };
        let bin = bin.min(7);
        self.hist[bin] += 1;
    }

    fn finish(self) -> Census {
        let count = self.count.max(1);
        let mut entropy = 0.0;
        for &h in &self.hist {
            if h == 0 {
                continue;
            }
            let p = h as f64 / count as f64;
            entropy -= p * p.log2();
        }
        Census {
            elements: self.n,
            zero_fraction: self.zeros as f64 / count as f64,
            abs_mean: self.abs_sum / count as f64,
            abs_max: self.abs_max,
            entropy_bits: entropy,
            sampled: self.count,
            unsupported: false,
        }
    }
}

/// IEEE 754 half -> f32.
fn f16_to_f32(h: u16) -> f32 {
    let sign = if h >> 15 == 1 { -1.0f32 } else { 1.0f32 };
    let exp = (h >> 10) & 0x1F;
    let man = h & 0x3FF;
    let val = match exp {
        0 if man == 0 => 0.0,
        0 => man as f32 * 2.0f32.powi(-24),
        0x1F if man == 0 => f32::INFINITY,
        0x1F => f32::NAN,
        _ => (1.0 + man as f32 / 1024.0) * 2.0f32.powi(exp as i32 - 15),
    };
    sign * val
}

fn read_f32(f: &mut &[u8]) -> std::io::Result<f32> {
    let (head, tail) = f.split_at(4);
    *f = tail;
    let mut b = [0u8; 4];
    b.copy_from_slice(head);
    Ok(f32::from_le_bytes(b))
}

fn read_f16(f: &mut &[u8]) -> std::io::Result<f32> {
    let (head, tail) = f.split_at(2);
    *f = tail;
    Ok(f16_to_f32(u16::from_le_bytes([head[0], head[1]])))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(ggml_type: i32, shape: Vec<i64>) -> TensorEntry {
        TensorEntry {
            name: "t".into(),
            shape,
            ggml_type,
            offset: 0,
            size_bytes: 0,
        }
    }

    #[test]
    fn f16_conversion_known_values() {
        assert_eq!(f16_to_f32(0x3C00), 1.0);
        assert_eq!(f16_to_f32(0x4000), 2.0);
        assert_eq!(f16_to_f32(0xBC00), -1.0);
        assert_eq!(f16_to_f32(0x0000), 0.0);
    }

    #[test]
    fn census_f32_zero_fraction() {
        let p = std::env::temp_dir().join("census_f32.bin");
        let vals: [f32; 8] = [0.0, 1.0, 0.0, 3.0, 0.0, 0.0, -2.0, 0.5];
        let mut bytes = Vec::new();
        for v in vals {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&p, &bytes).unwrap();
        let c = census_tensor(&p, &entry(0, vec![8])).unwrap();
        assert!(!c.unsupported);
        assert!((c.zero_fraction - 0.5).abs() < 1e-6);
        assert!((c.abs_mean - 0.8125).abs() < 1e-6);
        assert!((c.abs_max - 3.0).abs() < 1e-6);
        assert_eq!(c.sampled, 8);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn census_q8_0_rounds_to_int_values() {
        let p = std::env::temp_dir().join("census_q8.bin");
        // scale = 1.0 (f16 0x3C00), 32 int8 values.
        let mut b = vec![0x00, 0x3C];
        for i in 0i8..32 {
            b.push(i as u8);
        }
        std::fs::write(&p, &b).unwrap();
        let c = census_tensor(&p, &entry(8, vec![32])).unwrap();
        assert!(!c.unsupported);
        assert_eq!(c.sampled, 32);
        // values 0..31 => zero_fraction 1/32, abs_max 31.
        assert!((c.zero_fraction - 1.0 / 32.0).abs() < 1e-6);
        assert!((c.abs_max - 31.0).abs() < 1e-6);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn unsupported_type_reports_flag() {
        let p = std::env::temp_dir().join("census_unsup.bin");
        std::fs::write(&p, b"\0").unwrap();
        let c = census_tensor(&p, &entry(16, vec![1])).unwrap();
        assert!(c.unsupported);
        let _ = std::fs::remove_file(&p);
    }
}
