//! Offline experiments: no NVML initialization or engine control in these entry points.
// PROVENANCE: inspiration — none; VITRIOL-original replay and experiment contracts.

mod cache;
pub use cache::{Source, Stats};
pub mod governor;
pub mod kernel;
pub mod model;
mod process;
pub mod replay;
pub mod validation;

use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn fingerprint(value: &impl Serialize) -> Result<String, String> {
    Ok(digest(
        &serde_json::to_vec(value).map_err(|e| e.to_string())?,
    ))
}

pub fn file_digest(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.len() > (512 << 20) {
        return Err("artifacts must be regular files <=512 MiB".into());
    }
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(metadata.len() + 1);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0;
    loop {
        let size = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if size == 0 {
            if total != metadata.len() {
                return Err("artifact size changed while hashing".into());
            }
            return Ok(format!("{:x}", hash.finalize()));
        }
        total += size as u64;
        hash.update(&buffer[..size]);
    }
}

pub fn label(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err("labels must be nonempty, bounded and contain no control characters".into());
    }
    Ok(())
}
