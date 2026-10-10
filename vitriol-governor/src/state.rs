//! Persisted FSM (phase + last_transition). Streak trackers are rebuild-only
//! and reset on governor restart - dwell still holds via last_transition.

use crate::policy::Policy;
use std::path::Path;

pub fn load(path: &str) -> Policy {
    if !Path::new(path).exists() {
        return Policy::default();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| {
            crate::engine::log("WARN state file unreadable, starting from Full");
            Policy::default()
        })
}

pub fn store(path: &str, p: &Policy) {
    if let Some(dir) = Path::new(path).parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(p) {
        let tmp = format!("{path}.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}
