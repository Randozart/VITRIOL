//! PROFILES tab: hybrid config/profile editor.
//!
//! Two working surfaces, both read-only until an explicit action:
//!   - The active `~/.vitriol/config` INI, shown as editable `section.key` rows
//!     (form-style: navigate, edit value inline, save via temp+rename).
//!   - The profile set (bundled + installed), with save-as-new and delete.
//!
//! Nothing here spawns processes; raw full-file editing is left to the user's
//! `$EDITOR` via an explicit action that runs outside the TUI (never here).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// One editable config entry: section, bare key, current value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// INI section, e.g. `model` (empty for a top-level key).
    pub section: String,
    /// Bare key within the section.
    pub key: String,
    /// Current value (the editable string).
    pub value: String,
}

/// The loaded config file with its backing path (for atomic writes).
#[derive(Debug)]
pub struct ConfigFile {
    /// Absolute path of the INI file.
    pub path: PathBuf,
    /// Entries in stable section-then-key order.
    pub entries: Vec<Entry>,
}

impl ConfigFile {
    /// Load the active `~/.vitriol/config`. Missing file -> empty entry list.
    pub fn load(cfg: &Config) -> Self {
        let path = cfg.home_dir.join(".vitriol").join("config");
        let entries = fs::read_to_string(&path)
            .map(|t| parse_entries(&t))
            .unwrap_or_default();
        Self { path, entries }
    }

    /// Atomically write the current entries back to the INI (temp + rename).
    pub fn save(&self) -> Result<(), String> {
        let text = render_entries(&self.entries);
        atomic_write(&self.path, &text)
    }

    /// Add or replace an entry. Returns the previous value if it existed.
    pub fn upsert(&mut self, section: &str, key: &str, value: String) -> Option<String> {
        if let Some(e) = self
            .entries
            .iter_mut()
            .find(|e| e.section == section && e.key == key)
        {
            let old = std::mem::replace(&mut e.value, value);
            return Some(old);
        }
        self.entries.push(Entry {
            section: section.to_string(),
            key: key.to_string(),
            value,
        });
        self.entries
            .sort_by(|a, b| a.section.cmp(&b.section).then_with(|| a.key.cmp(&b.key)));
        None
    }

    /// Remove an entry by section+key; true if it existed.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|e| e.section != section || e.key != key);
        self.entries.len() != before
    }
}

/// Parse an INI text into entries, preserving section order on first sight.
fn parse_entries(text: &str) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if skip_comment(line) {
            continue;
        }
        if let Some(s) = section_of(line) {
            section = s;
            continue;
        }
        push_key(&mut out, line, &section);
    }
    out
}

/// True for blank lines and comment lines (start with `#`).
fn skip_comment(line: &str) -> bool {
    line.is_empty() || line.starts_with('#')
}

/// The trimmed section name when `line` is a `[section]` header, else None.
fn section_of(line: &str) -> Option<String> {
    let stripped = line.strip_prefix('[')?;
    let close = stripped.strip_suffix(']')?;
    let name = close.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Push one `key = value` line onto `out` under the active section.
fn push_key(out: &mut Vec<Entry>, line: &str, section: &str) {
    let Some((k, v)) = line.split_once('=') else {
        return;
    };
    let key = k.trim();
    if key.is_empty() {
        return;
    }
    out.push(Entry {
        section: section.to_string(),
        key: key.to_string(),
        value: v.trim().to_string(),
    });
}

/// Render entries back to INI text, grouping by section in first-seen order.
fn render_entries(entries: &[Entry]) -> String {
    let mut sections: Vec<&str> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut by_section: BTreeMap<&str, Vec<&Entry>> = BTreeMap::new();
    for e in entries {
        if seen.insert(&e.section) {
            sections.push(&e.section);
        }
        by_section.entry(&e.section).or_default().push(e);
    }
    let mut out = String::new();
    for section in sections {
        if !section.is_empty() {
            out.push('[');
            out.push_str(section);
            out.push_str("]\n");
        }
        let rows = by_section.get(section).map(|v| v.as_slice()).unwrap_or(&[]);
        for e in rows {
            out.push_str(&e.key);
            out.push_str(" = ");
            out.push_str(&e.value);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// Write `text` to `path` atomically: temp file in the same dir, then rename.
fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let tmp = dir.join(format!(".{}.tmp", file_stem(path)));
    fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
}

/// The file name without the final extension (for a temp sibling name).
fn file_stem(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| {
            n.rsplit_once('.')
                .map(|(stem, _)| stem.to_string())
                .unwrap_or_else(|| n.to_string())
        })
        .unwrap_or_else(|| "config".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_render_roundtrip() {
        let text = "# c\n[model]\npath = /x\nngl = 24\n\n[server]\nparallel=4\n";
        let entries = parse_entries(text);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].section, "model");
        assert_eq!(entries[0].key, "path");
        assert_eq!(entries[0].value, "/x");
        let rendered = render_entries(&entries);
        assert!(rendered.contains("[model]"));
        assert!(rendered.contains("path = /x"));
        assert!(rendered.contains("[server]"));
        assert!(rendered.contains("parallel = 4"));
    }

    #[test]
    fn upsert_adds_and_replaces() {
        let mut f = ConfigFile {
            path: PathBuf::from("/nonexistent/config"),
            entries: Vec::new(),
        };
        assert_eq!(f.upsert("model", "ngl", "24".into()), None);
        assert_eq!(f.upsert("model", "ngl", "32".into()), Some("24".into()));
        assert!(f
            .entries
            .iter()
            .any(|e| e.section == "model" && e.key == "ngl" && e.value == "32"));
    }

    #[test]
    fn remove_deletes_only_match() {
        let mut f = ConfigFile {
            path: PathBuf::from("/nonexistent/config"),
            entries: vec![
                Entry {
                    section: "model".into(),
                    key: "ngl".into(),
                    value: "24".into(),
                },
                Entry {
                    section: "server".into(),
                    key: "parallel".into(),
                    value: "1".into(),
                },
            ],
        };
        assert!(f.remove("model", "ngl"));
        assert!(!f.remove("model", "ngl"));
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.entries[0].key, "parallel");
    }
}
