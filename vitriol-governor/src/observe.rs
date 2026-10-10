//! NVML observation: per-GPU free VRAM + foreign compute-tenant detection.

use crate::policy::Sample;
use nvml_wrapper::Nvml;
use std::collections::HashSet;

pub struct Observer {
    nvml: Nvml,
}

impl Observer {
    pub fn new() -> Result<Observer, String> {
        let nvml = Nvml::init().map_err(|e| format!("nvml init: {e:?}"))?;
        Ok(Observer { nvml })
    }

    /// One system sample: free MiB per GPU + foreign-tenant flag.
    /// `engine_pids` are the engine's; a foreign compute PID holding at
    /// least `tenant_min_mib` counts as a tenant (desktop compositor /
    /// browser GPU contexts stay below it and must not trigger yields).
    pub fn sample(&self, engine_pids: &HashSet<u32>, engine_up: bool, tenant_min_mib: u64) -> Sample {
        let Ok(count) = self.nvml.device_count() else {
            return Sample { free_mib_per_gpu: vec![], foreign_tenant: false, engine_up };
        };
        let mut free = Vec::new();
        let mut foreign = false;
        for i in 0..count {
            let Ok(dev) = self.nvml.device_by_index(i) else { continue };
            let (free_mib, dev_foreign) = device_sample(&dev, engine_pids, tenant_min_mib);
            if let Some(mib) = free_mib {
                free.push(mib);
            }
            foreign = foreign || dev_foreign;
        }
        Sample { free_mib_per_gpu: free, foreign_tenant: foreign, engine_up }
    }
}

/// (free MiB, has foreign tenant) for one device. Per-process used memory
/// arrives in KiB and may be None on some drivers - treated as 0.
fn device_sample(
    dev: &nvml_wrapper::device::Device,
    engine_pids: &HashSet<u32>,
    tenant_min_mib: u64,
) -> (Option<u64>, bool) {
    let free_mib = dev.memory_info().ok().map(|m| m.free / (1024 * 1024));
    let min = tenant_min_mib * 1024 * 1024;
    let foreign = dev
        .running_compute_processes()
        .map(|procs| {
            procs.iter().any(|p| {
                !engine_pids.contains(&p.pid)
                    && matches!(p.used_gpu_memory, nvml_wrapper::enums::device::UsedGpuMemory::Used(b) if b >= min)
            })
        })
        .unwrap_or(false);
    (free_mib, foreign)
}

/// Scan /proc for processes whose comm matches `name` (the engine may have
/// children beyond the unit's MainPID).
pub fn pids_by_comm(name: &str) -> HashSet<u32> {
    let mut out = HashSet::new();
    let Ok(entries) = std::fs::read_dir("/proc") else { return out };
    for e in entries.flatten() {
        let Some(fname) = e.file_name().to_str().map(String::from) else { continue };
        let Ok(pid) = fname.parse::<u32>() else { continue };
        let Ok(comm) = std::fs::read_to_string(format!("/proc/{pid}/comm")) else { continue };
        if comm.trim() == name {
            out.insert(pid);
        }
    }
    out
}
