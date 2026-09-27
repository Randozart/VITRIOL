// PROVENANCE: original - VITRIOL (2026-09-27, plan
// .opencode/plans/demiurge-2026-09-27.md, M2). Decision audit: one JSONL
// line per arbiter decision (lane, cause, privacy, pressure). SQLite
// ledger with budget rows arrives in M3; the JSONL is the honest
// human-readable trail and the test seam.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Audit {
    path: PathBuf,
    lock: Mutex<()>,
}

impl Audit {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn default_path() -> PathBuf {
        if let Ok(p) = std::env::var("DEMIURGE_AUDIT_FILE") {
            return PathBuf::from(p);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        PathBuf::from(home).join(".vitriol/demiurge/decisions.jsonl")
    }

    /// Append one record. Logging failure is swallowed (traced): an audit
    /// hiccup must never break routing.
    pub fn append(&self, record: &serde_json::Value) {
        let _guard = self.lock.lock();
        let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            tracing::warn!(path = %self.path.display(), "audit append failed: cannot open");
            return;
        };
        if let Err(e) = writeln!(f, "{record}") {
            tracing::warn!(error = %e, "audit write failed");
        }
    }
}
