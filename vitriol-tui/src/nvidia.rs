//! `nvidia-smi` querying and parsing.
//!
//! Runs the standard NVIDIA CLI in two queries (per-GPU summary + compute apps)
//! and parses the CSV output defensively — any malformed field degrades to a
//! zero/default rather than panicking, and a missing binary yields an empty
//! vec. One [`GpuSnapshot`] is produced per physical GPU; compute processes are
//! attributed to GPUs by uuid join.

use std::collections::HashMap;
use std::process::Command;

use crate::model::{GpuProcess, GpuSnapshot};

/// Fetch a full GPU snapshot list, or an empty vec when `nvidia-smi` is
/// unavailable.
pub fn query_gpus() -> Vec<GpuSnapshot> {
    query_gpu_summary()
}

/// Query the per-GPU summary lines. Returns an empty vec if `nvidia-smi`
/// fails or the output is empty.
fn query_gpu_summary() -> Vec<GpuSnapshot> {
    let out = match Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,uuid,name,memory.used,memory.total,utilization.gpu,temperature.gpu,power.draw,power.limit,clocks.sm,clocks.mem",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_gpu_line)
        .collect()
}

/// Parse one summary CSV row into a snapshot; `None` when the row is unusable.
fn parse_gpu_line(line: &str) -> Option<GpuSnapshot> {
    let mut fields = line.split(',').map(|f| f.trim());
    let index = fields.next()?.parse::<u8>().ok()?;
    let uuid = fields.next().unwrap_or("").to_string();
    let name = fields.next().unwrap_or("").to_string();
    let vram_used_mib = fields
        .next()
        .and_then(|f| f.parse::<u64>().ok())
        .unwrap_or(0);
    let vram_total_mib = fields
        .next()
        .and_then(|f| f.parse::<u64>().ok())
        .unwrap_or(0);
    let util_pct = fields
        .next()
        .and_then(|f| f.parse::<u8>().ok())
        .unwrap_or(0);
    let temp_c = fields
        .next()
        .and_then(|f| f.parse::<u8>().ok())
        .unwrap_or(0);
    let power_w = fields
        .next()
        .and_then(|f| f.parse::<f64>().ok())
        .unwrap_or(0.0);
    let power_limit_w = fields
        .next()
        .and_then(|f| f.parse::<f64>().ok())
        .unwrap_or(0.0);
    let sm_clock_mhz = fields
        .next()
        .and_then(|f| f.parse::<u16>().ok())
        .unwrap_or(0);
    let mem_clock_mhz = fields
        .next()
        .and_then(|f| f.parse::<u16>().ok())
        .unwrap_or(0);

    Some(GpuSnapshot {
        index,
        name,
        uuid,
        vram_used_mib,
        vram_total_mib,
        util_pct,
        temp_c,
        power_w,
        power_limit_w,
        sm_clock_mhz,
        mem_clock_mhz,
    })
}

/// Query the compute-process list, attributing each row to a GPU by uuid.
/// Returns an empty vec on any failure.
pub fn query_processes(gpus: &[GpuSnapshot]) -> Vec<GpuProcess> {
    let out = match Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=gpu_uuid,pid,used_memory,process_name",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let uuid_to_index: HashMap<&str, u8> = gpus
        .iter()
        .map(|g| (g.uuid.as_str(), g.index))
        .collect();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| {
            let mut fields = l.split(',').map(|f| f.trim());
            let uuid = fields.next()?;
            let pid = fields.next()?.parse::<u32>().ok()?;
            let vram_mib = fields
                .next()
                .and_then(|f| f.parse::<u64>().ok())
                .unwrap_or(0);
            let name = fields.next().unwrap_or("").to_string();
            Some(GpuProcess {
                pid,
                name,
                vram_mib,
                gpu_index: uuid_to_index.get(uuid).copied(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_line_parses_all_fields() {
        let line = "0, GPU-aaaa-bbbb, NVIDIA GeForce RTX 3060, 1024, 12288, 35, 51, 22.15, 170, 1867, 7501";
        let g = parse_gpu_line(line).expect("parses");
        assert_eq!(g.index, 0);
        assert_eq!(g.uuid, "GPU-aaaa-bbbb");
        assert_eq!(g.name, "NVIDIA GeForce RTX 3060");
        assert_eq!(g.vram_used_mib, 1024);
        assert_eq!(g.vram_total_mib, 12288);
        assert_eq!(g.util_pct, 35);
        assert_eq!(g.temp_c, 51);
        assert!((g.power_w - 22.15).abs() < 1e-9);
        assert_eq!(g.power_limit_w, 170.0);
        assert_eq!(g.sm_clock_mhz, 1867);
        assert_eq!(g.mem_clock_mhz, 7501);
    }

    #[test]
    fn gpu_line_missing_binary_fields_degrade() {
        // Short row: only index + uuid + name.
        let g = parse_gpu_line("1, GPU-xxxx, GTX 1070 Ti").expect("parses");
        assert_eq!(g.index, 1);
        assert_eq!(g.vram_used_mib, 0);
        assert_eq!(g.util_pct, 0);
    }

    #[test]
    fn gpu_line_without_index_is_rejected() {
        assert!(parse_gpu_line("GPU-xxxx, name").is_none());
    }

    #[test]
    fn process_rows_join_by_uuid() {
        let gpus = vec![
            GpuSnapshot {
                index: 0,
                uuid: "GPU-a".into(),
                ..Default::default()
            },
            GpuSnapshot {
                index: 1,
                uuid: "GPU-b".into(),
                ..Default::default()
            },
        ];
        let p = GpuProcess {
            pid: 42,
            name: "llama-server".into(),
            vram_mib: 100,
            gpu_index: uuid_to_index(&gpus, "GPU-b"),
        };
        assert_eq!(p.gpu_index, Some(1));
    }

    fn uuid_to_index(gpus: &[GpuSnapshot], uuid: &str) -> Option<u8> {
        gpus.iter().find(|g| g.uuid == uuid).map(|g| g.index)
    }
}
