//! Synthetic GGUF generator — shapes-real, weights-dummy.
//!
//! Writes a minimal-but-valid GGUF that carries the REAL model's shape
//! metadata (architecture string, embedding/heads/KV-heads, expert count
//! and used-count, context) so llama-bench exercises this hardware at the
//! real model's compute and KV math — without loading 11.5 GiB of real
//! weights (plan invariant I1: shapes-real, weights-dummy).
//!
//! Weight payloads are zero bytes in standard Q8_0 blocks (34 bytes per 32
//! elements, f16 scale + i8 values) — valid quantized tensors the kernels
//! dequantize happily. Honest limitation (labeled in probe output): the
//! expert-weight kernels time Q8_0 math, not the real IQ3_S layout; the
//! ubatch/KV/attention/split curves are the representative part.
//!
//! PROVENANCE: original work — GGUF v3 container written from the format
//! spec in the in-tree engine (gguf-py reference), reusing gguf.rs's
//! tensor-size table.

use crate::gguf::ModelInfo;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

pub const GGUF_MAGIC: &[u8; 4] = b"GGUF";
pub const GGUF_VERSION: u32 = 3;
pub const ALIGN: u64 = 32;
pub const T_F32: u32 = 0; // 4 bytes / element (GGML_TYPE_F32)
pub const T_F16: u32 = 1; // 2 bytes / element (GGML_TYPE_F16)
pub const T_Q8_0: u32 = 8; // 34 bytes / 32 elements (GGML_TYPE_Q8_0)
const GGML_TYPE_Q8_0_SIZE: u64 = 34;
const GGML_TYPE_Q8_0_BLK: u64 = 32;

/// Q8_0 requires ne[0] % 32 == 0; fall back to F16 otherwise (e.g. the
/// GQA K/V projections at n_embd_kv = 852). Caught live via llama-bench.
fn q_for(ne0: u64) -> u32 {
    if ne0 % GGML_TYPE_Q8_0_BLK == 0 {
        T_Q8_0
    } else {
        T_F16
    }
}

#[derive(Debug, Clone)]
pub struct SynthSpec {
    pub arch: String,
    pub name: String,
    pub layers: u64,
    pub n_embd: u64,
    pub n_head: u64,
    pub n_kv_head: u64,
    pub n_experts: u64,
    pub n_expert_used: u64,
    pub ffn: u64,
    pub vocab: u64,
    pub ctx: u64,
}

pub enum SynthScale {
    /// Seconds to load — full grid breadth.
    Tiny,
    /// ~1 GB — kernel behavior closer to the real model.
    Mid,
}

/// Real shapes from the target model's header; layer count and expert FFN
/// width scaled so the file loads in seconds (tiny) or ~1 GB (mid).
pub fn spec_from_model(mi: &ModelInfo, scale: SynthScale) -> SynthSpec {
    let real_ffn = mi.per_layer_experts_bytes.max(1);
    // Derive a plausible real FFN width: experts_bytes ≈ 3 tensors ×
    // ffn × n_embd × n_experts × (34/32). Solve for ffn conservatively.
    let est_ffn = if mi.embedding_length > 0 && mi.expert_count > 0 {
        let per_expert_bytes = real_ffn / mi.expert_count.max(1);
        // 3 tensors, each ~ffn*embd elements, Q8_0 → ffn ≈ bytes/(3*embd*34/32)
        let denom = (3 * mi.embedding_length * GGML_TYPE_Q8_0_SIZE / GGML_TYPE_Q8_0_BLK).max(1);
        per_expert_bytes / denom.max(1)
    } else {
        512
    };
    let est_ffn = est_ffn.clamp(64, 4096);
    // FLOP-matched dense FFN: real per-token expert compute =
    // n_expert_used × real ffn. The llama arch has no MoE routing —
    // routing overhead is NOT synthesized (documented in probe reports).
    let used = if mi.expert_used_count > 0 {
        mi.expert_used_count
    } else {
        8
    };
    let dense_ffn = (est_ffn * used).clamp(256, 16384);
    let (layers, ffn) = match scale {
        SynthScale::Tiny => (8u64, (dense_ffn / 4).max(256)),
        SynthScale::Mid => (24u64, dense_ffn),
    };
    // Self-consistent head layout: n_embd must divide by n_head (the real
    // hybrid has heterogeneous layer dims, so its raw head count does not
    // divide the embedding — prefer a 128 head dim like the real attention
    // layers, else the largest divisor ≤ 64).
    let embd = mi.embedding_length;
    let n_head = if embd % 128 == 0 && embd / 128 <= 128 {
        embd / 128
    } else {
        (1..=64u64).rev().find(|d| embd % d == 0).unwrap_or(1)
    };
    // head_dim = n_embd / n_head; n_embd_kv = head_dim × n_kv_head.
    // n_rot (rope.dimension_count) is head_dim — NOT the KV width.
    let head_dim = embd / n_head;
    let n_embd_kv = head_dim * mi.head_count_kv.max(1);
    SynthSpec {
        arch: "llama".to_string(),
        name: format!(
            "{}-synth-{}",
            mi.architecture,
            if matches!(scale, SynthScale::Tiny) {
                "tiny"
            } else {
                "mid"
            }
        ),
        layers,
        n_embd: embd,
        n_head,
        n_kv_head: mi.head_count_kv,
        n_experts: mi.expert_count,
        n_expert_used: mi.expert_used_count,
        ffn,
        vocab: 32000,
        ctx: mi.context_length.min(8192).max(4096),
    }
}

// ── GGUF writer primitives ────────────────────────────────────────────────

fn w_u32(f: &mut Vec<u8>, v: u32) {
    f.extend_from_slice(&v.to_le_bytes());
}
fn w_u64(f: &mut Vec<u8>, v: u64) {
    f.extend_from_slice(&v.to_le_bytes());
}
fn w_str(f: &mut Vec<u8>, s: &str) {
    w_u64(f, s.len() as u64);
    f.extend_from_slice(s.as_bytes());
}
fn kv_u32(f: &mut Vec<u8>, key: &str, v: u32) {
    w_str(f, key);
    w_u32(f, 4); // type u32
    w_u32(f, v);
}
fn kv_str(f: &mut Vec<u8>, key: &str, v: &str) {
    w_str(f, key);
    w_u32(f, 8); // type string
    w_str(f, v);
}
fn kv_arr_str(f: &mut Vec<u8>, key: &str, items: &[String]) {
    w_str(f, key);
    w_u32(f, 9); // array
    w_u32(f, 8); // of string
    w_u64(f, items.len() as u64);
    for s in items {
        w_str(f, s);
    }
}
fn kv_arr_i32(f: &mut Vec<u8>, key: &str, items: &[i32]) {
    w_str(f, key);
    w_u32(f, 9); // array
    w_u32(f, 5); // of i32
    w_u64(f, items.len() as u64);
    for v in items {
        f.extend_from_slice(&v.to_le_bytes());
    }
}
fn tensor_info(f: &mut Vec<u8>, name: &str, ne: &[u64], t: u32, off: u64) {
    w_str(f, name);
    w_u32(f, ne.len() as u32);
    for d in ne {
        w_u64(f, *d);
    }
    w_u32(f, t);
    w_u64(f, off);
}

/// Q8_0 byte size for n elements (ggml: ceil(n/32) blocks × 34 bytes).
fn q8_bytes(n: u64) -> u64 {
    n.div_ceil(GGML_TYPE_Q8_0_BLK) * GGML_TYPE_Q8_0_SIZE
}

fn vocab_tokens(vocab: u64) -> Vec<String> {
    let mut t = vec![
        "<|endoftext|>".to_string(),
        "<|startoftext|>".to_string(),
        "Hello".to_string(),
        "world".to_string(),
        "the".to_string(),
        "a".to_string(),
    ];
    for b in 0..256u64 {
        t.push(format!("<0x{b:02X}>"));
    }
    while (t.len() as u64) < vocab {
        t.push(format!("tok{}", t.len()));
    }
    t.truncate(vocab as usize);
    t
}

/// Write the synthetic GGUF. Returns bytes written.
pub fn write_synthetic(path: &Path, spec: &SynthSpec) -> Result<u64> {
    let head_dim = spec.n_embd / spec.n_head.max(1);
    let n_embd_kv = head_dim * spec.n_kv_head.max(1);

    // Tensor table (name, ne, type). Q8_0 only where ne[0] % 32 == 0 —
    // quantized tensors need block-aligned rows (F16 otherwise). Dense FFN
    // (the llama arch has no MoE tensors; FLOPs matched via dense_ffn).
    struct T(String, Vec<u64>, u32);
    let mut tensors: Vec<T> = Vec::new();
    tensors.push(T(
        "token_embd.weight".into(),
        vec![spec.n_embd, spec.vocab],
        T_F16,
    ));
    tensors.push(T("output_norm.weight".into(), vec![spec.n_embd], T_F32));
    tensors.push(T(
        "output.weight".into(),
        vec![spec.n_embd, spec.vocab],
        q_for(spec.n_embd),
    ));
    for i in 0..spec.layers {
        let p = format!("blk.{i}.");
        tensors.push(T(format!("{p}attn_norm.weight"), vec![spec.n_embd], T_F32));
        tensors.push(T(
            format!("{p}attn_q.weight"),
            vec![spec.n_embd, spec.n_embd],
            q_for(spec.n_embd),
        ));
        // k/v projections: ne = {n_embd, n_embd_kv} (llama.cpp order)
        tensors.push(T(
            format!("{p}attn_k.weight"),
            vec![spec.n_embd, n_embd_kv],
            q_for(n_embd_kv),
        ));
        tensors.push(T(
            format!("{p}attn_v.weight"),
            vec![spec.n_embd, n_embd_kv],
            q_for(n_embd_kv),
        ));
        tensors.push(T(
            format!("{p}attn_output.weight"),
            vec![spec.n_embd, spec.n_embd],
            q_for(spec.n_embd),
        ));
        tensors.push(T(format!("{p}ffn_norm.weight"), vec![spec.n_embd], T_F32));
        let moe = spec.n_experts > 0 && spec.n_expert_used > 0;
        if moe {
            tensors.push(T(
                format!("{p}ffn_gate_inp.weight"),
                vec![spec.n_experts, spec.n_embd],
                T_F32,
            ));
            tensors.push(T(
                format!("{p}ffn_gate_exps.weight"),
                vec![spec.ffn, spec.n_embd, spec.n_experts],
                q_for(spec.ffn),
            ));
            tensors.push(T(
                format!("{p}ffn_down_exps.weight"),
                vec![spec.n_embd, spec.ffn, spec.n_experts],
                q_for(spec.ffn),
            ));
            tensors.push(T(
                format!("{p}ffn_up_exps.weight"),
                vec![spec.ffn, spec.n_embd, spec.n_experts],
                q_for(spec.ffn),
            ));
        } else {
            tensors.push(T(
                format!("{p}ffn_gate.weight"),
                vec![spec.n_embd, spec.ffn],
                q_for(spec.ffn),
            ));
            tensors.push(T(
                format!("{p}ffn_down.weight"),
                vec![spec.ffn, spec.n_embd],
                q_for(spec.n_embd),
            ));
            tensors.push(T(
                format!("{p}ffn_up.weight"),
                vec![spec.n_embd, spec.ffn],
                q_for(spec.ffn),
            ));
        }
    }

    // Layout pass: KVs first (counted!), then the header, then tensor
    // infos + aligned data offsets. The earlier draft wrote tensor_count
    // and went STRAIGHT into the KV bytes — kv_count was missing, so every
    // reader consumed the first key's length prefix as kv_count and
    // misaligned into the key text (caught live 2026-11-13).
    let mut kv: Vec<u8> = Vec::new();
    let mut kv_count: u64 = 0;
    macro_rules! kv_u32_c {
        ($key:expr, $v:expr) => {{
            kv_u32(&mut kv, $key, $v);
            kv_count += 1;
        }};
    }
    macro_rules! kv_str_c {
        ($key:expr, $v:expr) => {{
            kv_str(&mut kv, $key, $v);
            kv_count += 1;
        }};
    }
    macro_rules! kv_f32_c {
        ($key:expr, $v:expr) => {{
            let v: f32 = $v;
            w_str(&mut kv, $key);
            w_u32(&mut kv, 6); // type f32
            kv.extend_from_slice(&v.to_le_bytes());
            kv_count += 1;
        }};
    }
    kv_str_c!("general.architecture", &spec.arch);
    kv_str_c!("general.name", &spec.name);
    kv_u32_c!("general.alignment", ALIGN as u32);
    kv_u32_c!("general.file_type", 1);
    kv_u32_c!(&format!("{}.context_length", spec.arch), spec.ctx as u32);
    kv_u32_c!(
        &format!("{}.embedding_length", spec.arch),
        spec.n_embd as u32
    );
    kv_u32_c!(&format!("{}.block_count", spec.arch), spec.layers as u32);
    kv_u32_c!(
        &format!("{}.feed_forward_length", spec.arch),
        spec.ffn as u32
    );
    kv_u32_c!(
        &format!("{}.attention.head_count", spec.arch),
        spec.n_head as u32
    );
    kv_u32_c!(
        &format!("{}.attention.head_count_kv", spec.arch),
        spec.n_kv_head as u32
    );
    kv_u32_c!(
        &format!("{}.expert_count", spec.arch),
        spec.n_experts as u32
    );
    kv_u32_c!(
        &format!("{}.expert_used_count", spec.arch),
        spec.n_expert_used as u32
    );
    kv_u32_c!(
        &format!("{}.expert_feed_forward_length", spec.arch),
        spec.ffn as u32
    );
    kv_f32_c!(
        &format!("{}.attention.layer_norm_rms_epsilon", spec.arch),
        1e-5
    );
    kv_f32_c!(&format!("{}.rope.theta", spec.arch), 1_000_000.0);
    kv_u32_c!(
        &format!("{}.rope.dimension_count", spec.arch),
        head_dim as u32
    );
    // qwen35-family partial-RoPE sections: 4× i32. Values do not affect
    // benchmark shapes or timing (probe weights are zeros — output math
    // quality is irrelevant); any loadable combination works.
    {
        w_str(&mut kv, &format!("{}.rope.dimension_sections", spec.arch));
        w_u32(&mut kv, 9); // array
        w_u32(&mut kv, 5); // of i32
        w_u64(&mut kv, 4);
        for v in [0i32; 4] {
            kv.extend_from_slice(&v.to_le_bytes());
        }
        kv_count += 1;
    }
    // MTP/nextn layer count — also how gguf.rs detects has_mtp.
    kv_u32_c!(&format!("{}.nextn_predict_layers", spec.arch), 1);
    let toks = vocab_tokens(spec.vocab);
    kv_str_c!("tokenizer.ggml.model", "gpt2");
    {
        w_str(&mut kv, "tokenizer.ggml.tokens");
        w_u32(&mut kv, 9); // array
        w_u32(&mut kv, 8); // of string
        w_u64(&mut kv, toks.len() as u64);
        for t in &toks {
            w_str(&mut kv, t);
        }
        kv_count += 1;
    }
    {
        w_str(&mut kv, "tokenizer.ggml.token_type");
        w_u32(&mut kv, 9); // array
        w_u32(&mut kv, 5); // of i32
        w_u64(&mut kv, toks.len() as u64);
        for _ in &toks {
            kv.extend_from_slice(&1i32.to_le_bytes());
        }
        kv_count += 1;
    }
    {
        w_str(&mut kv, "tokenizer.ggml.merges");
        w_u32(&mut kv, 9); // array
        w_u32(&mut kv, 8); // of string
        w_u64(&mut kv, 0);
        kv_count += 1;
    }

    // header = magic + version + tensor_count + KV_COUNT + kv
    let mut head: Vec<u8> = Vec::new();
    head.extend_from_slice(GGUF_MAGIC);
    w_u32(&mut head, GGUF_VERSION);
    w_u64(&mut head, tensors.len() as u64);
    w_u64(&mut head, kv_count);
    head.extend_from_slice(&kv);

    // tensor infos + data offsets (aligned)
    let mut info: Vec<u8> = Vec::new();
    let mut off: u64 = 0;
    for t in &tensors {
        let bytes = match t.2 {
            T_Q8_0 => q8_bytes(t.1.iter().product::<u64>()),
            T_F32 => t.1.iter().product::<u64>() * 4,
            T_F16 => t.1.iter().product::<u64>() * 2,
            _ => unreachable!(),
        };
        tensor_info(&mut info, &t.0, &t.1, t.2, off);
        off += (bytes + ALIGN - 1) / ALIGN * ALIGN;
    }
    head.extend_from_slice(&info);
    let data_start = head.len() as u64;
    let data_start = (data_start + ALIGN - 1) / ALIGN * ALIGN;

    // Emit: header+info, pad, then zero data per tensor (streamed).
    let mut file =
        std::fs::File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(&head)?;
    // Pad to the aligned data start. COUNT the bytes — re-checking
    // head.len() here is an infinite loop (the Vec never grows; caught live
    // 2026-11-13 as a 5 MB/s infinite one-byte write loop).
    let pad = data_start.saturating_sub(head.len() as u64);
    for _ in 0..pad {
        file.write_all(&[0])?;
    }
    let zero_block = vec![0u8; 1 << 20]; // 1 MiB zeros
    let mut remaining = off;
    let mut data_iters = 0u64;
    while remaining > 0 {
        let chunk = zero_block.len().min(remaining as usize);
        file.write_all(&zero_block[..chunk])?;
        remaining -= chunk as u64;
        data_iters += 1;
        if data_iters > 100_000 {
            break;
        }
    }
    file.flush()?;
    Ok(data_start + off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q8_math_and_vocab_fallback() {
        assert_eq!(q8_bytes(32), 34);
        assert_eq!(q8_bytes(33), 68);
        assert_eq!(q8_bytes(1), 34);
        let toks = vocab_tokens(280);
        assert_eq!(toks.len(), 280);
        assert!(toks.iter().any(|t| t == "<0xFF>"));
    }

    #[test]
    fn spec_from_model_scales_layers_not_shapes() {
        // Tiny model info (shapes are what must stay real)
        let mi = ModelInfo {
            architecture: "qwen38moe".into(),
            context_length: 262_144,
            block_count: 65,
            expert_count: 256,
            expert_used_count: 8,
            embedding_length: 2048,
            head_count: 16,
            head_count_kv: 4,
            has_mtp: true,
            total_size_bytes: 12_000_000_000,
            tensor_count: 0,
            per_layer_attn_bytes: 0,
            per_layer_experts_bytes: 3 * 1024 * 2048 * 256 * 34 / 32,
            tensors: Vec::new(),
        };
        let tiny = spec_from_model(&mi, SynthScale::Tiny);
        assert_eq!(tiny.n_embd, 2048);
        assert_eq!(tiny.n_experts, 256);
        assert_eq!(tiny.n_head, 16);
        assert_eq!(tiny.n_kv_head, 4);
        assert_eq!(tiny.layers, 8);
        let mid = spec_from_model(&mi, SynthScale::Mid);
        assert_eq!(mid.layers, 24);
        assert_eq!(mid.n_embd, tiny.n_embd, "shapes stay real");
    }
}

#[cfg(test)]
mod exec_tests {
    use super::*;

    #[test]
    fn write_executes_fast_and_reads_back() {
        let spec = SynthSpec {
            arch: "qwen35".into(),
            name: "synth-test".into(),
            layers: 2,
            n_embd: 256,
            n_head: 8,
            n_kv_head: 2,
            n_experts: 4,
            n_expert_used: 2,
            ffn: 128,
            vocab: 300,
            ctx: 4096,
        };
        let dir = std::env::temp_dir().join(format!("synth-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("synth.gguf");
        let t0 = std::time::Instant::now();
        let bytes = write_synthetic(&path, &spec).expect("writes");
        println!(
            "WRITE OK: {bytes} bytes in {:.2}s",
            t0.elapsed().as_secs_f64()
        );
        let dt = t0.elapsed().as_secs_f64();
        assert!(dt < 2.0, "write took {dt:.2}s — pad loop pathological");
        assert!(
            bytes > 1_000_000,
            "tiny spec should still be >1MB (MoE experts), got {bytes}"
        );
        println!("READ START");
        let mi = crate::gguf::read_gguf(&path).expect("reads back");
        println!("READ OK");
        assert_eq!(mi.architecture, "qwen35");
        assert_eq!(mi.block_count, 2);
        assert_eq!(mi.embedding_length, 256);
        assert_eq!(mi.expert_count, 4);
        assert_eq!(mi.head_count_kv, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
