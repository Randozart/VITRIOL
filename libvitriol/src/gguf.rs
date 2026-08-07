//! GGUF v3 parser - extracts metadata and tensor sizes without storing KV values.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

pub struct ModelInfo {
    pub architecture: String,
    pub context_length: u64,
    pub block_count: u64,
    pub expert_count: u64,
    pub expert_used_count: u64,
    pub embedding_length: u64,
    pub head_count: u64,
    pub head_count_kv: u64,
    pub has_mtp: bool,
    pub total_size_bytes: u64,
    pub tensor_count: usize,
    pub per_layer_attn_bytes: u64,
    pub per_layer_experts_bytes: u64,
    /// The tensor catalog: one entry per weight (name, shape, type, offset).
    pub tensors: Vec<TensorEntry>,
}

/// One weight tensor in the catalog — the "row" of the weight database.
#[derive(Debug, Clone)]
pub struct TensorEntry {
    /// Fully qualified tensor name, e.g. `blk.12.ffn_gate.weight`.
    pub name: String,
    /// Logical shape (GGUF dims).
    pub shape: Vec<i64>,
    /// GGML type enum value.
    pub ggml_type: i32,
    /// Byte offset of the tensor payload in the file.
    pub offset: u64,
    /// Encoded payload size in bytes.
    pub size_bytes: u64,
}

/// The GGML type name for an enum value (falls back to `unknown-N`).
pub fn type_name(enum_val: i32) -> String {
    GGML_TYPE_TABLE
        .iter()
        .find(|t| t.enum_val == enum_val)
        .map(|t| t.name.to_string())
        .unwrap_or_else(|| format!("unknown-{enum_val}"))
}

pub struct GgmlTypeInfo {
    pub enum_val: i32,
    pub name: &'static str,
    pub blck_size: u64,
    pub type_size: u64,
}

pub const GGML_TYPE_TABLE: &[GgmlTypeInfo] = &[
    GgmlTypeInfo { enum_val: 0,  name: "f32",     blck_size: 1,   type_size: 4   },
    GgmlTypeInfo { enum_val: 1,  name: "f16",     blck_size: 1,   type_size: 2   },
    GgmlTypeInfo { enum_val: 2,  name: "q4_0",    blck_size: 32,  type_size: 18  },
    GgmlTypeInfo { enum_val: 3,  name: "q4_1",    blck_size: 32,  type_size: 20  },
    GgmlTypeInfo { enum_val: 6,  name: "q5_0",    blck_size: 32,  type_size: 22  },
    GgmlTypeInfo { enum_val: 7,  name: "q5_1",    blck_size: 32,  type_size: 24  },
    GgmlTypeInfo { enum_val: 8,  name: "q8_0",    blck_size: 32,  type_size: 34  },
    GgmlTypeInfo { enum_val: 10, name: "q2_K",    blck_size: 256, type_size: 40  },
    GgmlTypeInfo { enum_val: 11, name: "q3_K",    blck_size: 256, type_size: 44  },
    GgmlTypeInfo { enum_val: 12, name: "q4_K",    blck_size: 256, type_size: 144 },
    GgmlTypeInfo { enum_val: 13, name: "q5_K",    blck_size: 256, type_size: 176 },
    GgmlTypeInfo { enum_val: 14, name: "q6_K",    blck_size: 256, type_size: 210 },
    GgmlTypeInfo { enum_val: 16, name: "iq2_xxs", blck_size: 256, type_size: 66  },
    GgmlTypeInfo { enum_val: 17, name: "iq2_xs",  blck_size: 256, type_size: 74  },
    GgmlTypeInfo { enum_val: 18, name: "iq3_xxs", blck_size: 256, type_size: 98  },
    GgmlTypeInfo { enum_val: 19, name: "iq1_s",   blck_size: 256, type_size: 50  },
    GgmlTypeInfo { enum_val: 20, name: "iq4_nl",  blck_size: 32,  type_size: 18  },
    GgmlTypeInfo { enum_val: 21, name: "iq3_s",   blck_size: 256, type_size: 116 },
    GgmlTypeInfo { enum_val: 22, name: "iq2_s",   blck_size: 256, type_size: 82  },
    GgmlTypeInfo { enum_val: 23, name: "iq4_xs",  blck_size: 256, type_size: 148 },
    GgmlTypeInfo { enum_val: 29, name: "iq1_m",   blck_size: 256, type_size: 56  },
    GgmlTypeInfo { enum_val: 30, name: "bf16",    blck_size: 1,   type_size: 2   },
];

pub fn tensor_size_bytes(type_enum: i32, ne: &[i64]) -> u64 {
    if let Some(info) = GGML_TYPE_TABLE.iter().find(|t| t.enum_val == type_enum) {
        let mut total = info.type_size;
        if !ne.is_empty() {
            total = total * (ne[0] as u64) / info.blck_size;
        }
        for d in &ne[1..] { total *= *d as u64; }
        total
    } else {
        // Unknown type: estimate 2 bits per element (IQ2_M, IQ1_S, etc.)
        let mut total = 1u64;
        for &d in ne {
            total = total.saturating_mul(d as u64);
        }
        // 2 bits = 0.25 bytes per element, but we need to round up for safety
        (total + 3) / 4
    }
}

fn read_u32(f: &mut File) -> u32 { let mut b=[0u8;4]; f.read_exact(&mut b).ok(); u32::from_le_bytes(b) }
fn read_i32(f: &mut File) -> i32 { let mut b=[0u8;4]; f.read_exact(&mut b).ok(); i32::from_le_bytes(b) }
fn read_u64(f: &mut File) -> u64 { let mut b=[0u8;8]; f.read_exact(&mut b).ok(); u64::from_le_bytes(b) }
fn read_i64(f: &mut File) -> i64 { let mut b=[0u8;8]; f.read_exact(&mut b).ok(); i64::from_le_bytes(b) }
fn read_u8(f: &mut File) -> u8 { let mut b=[0u8;1]; f.read_exact(&mut b).ok(); b[0] }

fn read_str(f: &mut File) -> String {
    let len = read_u64(f) as usize;
    let mut buf = vec![0u8; len];
    if len > 0 { f.read_exact(&mut buf).ok(); }
    String::from_utf8_lossy(&buf).to_string()
}

fn skip_val(f: &mut File, tval: i32) {
    match tval {
        0 | 1 | 7 => { f.read_exact(&mut [0u8;1]).ok(); }
        2 | 3 => { f.read_exact(&mut [0u8;2]).ok(); }
        4 | 5 | 6 => { f.read_exact(&mut [0u8;4]).ok(); }
        10 | 11 | 12 => { f.read_exact(&mut [0u8;8]).ok(); }
        8 => { let len=read_u64(f)as usize; f.read_exact(&mut vec![0u8;len]).ok(); }
        9 => { let et=read_i32(f); let cnt=read_u64(f)as usize;
                for _ in 0..cnt { skip_val(f, et); } }
        _ => { f.read_exact(&mut [0u8;4]).ok(); }
    }
}

fn parse_layer(s: &str) -> Option<(u64, &str)> {
    let rest = s.strip_prefix("blk.")?;
    let dot = rest.find('.')?;
    let lidx: u64 = rest[..dot].parse().ok()?;
    Some((lidx, &rest[dot + 1..]))
}

pub fn read_gguf(path: &Path) -> anyhow::Result<ModelInfo> {
    use anyhow::Context;
    let mut f = File::open(path)
        .with_context(|| format!("cannot open: {}", path.display()))?;
    let mut magic = [0u8; 4];
    f.read_exact(&mut magic)?;
    if &magic != b"GGUF" { anyhow::bail!("not a GGUF file"); }
    let _version = read_u32(&mut f);
    let tensor_count = read_i64(&mut f) as u64;
    let kv_count = read_i64(&mut f) as u64;

    // Read all KVs but only store metadata we need
    let mut meta: HashMap<String, String> = HashMap::new();
    let store_keys: std::collections::HashSet<&str> = [
        "general.architecture", "block_count", "embedding_length",
        "context_length", "expert_count", "expert_used_count",
        "attention.head_count", "attention.head_count_kv",
        "nextn_predict_layers",
    ].iter().cloned().collect();

    let mut arch = "unknown".to_string();

    for _ in 0..kv_count {
        let key = read_str(&mut f);
        let tval = read_i32(&mut f);
        // Check if this key (or with arch prefix) is wanted
        let is_wanted = store_keys.contains(key.as_str())
            || key.starts_with("llama.")
            || key.starts_with(&arch)
            || key.contains("block_count")
            || key.contains("head_count");
        if is_wanted {
            let val = read_kv_as_str(&mut f, tval);
            meta.insert(key, val);
        } else {
            skip_val(&mut f, tval);
        }
        if let Some(v) = meta.get("general.architecture") {
            arch = v.clone();
        }
    }

    fn mv(meta: &HashMap<String,String>, key: &str, def: u64) -> u64 {
        meta.get(key).and_then(|v| v.parse().ok()).unwrap_or(def)
    }
    fn arch_mv(meta: &HashMap<String,String>, a:&str, k:&str, d:u64) -> u64 {
        for p in &[format!("{}.{}", a, k), format!("llama.{}", k)] {
            let v = mv(meta, p, u64::MAX);
            if v != u64::MAX { return v; }
        }
        d
    }

    let ctx_len = arch_mv(&meta, &arch, "context_length", 0);
    let bc = arch_mv(&meta, &arch, "block_count", 0);
    let ec = arch_mv(&meta, &arch, "expert_count", 0);
    let eu = arch_mv(&meta, &arch, "expert_used_count", 0);
    let el = arch_mv(&meta, &arch, "embedding_length", 0);
    let hc = arch_mv(&meta, &arch, "attention.head_count", 32);
    let hckv = arch_mv(&meta, &arch, "attention.head_count_kv", 8);
    let has_mtp_meta = meta.keys().any(|k| {
        let kl = k.to_lowercase();
        kl.contains("nextn_predict") || kl.contains("mtp")
    });

    // Parse tensors for per-layer sizes and build the catalog.
    let mut per_attn: HashMap<u64,u64> = HashMap::new();
    let mut per_exp: HashMap<u64,u64> = HashMap::new();
    let mut total_size: u64 = 0;
    let mut has_mtp_tensor = false;
    let mut tensors: Vec<TensorEntry> = Vec::with_capacity(tensor_count as usize);

    for _ in 0..tensor_count {
        let tname = read_str(&mut f);
        let nd = read_u32(&mut f);
        let mut dims = Vec::with_capacity(nd as usize);
        for _ in 0..nd { dims.push(read_i64(&mut f)); }
        let dtype = read_i32(&mut f);
        let off = read_u64(&mut f);
        let tsize = tensor_size_bytes(dtype, &dims);
        total_size += tsize;
        tensors.push(TensorEntry {
            name: tname.clone(),
            shape: dims.clone(),
            ggml_type: dtype,
            offset: off,
            size_bytes: tsize,
        });
        if let Some((lidx, sfx)) = parse_layer(&tname) {
            let sl = sfx.to_lowercase();
            if sl.contains("attn_qkv") || sl.contains("attn_output") {
                *per_attn.entry(lidx).or_insert(0) += tsize;
            } else if sl.contains("ffn_down_exps")
                || sl.contains("ffn_gate_exps")
                || sl.contains("ffn_up_exps")
            {
                *per_exp.entry(lidx).or_insert(0) += tsize;
            }
        }
        if tname.to_lowercase().contains("mtp") { has_mtp_tensor = true; }
    }

    let aa = if per_attn.is_empty() { 0 } else { per_attn.values().sum::<u64>() / per_attn.len() as u64 };
    let ae = if per_exp.is_empty() { 0 } else { per_exp.values().sum::<u64>() / per_exp.len() as u64 };

    Ok(ModelInfo {
        architecture: arch,
        context_length: ctx_len,
        block_count: bc,
        expert_count: ec,
        expert_used_count: eu,
        embedding_length: el,
        head_count: hc,
        head_count_kv: hckv,
        has_mtp: has_mtp_meta || has_mtp_tensor,
        total_size_bytes: total_size,
        tensor_count: tensor_count as usize,
        per_layer_attn_bytes: aa,
        per_layer_experts_bytes: ae,
        tensors,
    })
}

fn read_kv_as_str(f: &mut File, tval: i32) -> String {
    match tval {
        8 => read_str(f),
        0 | 1 | 7 => format!("{}", read_u8(f)),
        2 => format!("{}", { let mut b=[0u8;2]; f.read_exact(&mut b).ok(); u16::from_le_bytes(b) }),
        3 => format!("{}", { let mut b=[0u8;2]; f.read_exact(&mut b).ok(); i16::from_le_bytes(b) }),
        4 => format!("{}", read_u32(f)),
        5 => format!("{}", read_i32(f)),
        6 => format!("{}", { let mut b=[0u8;4]; f.read_exact(&mut b).ok(); f32::from_le_bytes(b) }),
        10 => format!("{}", read_u64(f)),
        11 => format!("{}", read_i64(f)),
        12 => format!("{}", { let mut b=[0u8;8]; f.read_exact(&mut b).ok(); f64::from_le_bytes(b) }),
        9 => { let et=read_i32(f); let cnt=read_u64(f)as usize;
                for _ in 0..cnt { skip_val(f, et); }
                format!("[{} elems]", cnt) }
        _ => "?".to_string(),
    }
}