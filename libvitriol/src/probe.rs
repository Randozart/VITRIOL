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

    let (name, vram, cc, pgen, pw) = nvidia_smi();
    let cpu = read_cpu();
    let avx2 = has_avx2();
    let ram = read_ram();
    let ipc = check_ipc();

    Ok(HardwareInfo {
        probed_at: ts,
        gpus: vec![GpuInfo {
            index: 0,
            name,
            vram_mib: vram,
            compute_cap: cc,
            pcie_gen: pgen,
            pcie_width: pw,
        }],
        cpu,
        has_avx2: avx2,
        ram_mib: ram,
        gpu_count: if vram > 0 { 1 } else { 0 },
        has_ipc_lock: ipc,
    })
}

fn nvidia_smi() -> (String, u64, String, u32, u32) {
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
    (name, vram, cc, pgen, pw)
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
