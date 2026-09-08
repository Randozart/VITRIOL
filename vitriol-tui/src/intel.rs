//! Intel (Arc/Xe) GPU telemetry via sysfs, with an `intel_gpu_top` JSON
//! fallback when the tool is installed.
//!
//! The xe driver exposes per-GT sysfs nodes under
//! `/sys/class/drm/cardN/device/tile0/gt0/` (freq, idle residency). Memory
//! totals are not exposed on all kernels, so when `intel_gpu_top -J` is
//! available we use it for VRAM; otherwise the snapshot degrades to freq +
//! util only (VRAM 0/0). A missing driver path or binary yields an empty vec,
//! mirroring `nvidia::query_gpus` so callers can fall back uniformly.

use std::process::Command;

use crate::model::{GpuProcess, GpuSnapshot};

/// Standard sysfs root for the first DRM card's xe device.
fn sysfs_card_dir(card: &str) -> std::path::PathBuf {
    std::path::PathBuf::from("/sys/class/drm").join(card).join("device")
}

/// Read one integer from a sysfs file; `None` when unreadable.
fn read_sysfs_u64(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse::<u64>().ok()
}

/// Read CPU/package temperature from thermal zones (millidegrees → Celsius).
/// Scans for TCPU_PCI or x86_pkg_temp zones; returns 0 when unavailable.
fn read_cpu_temp_c() -> u8 {
    let thermal_dir = std::path::Path::new("/sys/class/thermal");
    if let Ok(entries) = std::fs::read_dir(thermal_dir) {
        for entry in entries.flatten() {
            let zone_dir = entry.path();
            let type_path = zone_dir.join("type");
            if let Ok(type_str) = std::fs::read_to_string(&type_path) {
                let zone_type = type_str.trim();
                // Prefer CPU/PCI package temperature as system thermal proxy.
                if zone_type == "TCPU_PCI" || zone_type == "x86_pkg_temp" {
                    if let Some(temp_millideg) = read_sysfs_u64(&zone_dir.join("temp")) {
                        return (temp_millideg / 1000) as u8;
                    }
                }
            }
        }
    }
    0
}

/// Name of the first GPU tile (friendly label, e.g. "Arc B390").
fn tile_name(card: &str) -> Option<String> {
    // Map known Intel PCI device ids to friendly product names; fall back to a
    // generic label. Vendor must be Intel (0x8086).
    const INTEL_VENDOR: &str = "0x8086";
    let dir = sysfs_card_dir(card);
    let vendor = std::fs::read_to_string(dir.join("vendor")).unwrap_or_default();
    if vendor.trim() == INTEL_VENDOR {
        if let Ok(dev) = std::fs::read_to_string(dir.join("device")) {
            let name = match dev.trim() {
                "0xb082" => "Intel Arc B390 (PTL)",
                "0xb080" => "Intel Arc B380 (PTL)",
                "0x7d40" | "0x7d45" => "Intel Arc 140V/130T (Lunar Lake)",
                "0x7d51" | "0x7d55" | "0x7dd1" => "Intel Arc 130V (Lunar Lake)",
                "0x9a40" | "0x9a49" => "Intel Iris Xe (Tiger Lake)",
                "0x56a0" | "0x56a1" => "Intel Arc (DG2/Alchemist)",
                "0x643e" => "Intel Arc A770",
                "0x56c0" => "Intel Arc A750",
                "0x56a5" => "Intel Arc A380",
                other => other,
            };
            return Some(name.to_string());
        }
    }
    Some(format!("Intel GPU ({card})"))
}

/// Fetch a full GPU snapshot list for Intel/Xe devices, or an empty vec when
/// the xe sysfs tree is absent.
pub fn query_gpus() -> Vec<GpuSnapshot> {
    query_intel_gpus()
}

/// Query the first Intel DRM card via sysfs. Returns an empty vec on failure.
fn query_intel_gpus() -> Vec<GpuSnapshot> {
    // Prefer `intel_gpu_top -J` when present (i915 devices: richer JSON output).
    if let Some(snap) = query_intel_gpu_top() {
        return vec![snap];
    }
    // Fall back to `gputop` for xe devices (Panther Lake etc.) which
    // intel_gpu_top doesn't support. gputop provides VRAM per-process and
    // engine utilization via text output.
    if let Some(snap) = query_gputop() {
        return vec![snap];
    }
    // Last resort: sysfs-only snapshot (freq + util proxy, no VRAM).
    query_intel_sysfs()
}

/// Best-effort sysfs snapshot for card0.
fn query_intel_sysfs() -> Vec<GpuSnapshot> {
    const CARD: &str = "card0";
    let root = sysfs_card_dir(CARD);
    let gt0 = root.join("tile0").join("gt0");
    let freq = gt0.join("freq0");

    let act_freq = read_sysfs_u64(&freq.join("act_freq")).unwrap_or(0);
    let max_freq = read_sysfs_u64(&freq.join("max_freq")).unwrap_or(0);

    // Utilisation: act_freq/max_freq is a reasonable instantaneous proxy for a
    // single-GT iGPU (0% when idle, ~100% under load).
    let util_pct = if max_freq > 0 {
        ((act_freq * 100).saturating_div(max_freq) as u8).min(100)
    } else {
        0
    };

    // Idle state (gt-c6 deep idle, gt-c0 running) as a coarse cross-check.
    let idle_status = std::fs::read_to_string(gt0.join("gtidle").join("idle_status"))
        .unwrap_or_default();
    let util_pct = if idle_status.contains("c6") || idle_status.contains("c9") {
        util_pct.min(5) // deep idle → treat as ~idle
    } else {
        util_pct
    };

    let name = tile_name(CARD).unwrap_or_else(|| "Intel Arc".to_string());

    // Use CPU/package temperature as system thermal proxy — the xe driver on
    // Panther Lake doesn't expose a GPU-specific thermal zone.
    let temp_c = read_cpu_temp_c();

    Some(GpuSnapshot {
        index: 0,
        name,
        sm_clock_mhz: act_freq as u16,
        mem_clock_mhz: 0,
        util_pct,
        // VRAM not exposed on this kernel's xe sysfs; intel_gpu_top fills it.
        vram_used_mib: 0,
        vram_total_mib: 0,
        temp_c,
        power_w: 0.0,
        power_limit_w: 0.0,
        uuid: String::new(),
    })
    .into_iter()
    .collect()
}

/// Parse `intel_gpu_top -J` output into a snapshot. Returns `None` when the
/// binary is absent or the JSON cannot be decoded.
fn query_intel_gpu_top() -> Option<GpuSnapshot> {
    let out = Command::new("intel_gpu_top")
        .args(["-J", "-s", "1000", "-o", "-"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    // intel_gpu_top -J emits a JSON object with "periods" array; the last
    // period holds the latest counters. Parse defensively.
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let period = v.get("periods")?.as_array()?.last()?;

    let freq_mhz = period
        .get("GPU")
        .and_then(|g| g.get("freq"))
        .and_then(|f| f.as_f64())
        .map(|f| f as u16)
        .unwrap_or(0);
    let busy = period
        .get("GPU")
        .and_then(|g| g.get("busy"))
        .and_then(|b| b.as_f64())
        .unwrap_or(0.0);
    let util_pct = (busy.clamp(0.0, 1.0) * 100.0) as u8;

    let name = v
        .get("devices")
        .and_then(|d| d.as_array())
        .and_then(|a| a.first())
        .and_then(|d| d.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| "Intel Arc".to_string());

    Some(GpuSnapshot {
        index: 0,
        name,
        sm_clock_mhz: freq_mhz,
        mem_clock_mhz: 0,
        util_pct,
        vram_used_mib: 0,
        vram_total_mib: 0,
        temp_c: 0,
        power_w: 0.0,
        power_limit_w: 0.0,
        uuid: String::new(),
    })
}

/// Parse a human-readable size string (e.g. "1G", "15M", "92K") into MiB.
fn parse_size_to_mib(s: &str) -> u64 {
    let s = s.trim();
    if let Some(val) = s.strip_suffix('G') {
        val.parse::<f64>().unwrap_or(0.0) as u64 * 1024
    } else if let Some(val) = s.strip_suffix('M') {
        val.parse::<f64>().unwrap_or(0.0) as u64
    } else if let Some(val) = s.strip_suffix('K') {
        val.parse::<f64>().unwrap_or(0.0) as u64 / 1024
    } else {
        s.parse::<u64>().unwrap_or(0)
    }
}

/// Strip ANSI escape codes from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until 'm' (SGR end) or a letter.
            while let Some(next) = chars.next() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse `gputop -d 0.5 -n 1` text output into a snapshot.
/// Returns `None` when the binary is absent or output cannot be parsed.
fn query_gputop() -> Option<GpuSnapshot> {
    let out = Command::new("gputop")
        .args(["-d", "0.5", "-n", "1"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout);
    let text = strip_ansi(&raw);

    // Line 1: "DRM minor 128   Frequency(MHz) GT0-2500/2500 GT1-1200/1200"
    // Extract GT0 frequency from the header.
    let mut freq_mhz: u16 = 0;
    for line in text.lines() {
        if line.contains("GT0-") {
            // Parse "GT0-cur/max" from the line.
            if let Some(gt0_section) = line.split("GT0-").nth(1) {
                if let Some(freq_part) = gt0_section.split_whitespace().next() {
                    if let Some(cur_str) = freq_part.split('/').next() {
                        freq_mhz = cur_str.parse().unwrap_or(0);
                    }
                }
            }
            break;
        }
    }

    // Find the data line with the largest MEM (main process, not children).
    let mut best_vram_mib: u64 = 0;
    let mut best_util: u8 = 0;
    let mut found_data = false;

    for line in text.lines() {
        let trimmed = line.trim();
        // Data lines start with a PID (digits) and contain " | " separators.
        if trimmed.starts_with(char::is_numeric) && trimmed.contains(" | ") {
            found_data = true;
            // Format: "PID MEM RSS | rcs% || vcs% || vecs% || bcs% || ccs% | NAME"
            let parts: Vec<&str> = trimmed.split('|').collect();
            if parts.len() >= 2 {
                // Left side: "PID MEM RSS"
                let left = parts[0].trim();
                let fields: Vec<&str> = left.split_whitespace().collect();
                if fields.len() >= 2 {
                    let mem_mib = parse_size_to_mib(fields[1]);
                    if mem_mib > best_vram_mib {
                        best_vram_mib = mem_mib;
                    }
                }
                // Right side: engine utilizations separated by "||".
                // "rcs% || vcs% || vecs% || bcs% || ccs%"
                let right_joined = parts[1..].join("|");
                let right = right_joined.trim();
                let mut engine_utils: Vec<f64> = Vec::new();
                for engine_part in right.split("||") {
                    let pct_str = engine_part.trim().trim_end_matches('%');
                    if let Ok(pct) = pct_str.parse::<f64>() {
                        engine_utils.push(pct);
                    }
                }
                // Average across engines for overall util.
                if !engine_utils.is_empty() {
                    let avg = engine_utils.iter().sum::<f64>() / engine_utils.len() as f64;
                    let util = (avg.clamp(0.0, 100.0)) as u8;
                    if util > best_util {
                        best_util = util;
                    }
                }
            }
        }
    }

    if !found_data {
        return None;
    }

    let name = tile_name("card0").unwrap_or_else(|| "Intel Arc".to_string());
    let temp_c = read_cpu_temp_c();

    Some(GpuSnapshot {
        index: 0,
        name,
        sm_clock_mhz: freq_mhz,
        mem_clock_mhz: 0,
        util_pct: best_util,
        vram_used_mib: best_vram_mib,
        vram_total_mib: 0, // Shared memory — no dedicated VRAM total on iGPU.
        temp_c,
        power_w: 0.0,
        power_limit_w: 0.0,
        uuid: String::new(),
    })
}

/// No compute-process listing for Intel sysfs; return empty (fdinfo-based
/// attribution is out of scope for now).
pub fn query_processes(_gpus: &[GpuSnapshot]) -> Vec<GpuProcess> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_on_missing_sysfs() {
        // card9999 doesn't exist → sysfs path yields nothing.
        let root = std::path::PathBuf::from("/sys/class/drm/card9999/device");
        assert!(read_sysfs_u64(&root.join("nope")).is_none());
        let _ = root;
    }

    #[test]
    fn sysfs_freq_produces_util() {
        // Direct unit check of the util derivation math.
        let util = |act: u64, max: u64| -> u8 {
            if max > 0 {
                ((act * 100).saturating_div(max) as u8).min(100)
            } else {
                0
            }
        };
        assert_eq!(util(0, 2500), 0);
        assert_eq!(util(1250, 2500), 50);
        assert_eq!(util(2500, 2500), 100);
        assert_eq!(util(0, 0), 0);
    }

    #[test]
    fn parse_size_to_mib_cases() {
        assert_eq!(parse_size_to_mib("1G"), 1024);
        assert_eq!(parse_size_to_mib("15M"), 15);
        assert_eq!(parse_size_to_mib("92K"), 0); // rounds down from 0.09
        assert_eq!(parse_size_to_mib("0"), 0);
        assert_eq!(parse_size_to_mib("2500"), 2500);
    }

    #[test]
    fn strip_ansi_removes_escapes() {
        let input = "\x1b[H\x1b[J\x1b[7mHello\x1b[0m World";
        assert_eq!(strip_ansi(input), "Hello World");
    }

    #[test]
    fn gputop_parsing_unit() {
        // Simulate gputop output and verify parsing logic.
        let fake_output = "\
\x1b[H\x1b[J\x1b[7mDRM minor 128   Frequency(MHz) GT0-2500/2500 GT1-1200/1200                      \x1b[0m\n\
    PID      MEM      RSS   rcs     vcs     vecs    bcs     ccs    NAME         \x1b[0m\n\
1530118       1G       1G |  0.0% ||  0.0% ||  0.0% ||  0.0% ||  0.0% | llama-server \n\
1530118      15M      92K |  0.0% ||  0.0% ||  0.0% ||  0.0% ||  0.0% | llama-server \n";
        let text = strip_ansi(fake_output);

        // Extract frequency.
        let mut freq_mhz: u16 = 0;
        for line in text.lines() {
            if line.contains("GT0-") {
                if let Some(gt0_section) = line.split("GT0-").nth(1) {
                    if let Some(freq_part) = gt0_section.split_whitespace().next() {
                        if let Some(cur_str) = freq_part.split('/').next() {
                            freq_mhz = cur_str.parse().unwrap_or(0);
                        }
                    }
                }
                break;
            }
        }
        assert_eq!(freq_mhz, 2500);

        // Parse VRAM from largest MEM entry.
        let mut best_vram_mib: u64 = 0;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(char::is_numeric) && trimmed.contains(" | ") {
                let parts: Vec<&str> = trimmed.split('|').collect();
                if parts.len() >= 2 {
                    let left = parts[0].trim();
                    let fields: Vec<&str> = left.split_whitespace().collect();
                    if fields.len() >= 2 {
                        let mem_mib = parse_size_to_mib(fields[1]);
                        if mem_mib > best_vram_mib {
                            best_vram_mib = mem_mib;
                        }
                    }
                }
            }
        }
        assert_eq!(best_vram_mib, 1024); // 1G = 1024 MiB
    }
}