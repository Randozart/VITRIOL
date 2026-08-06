//! `nvidia-smi` querying and parsing.
//!
//! Runs the standard NVIDIA CLI in two queries (GPU summary + compute apps)
//! and parses the CSV output defensively — any malformed field degrades to a
//! zero/default rather than panicking, and a missing binary yields `None`.

use std::process::Command;

use crate::model::{GpuProcess, GpuSnapshot};

/// Fetch a full GPU snapshot, or `None` when `nvidia-smi` is unavailable.
pub fn query_gpu() -> Option<GpuSnapshot> {
    let summary = query_gpu_summary()?;
    Some(summary)
}

/// Query the single-GPU summary line. Returns `None` if `nvidia-smi` fails or
/// the output is empty.
fn query_gpu_summary() -> Option<GpuSnapshot> {
    let out = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.used,memory.total,utilization.gpu,temperature.gpu,power.draw,power.limit,clocks.sm,clocks.mem",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .to_string();
    let mut fields = line.split(',').map(|f| f.trim());
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

    let processes = query_processes();

    Some(GpuSnapshot {
        present: true,
        name,
        vram_used_mib,
        vram_total_mib,
        util_pct,
        temp_c,
        power_w,
        power_limit_w,
        sm_clock_mhz,
        mem_clock_mhz,
        processes,
    })
}

/// Query the compute-process list. Returns an empty vec on any failure.
fn query_processes() -> Vec<GpuProcess> {
    let out = match Command::new("nvidia-smi")
        .args([
            "--query-compute-apps=pid,used_memory,process_name",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .filter_map(|l| {
            let mut fields = l.split(',');
            let pid = fields.next()?.trim().parse::<u32>().ok()?;
            let vram_mib = fields
                .next()
                .and_then(|f| f.trim().parse::<u64>().ok())
                .unwrap_or(0);
            let name = fields.next().unwrap_or("").trim().to_string();
            Some(GpuProcess {
                pid,
                name,
                vram_mib,
            })
        })
        .collect()
}
