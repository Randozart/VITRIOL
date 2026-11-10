// Engine KV disk cache manager — the slot-save-path directory
// (/scratch/kvtier on this host). Entries are multi-GB slot mirrors
// (slot0.bin) plus per-prompt cache-*.bin files; the directory grows until
// pruned (owner request 2026-11-10: one command + a small age/size UI).
//
// PROVENANCE: original work, this repo.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub modified: SystemTime,
}

impl CacheEntry {
    pub fn age(&self) -> Duration {
        self.modified.elapsed().unwrap_or_default()
    }
}

/// slot0.bin is the engine's warm-boot restore checkpoint. The UI and the
/// blanket wipe protect it; age-based prune also protects it unless the
/// caller explicitly says otherwise.
pub fn is_protected(name: &str) -> bool {
    name == "slot0.bin"
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// Resolve the cache directory: explicit env override, then the live engine
/// config's `slot_save_path`, then the host default.
pub fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("VITRIOL_KV_DISK_DIR") {
        if !dir.trim().is_empty() {
            return expand_home(dir.trim());
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let cfg = PathBuf::from(home).join(".vitriol").join("config");
        if let Ok(text) = fs::read_to_string(&cfg) {
            if let Some(dir) = slot_save_path_from_config(&text) {
                return expand_home(&dir);
            }
        }
    }
    PathBuf::from("/scratch/kvtier")
}

fn slot_save_path_from_config(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        let Some(value) = line.strip_prefix("slot_save_path") else {
            continue;
        };
        let value = value.trim_start();
        let Some(value) = value.strip_prefix('=') else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// List cache files, newest first. Missing/unreadable directory = empty list.
pub fn scan(dir: &Path) -> Vec<CacheEntry> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<CacheEntry> = rd
        .flatten()
        .filter_map(|ent| {
            let meta = ent.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some(CacheEntry {
                name: ent.file_name().to_string_lossy().to_string(),
                path: ent.path(),
                size: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            })
        })
        .collect();
    entries.sort_by(|a, b| b.modified.cmp(&a.modified));
    entries
}

pub fn total_size(entries: &[CacheEntry]) -> u64 {
    entries.iter().map(|e| e.size).sum()
}

pub fn fmt_bytes(n: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let f = n as f64;
    if f >= GIB {
        format!("{:.2} GiB", f / GIB)
    } else if f >= MIB {
        format!("{:.1} MiB", f / MIB)
    } else if n >= 1024 {
        format!("{} KiB", n / 1024)
    } else {
        format!("{n} B")
    }
}

pub fn fmt_age(age: Duration) -> String {
    let s = age.as_secs();
    if s < 60 {
        "now".to_string()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else if s < 86_400 {
        format!("{}h", s / 3600)
    } else if s < 7 * 86_400 {
        format!("{}d", s / 86_400)
    } else {
        format!("{}w", s / (7 * 86_400))
    }
}

/// Parse `30m`, `12h`, `7d`, `4w`. A bare integer means days.
pub fn parse_age(text: &str) -> Option<Duration> {
    let text = text.trim().to_ascii_lowercase();
    if text.is_empty() {
        return None;
    }
    let (num, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&text[..i], c),
        _ => (text.as_str(), 'd'),
    };
    let n: u64 = num.trim().parse().ok()?;
    let secs = match unit {
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(60 * 60)?,
        'd' => n.checked_mul(24 * 60 * 60)?,
        'w' => n.checked_mul(7 * 24 * 60 * 60)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// Delete cache entries older than `age` (None = every non-protected entry).
/// `include_protected` may only be set by an explicit user action.
pub fn wipe(dir: &Path, age: Option<Duration>, include_protected: bool) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in scan(dir) {
        if !include_protected && is_protected(&entry.name) {
            continue;
        }
        if let Some(age) = age {
            if entry.age() < age {
                continue;
            }
        }
        if fs::remove_file(&entry.path).is_ok() {
            count += 1;
            bytes = bytes.saturating_add(entry.size);
        }
    }
    (count, bytes)
}

pub fn delete(entry: &CacheEntry) -> bool {
    fs::remove_file(&entry.path).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn parses_age_units() {
        assert_eq!(parse_age("30m"), Some(Duration::from_secs(1800)));
        assert_eq!(parse_age("12h"), Some(Duration::from_secs(43200)));
        assert_eq!(parse_age("7d"), Some(Duration::from_secs(604800)));
        assert_eq!(parse_age("4w"), Some(Duration::from_secs(2419200)));
        assert_eq!(parse_age("3"), Some(Duration::from_secs(259200)));
        assert_eq!(parse_age("nope"), None);
    }

    #[test]
    fn parses_slot_save_path_from_config() {
        let text = "# comment\n[server]\nslot_save_path = /scratch/kvtier\ncache_ram = 12288\n";
        assert_eq!(
            slot_save_path_from_config(text).as_deref(),
            Some("/scratch/kvtier")
        );
        assert_eq!(slot_save_path_from_config("slot_context = "), None);
    }

    #[test]
    fn protect_slot0_by_default() {
        assert!(is_protected("slot0.bin"));
        assert!(!is_protected("cache-abc.bin"));
    }

    #[test]
    fn wipe_all_still_protects_slot0() {
        let dir = std::env::temp_dir().join(format!("officina-cache-all-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("slot0.bin"), b"slot").unwrap();
        fs::write(dir.join("cache-a.bin"), b"aaa").unwrap();
        fs::write(dir.join("cache-b.bin"), b"bb").unwrap();
        let (count, bytes) = wipe(&dir, None, false);
        assert_eq!(count, 2);
        assert_eq!(bytes, 5);
        assert!(dir.join("slot0.bin").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wipe_prunes_old_but_keeps_slot0() {
        let dir = std::env::temp_dir().join(format!("officina-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("slot0.bin"), b"slot").unwrap();
        fs::write(dir.join("cache-new.bin"), b"new").unwrap();
        let old = dir.join("cache-old.bin");
        fs::write(&old, b"old").unwrap();
        // Backdate the old entry by 10 days.
        let old_time = SystemTime::now() - Duration::from_secs(10 * 24 * 3600);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(old_time)
            .unwrap();

        let (count, bytes) = wipe(&dir, Some(Duration::from_secs(7 * 24 * 3600)), false);
        assert_eq!(count, 1, "only the old cache entry goes");
        assert_eq!(bytes, 3);
        assert!(dir.join("slot0.bin").exists(), "slot0 protected");
        assert!(dir.join("cache-new.bin").exists(), "fresh entry kept");
        assert!(!old.exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
