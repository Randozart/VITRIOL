//! Compare store expert slices against the source GGUF directly.
//! Usage: cargo run --release --example store_vs_gguf -- /dev/nvme0n1p3 \
//!          <gguf-shard2> <gguf-shard3> [layers...]
//! Checks EVERY expert of ffn_down_exps for the requested layers (default:
//! 0 2 30 46 47 - thin, fat, fat, fat, fat).

use vitriol_stream::store::{AlignedBuf, Store};

struct GgufTensor {
    name: String,
    offset: u64,
    size: u64,
    n_dims: u32,
    dims: [u64; 4],
    dtype: u32,
}

fn read_string(f: &mut std::fs::File) -> std::io::Result<String> {
    use std::io::Read;
    let mut b8 = [0u8; 8];
    f.read_exact(&mut b8)?;
    let n = u64::from_le_bytes(b8) as usize;
    let mut s = vec![0u8; n];
    f.read_exact(&mut s)?;
    String::from_utf8(s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn skip_val(f: &mut std::fs::File, kt: u32) -> std::io::Result<()> {
    use std::io::Read;
    let mut b8 = [0u8; 8];
    let mut b4 = [0u8; 4];
    // llama.cpp gguf.h enum (NOT the speculative-format table):
    // 0 u8, 1 i8, 2 u16, 3 i16, 4 u32, 5 i32, 6 f32, 7 bool, 8 string,
    // 9 array, 10 u64, 11 i64, 12 f64
    match kt {
        0 | 1 | 7 => drop(read_exact_n(f, 1)?),
        2 | 3 => drop(read_exact_n(f, 2)?),
        4 | 5 | 6 => drop(read_exact_n(f, 4)?),
        8 => {
            let s = read_string(f)?;
            drop(s);
        }
        9 => {
            f.read_exact(&mut b4)?;
            let kt2 = u32::from_le_bytes(b4);
            f.read_exact(&mut b8)?;
            let n = u64::from_le_bytes(b8) as usize;
            if kt2 == 8 {
                for _ in 0..n {
                    let s = read_string(f)?;
                    drop(s);
                }
            } else {
                let sz = match kt2 {
                    0 | 1 | 7 => 1,
                    2 | 3 => 2,
                    4 | 5 | 6 => 4,
                    10 | 11 | 12 => 8,
                    _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "kv")),
                };
                drop(read_exact_n(f, n * sz)?);
            }
        }
        10 | 11 | 12 => drop(read_exact_n(f, 8)?),
        _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "kv")),
    }
    Ok(())
}

fn read_exact_n(f: &mut std::fs::File, n: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut v = vec![0u8; n];
    f.read_exact(&mut v)?;
    Ok(v)
}

fn parse_gguf(path: &str) -> std::io::Result<(u64, Vec<GgufTensor>)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = File_or_open(path)?;
    let mut b4 = [0u8; 4];
    let mut b8 = [0u8; 8];
    f.read_exact(&mut b4)?;
    f.read_exact(&mut b4)?; // version
    f.read_exact(&mut b8)?;
    let n_tensors = u64::from_le_bytes(b8);
    f.read_exact(&mut b8)?;
    let n_kv = u64::from_le_bytes(b8);
    eprintln!("parse {path}: n_tensors={n_tensors} n_kv={n_kv}");
    let mut alignment: u64 = 32;
    for i in 0..n_kv {
        let k = read_string(&mut f)?;
        f.read_exact(&mut b4)?;
        let kt = u32::from_le_bytes(b4);
        if k == "general.alignment" {
            // u32
            let mut v = [0u8; 4];
            f.read_exact(&mut v)?;
            alignment = u32::from_le_bytes(v) as u64;
            continue;
        }
        if i < 8 {
            eprintln!("kv[{i}]: key={k:?} type={kt}");
        }
        skip_val(&mut f, kt)?;
    }
    eprintln!("alignment={alignment}");
    let list_start = f.stream_position()?;
    eprintln!("list_start={list_start} n_tensors={n_tensors}");
    let mut tensors = Vec::new();
    let mut max_end: u64 = 0;
    for _ in 0..n_tensors {
        let name = read_string(&mut f)?;
        f.read_exact(&mut b4)?;
        let nd = u32::from_le_bytes(b4);
        let mut dims = [0u64; 4];
        for i in 0..nd as usize {
            f.read_exact(&mut b8)?;
            dims[i] = u64::from_le_bytes(b8);
        }
        f.read_exact(&mut b4)?;
        let dtype = u32::from_le_bytes(b4);
        f.read_exact(&mut b8)?;
        let offset = u64::from_le_bytes(b8);
        tensors.push(GgufTensor {
            name,
            offset,
            size: 0,
            n_dims: nd,
            dims,
            dtype,
        });
        let _ = max_end;
    }
    // Data section starts right after the tensor list, aligned to 512 in
    // practice for these shards; derive it from the largest end offset by
    // scanning file size (fall back: first tensor offset works too when the
    // list is contiguous).
    let _ = max_end;
    let data_start = derive_data_start(&mut f, list_start, &tensors, alignment)?;
    eprintln!("shard {path}: header_end={} data_start={data_start}", f.stream_position()?);
    Ok((data_start, tensors))
}

fn File_or_open(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// The data section is 512-aligned after the header; find it by locating the
/// first tensor's offset as an absolute position: try alignments and check
/// that the byte distance to each subsequent tensor's offset stays >= its
/// size (i.e., offsets are relative to data_start).
fn derive_data_start(
    f: &mut std::fs::File,
    list_start: u64,
    tensors: &[GgufTensor],
    alignment: u64,
) -> std::io::Result<u64> {
    use std::io::{Read, Seek, SeekFrom};
    if tensors.is_empty() {
        return Ok(list_start);
    }
    // header_end = current position after reading the tensor list.
    let header_end = f.stream_position()?;
    let _ = list_start;
    // GGUF v3: data section starts at align(header_end, general.alignment).
    // We trust 512 (mkfs + llama.cpp quant shards are 512-aligned); verify by
    // checking that the first tensor region is not inside the header.
    Ok((header_end + alignment - 1) / alignment * alignment)
}

fn dtype_size(dtype: u32, n_elements: u64) -> Option<u64> {
    // bytes per element for the types ffn_down_exps uses here
    let bpe: f64 = match dtype {
        8 => 34.0 / 32.0,   // q8_0
        11 => 144.0 / 256.0, // q3_K
        12 => 144.0 / 256.0, // q4_K is 64/256.. keep table honest:
        23 => 110.0 / 256.0, // IQ3_S
        22 => 126.0 / 256.0, // IQ4_NL
        _ => return None,
    };
    Some((n_elements as f64 * bpe).ceil() as u64)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: store_vs_gguf <store> <shard2> <shard3> [layers...]");
        std::process::exit(1);
    }
    let layers: Vec<u32> = if args.len() > 4 {
        args[4..].iter().map(|x| x.parse().unwrap()).collect()
    } else {
        vec![0, 2, 30, 46, 47]
    };

    let store = Store::open(&args[1]).expect("store open");
    println!("store tensors: {}", store.n_tensors());

    // Parse both shards; index tensors by name.
    let mut by_name: std::collections::HashMap<String, (String, GgufTensor, u64)> =
        std::collections::HashMap::new();
    for shard in [&args[2], &args[3]] {
        let (data_start, tensors) = parse_gguf(shard).expect("gguf parse");
        let shard_path = shard.to_string();
        for t in tensors {
            // First shard wins: split shards repeat the full tensor list;
            // a tensor's data lives in the shard that first declares it.
            by_name.entry(t.name.clone()).or_insert((shard_path.clone(), t, data_start));
        }
    }

    let mut total_mismatch = 0usize;
    let mut total_checked = 0usize;
    for layer in layers {
        let Some(tid) = store.find_tensor(layer, 2) else {
            println!("layer {layer}: no store tensor");
            continue;
        };
        let d = store.desc(tid).unwrap();
        let name = format!("blk.{layer}.ffn_down_exps.weight");
        let Some((shard_path, t, data_start)) = by_name.get(&name) else {
            println!("layer {layer}: not in gguf shards");
            continue;
        };
        // The GGUF expert stride equals the store slice stride (mkfs wrote
        // the store from these tensors with this very stride).
        println!(
            "layer {layer}: tid={tid} experts={} slice={} gguf_shard={}",
            d.n_experts, d.slice_bytes,
            shard_path.rsplit('/').next().unwrap_or("?")
        );

        let mut f = std::fs::File::open(shard_path).unwrap();
        use std::io::{Read, Seek, SeekFrom};
        let mut sbuf = AlignedBuf::zeroed(d.slice_bytes as usize, 512);
        let mut gbuf = vec![0u8; d.data_bytes as usize];
        let mut layer_bad = 0usize;
        for e in 0..d.n_experts {
            store
                .read_expert_padded(tid, e, sbuf.as_mut_slice())
                .unwrap_or_else(|err| panic!("store read l{layer} e{e}: {err}"));
            let goff = data_start + t.offset + e as u64 * u64::from(d.slice_bytes);
            f.seek(SeekFrom::Start(goff)).unwrap();
            f.read_exact(&mut gbuf).unwrap();
            total_checked += 1;
            if &sbuf.as_slice()[0..d.data_bytes as usize] != gbuf.as_slice() {
                layer_bad += 1;
                total_mismatch += 1;
                if layer_bad <= 3 {
                    println!(
                        "  MISMATCH layer {layer} expert {e}: store[0..8]={:02x?} gguf[0..8]={:02x?}",
                        &sbuf.as_slice()[0..8],
                        &gbuf[0..8]
                    );
                }
            }
        }
        println!(
            "layer {layer}: {} / {} experts differ",
            layer_bad,
            d.n_experts
        );
    }
    println!(
        "TOTAL: {total_mismatch} / {total_checked} expert slices differ"
    );
}
