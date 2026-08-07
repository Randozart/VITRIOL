//! Named Rectification Masks — version-controlled firing tallies.
//!
//! A named mask is an ordered sequence of sparse transactions; the flat active
//! mask is always the *union* of the remaining transactions. Recording a
//! `RECTIFY` pass appends a transaction; `REVERT` surgically drops one. Because
//! each transaction is tiny (a sparse list of active expert ids), masks stay a
//! few kilobytes and rollback is exact — no pollution to guess away.
//!
//! Stored as JSON at `~/.vitriol/masks/<name>.mask`; transactions are
//! newest-highest-id. The engine owns no I/O policy beyond its own files.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One firing pass recorded into a mask.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MaskTxn {
    /// Monotonic transaction id (higher = newer).
    pub id: u64,
    /// UTC timestamp (seconds since epoch).
    pub ts: u64,
    /// The prompt that drove the pass.
    pub prompt: String,
    /// Source: `manual` (daily RECTIFY) or `ascensus` (cloud batch).
    pub source: String,
    /// Active expert ids that fired during the pass (sparse).
    pub fired: Vec<u32>,
}

/// A named mask file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MaskFile {
    /// Mask name (matches the file stem).
    pub name: String,
    /// Transactions, ordered oldest first.
    pub transactions: Vec<MaskTxn>,
}

impl MaskFile {
    /// A new empty mask.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            transactions: Vec::new(),
        }
    }

    /// The union of all remaining transactions (the derived flat mask).
    pub fn union_active(&self) -> BTreeSet<u32> {
        let mut set = BTreeSet::new();
        for t in &self.transactions {
            set.extend(t.fired.iter().copied());
        }
        set
    }

    /// Append a transaction with the next id; returns the txn.
    pub fn add(&mut self, ts: u64, prompt: &str, source: &str, fired: Vec<u32>) -> MaskTxn {
        let next = self.transactions.last().map(|t| t.id + 1).unwrap_or(1);
        let txn = MaskTxn {
            id: next,
            ts,
            prompt: prompt.to_string(),
            source: source.to_string(),
            fired,
        };
        self.transactions.push(txn.clone());
        txn
    }

    /// Remove a transaction by id; returns it when found.
    pub fn remove(&mut self, id: u64) -> Option<MaskTxn> {
        let idx = self.transactions.iter().position(|t| t.id == id)?;
        Some(self.transactions.remove(idx))
    }

    /// Summary stats for the union against `total` possible elements.
    pub fn stats(&self, total: u64) -> MaskStats {
        let active = self.union_active().len() as u64;
        MaskStats {
            active,
            total,
            dross: total.saturating_sub(active),
            txn_count: self.transactions.len() as u64,
        }
    }

    /// Load a mask from a file path; a missing file is an error.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    /// Write the mask to a file path (JSON).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize {}: {e}", path.display()))?;
        fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// A valid mask name (alnum, hyphen, underscore).
    pub fn valid_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }
}

/// Mask census numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MaskStats {
    /// Active (fired) elements in the union.
    pub active: u64,
    /// Total possible elements.
    pub total: u64,
    /// Never-fired (dross) elements.
    pub dross: u64,
    /// Number of recorded transactions.
    pub txn_count: u64,
}

impl MaskStats {
    /// Active fraction in `[0, 1]` (1.0 when `total` is 0).
    pub fn active_fraction(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            self.active as f64 / self.total as f64
        }
    }
}

/// The masks directory under a home dir.
pub fn masks_dir(home: &Path) -> PathBuf {
    home.join(".vitriol/masks")
}

/// The file path for a named mask.
pub fn mask_path(home: &Path, name: &str) -> PathBuf {
    masks_dir(home).join(format!("{name}.mask"))
}

/// List named masks (sorted stems) in the dir; missing dir -> empty.
pub fn list(home: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(masks_dir(home)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("mask"))
        .filter_map(|e| {
            e.path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("officina_mask_test_{name}"))
    }

    #[test]
    fn union_is_derived_from_transactions() {
        let mut m = MaskFile::new("vulkan");
        m.add(1, "a", "manual", vec![1, 2, 3]);
        m.add(2, "b", "manual", vec![3, 4]);
        let union = m.union_active();
        assert_eq!(union, BTreeSet::from([1, 2, 3, 4]));
    }

    #[test]
    fn revert_excludes_only_the_target_txn() {
        let mut m = MaskFile::new("vulkan");
        m.add(1, "a", "manual", vec![1, 2, 3]);
        m.add(2, "polluted", "manual", vec![99, 100]);
        m.add(3, "c", "manual", vec![1, 5]);
        let removed = m.remove(2).unwrap();
        assert_eq!(removed.prompt, "polluted");
        assert_eq!(m.union_active(), BTreeSet::from([1, 2, 3, 5]));
    }

    #[test]
    fn ids_are_monotonic() {
        let mut m = MaskFile::new("vulkan");
        assert_eq!(m.add(1, "a", "manual", vec![1]).id, 1);
        assert_eq!(m.add(2, "b", "manual", vec![2]).id, 2);
        m.remove(1);
        assert_eq!(m.add(3, "c", "manual", vec![3]).id, 3);
    }

    #[test]
    fn stats_compute_active_and_dross() {
        let mut m = MaskFile::new("vulkan");
        m.add(1, "a", "manual", vec![1, 2]);
        m.add(2, "b", "ascensus", vec![2, 3]);
        let s = m.stats(64);
        assert_eq!(s.active, 3);
        assert_eq!(s.dross, 61);
        assert_eq!(s.txn_count, 2);
        assert!((s.active_fraction() - 3.0 / 64.0).abs() < 1e-9);
    }

    #[test]
    fn save_load_roundtrip() {
        let home = tmp_home("roundtrip");
        let _ = fs::remove_dir_all(&home);
        let path = mask_path(&home, "vulkan");
        let mut m = MaskFile::new("vulkan");
        m.add(1, "shader", "manual", vec![1, 2, 3]);
        m.save(&path).unwrap();
        let loaded = MaskFile::load(&path).unwrap();
        assert_eq!(loaded, m);
        assert_eq!(loaded.union_active(), BTreeSet::from([1, 2, 3]));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn list_returns_sorted_stems() {
        let home = tmp_home("list");
        let _ = fs::remove_dir_all(&home);
        MaskFile::new("b-second")
            .save(&mask_path(&home, "b-second"))
            .unwrap();
        MaskFile::new("a-first")
            .save(&mask_path(&home, "a-first"))
            .unwrap();
        assert_eq!(list(&home), vec!["a-first", "b-second"]);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn valid_name_rules() {
        assert!(MaskFile::valid_name("vulkan_shaders"));
        assert!(MaskFile::valid_name("c-firmware"));
        assert!(!MaskFile::valid_name(""));
        assert!(!MaskFile::valid_name("bad name"));
        assert!(!MaskFile::valid_name("bad/name"));
    }
}
