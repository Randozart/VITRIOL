use anyhow::Result;
use std::process::Command;

#[derive(Debug, Clone, serde::Serialize)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    pub vram_mib: u64,
    pub compute_cap: String,
    pub pcie_gen: u32,
    pub pcie_width: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareInfo {
    pub probed_at: String,
    pub gpus: Vec<GpuInfo>,
    pub cpu: String,
    pub has_avx2: bool,
    pub ram_mib: u64,
    pub gpu_count: u32,
    pub has_ipc_lock: bool,
}

pub fn probe_hardware() -> Result<HardwareInfo> {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_default();

    let gpus = nvidia_smi_gpus();
    let cpu = read_cpu();
    let avx2 = has_avx2();
    let ram = read_ram();
    let ipc = check_ipc();

    Ok(HardwareInfo {
        probed_at: ts,
        gpus: gpus.clone(),
        cpu,
        has_avx2: avx2,
        ram_mib: ram,
        gpu_count: gpus.len() as u32,
        has_ipc_lock: ipc,
    })
}

/* Enumerate every GPU with one nvidia-smi call. Field order (CSV, one GPU
 * per line, header off): index,name,memory.total,compute_cap,
 * pcie.link.gen.current,pcie.link.width.current.
 * Names contain spaces, so rows split on commas — never on whitespace. */
fn nvidia_smi_gpus() -> Vec<GpuInfo> {
    let mut gpus = Vec::new();
    let out = cmd(
        "nvidia-smi",
        &[
            "--query-gpu=index,name,memory.total,compute_cap,pcie.link.gen.current,pcie.link.width.current",
            "--format=csv,noheader",
        ],
    );
    for line in out.lines() {
        let fields: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if fields.len() < 6 {
            continue;
        }
        let idx = fields[0].parse::<u32>().unwrap_or(gpus.len() as u32);
        let vram = fields[2]
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        if vram == 0 {
            continue;
        }
        gpus.push(GpuInfo {
            index: idx,
            name: fields[1].to_string(),
            vram_mib: vram,
            compute_cap: fields[3].to_string(),
            pcie_gen: fields[4].parse::<u32>().unwrap_or(0),
            pcie_width: fields[5].parse::<u32>().unwrap_or(0),
        });
    }
    gpus.sort_by_key(|g| g.index);
    if gpus.is_empty() {
        /* Fallback: mirror the old single-GPU probe in case the CSV layout differs */
        let name = cmd("nvidia-smi", &["--query-gpu=name", "--format=csv,noheader"]);
        let vram = cmd(
            "nvidia-smi",
            &["--query-gpu=memory.total", "--format=csv,noheader"],
        )
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
        let cc = cmd(
            "nvidia-smi",
            &["--query-gpu=compute_cap", "--format=csv,noheader"],
        );
        let pgen = cmd(
            "nvidia-smi",
            &["--query-gpu=pcie.link.gen.current", "--format=csv,noheader"],
        )
        .parse::<u32>()
        .unwrap_or(0);
        let pw = cmd(
            "nvidia-smi",
            &[
                "--query-gpu=pcie.link.width.current",
                "--format=csv,noheader",
            ],
        )
        .parse::<u32>()
        .unwrap_or(0);
        if vram > 0 {
            gpus.push(GpuInfo {
                index: 0,
                name,
                vram_mib: vram,
                compute_cap: cc,
                pcie_gen: pgen,
                pcie_width: pw,
            });
        }
    }
    gpus
}

fn cmd(prog: &str, args: &[&str]) -> String {
    Command::new(prog)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn read_cpu() -> String {
    if let Ok(data) = std::fs::read_to_string("/proc/cpuinfo") {
        for line in data.lines() {
            if line.starts_with("model name") {
                if let Some(val) = line.split(':').nth(1) {
                    return val.trim().to_string();
                }
            }
        }
    }
    String::new()
}

fn has_avx2() -> bool {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .map(|s| s.contains("avx2"))
        .unwrap_or(false)
}

fn read_ram() -> u64 {
    if let Ok(data) = std::fs::read_to_string("/proc/meminfo") {
        for line in data.lines() {
            if line.starts_with("MemTotal") {
                if let Some(val) = line.split_whitespace().nth(1) {
                    if let Ok(kb) = val.parse::<u64>() {
                        return kb / 1024;
                    }
                }
            }
        }
    }
    0
}

fn check_ipc() -> bool {
    for path in &["/usr/local/bin/llama-server", "/usr/bin/llama-server"] {
        if let Ok(o) = Command::new("getcap").arg(path).output() {
            if String::from_utf8_lossy(&o.stdout).contains("cap_ipc_lock") {
                return true;
            }
        }
    }
    false
}
