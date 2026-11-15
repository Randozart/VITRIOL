//! Offline experiments: no NVML initialization or engine control in these entry points.
// PROVENANCE: inspiration — none; VITRIOL-original replay and experiment contracts.

mod cache;
pub mod governor;
pub mod model;
pub mod replay;

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

pub fn label(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err("labels must be nonempty, bounded and contain no control characters".into());
    }
    Ok(())
}
