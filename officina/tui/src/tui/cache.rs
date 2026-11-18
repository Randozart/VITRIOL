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
/// The policy the engine is enforcing: `[server] cache_max_gb` from the config
/// (GiB), else the engine's historical derived cap (12 x cache_ram MiB). 0 means
/// no cap. The engine evicts the least-recently-addressed entry above this, so
/// the UI must show the same number the engine acts on (2026-11-17).
/// The engine's RAM prompt-cache budget (`[server] cache_ram`, MiB). Separate
/// from the disk cap: one holds live session states, the other holds snapshots.
pub fn ram_mib() -> u64 {
    let Some(home) = std::env::var_os("HOME") else {
        return 0;
    };
    let Ok(text) = fs::read_to_string(PathBuf::from(home).join(".vitriol").join("config")) else {
        return 0;
    };
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("cache_ram") {
            if let Some(v) = v.split('=').nth(1) {
                return v.trim().parse().unwrap_or(0);
            }
        }
    }
    0
}

pub fn cap_bytes() -> u64 {
    if let Ok(gb) = std::env::var("VITRIOL_KV_CACHE_MAX_GB") {
        if let Ok(value) = gb.trim().parse::<f64>() {
            if value > 0.0 {
                return (value * 1_073_741_824.0) as u64;
            }
        }
    }
    let Some(home) = std::env::var_os("HOME") else {
        return 0;
    };
    let Ok(text) = fs::read_to_string(PathBuf::from(home).join(".vitriol").join("config")) else {
        return 0;
    };
    let mut cap_gb = 0.0f64;
    let mut cache_ram_mib = 0u64;
    for line in text.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("cache_max_gb") {
            if let Some(v) = v.split('=').nth(1) {
                cap_gb = v.trim().parse().unwrap_or(0.0);
            }
        } else if let Some(v) = line.strip_prefix("cache_ram") {
            if let Some(v) = v.split('=').nth(1) {
                cache_ram_mib = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if cap_gb > 0.0 {
        return (cap_gb * 1_073_741_824.0) as u64;
    }
    // Mirror the engine's fallback so the readout never lies about the policy.
    cache_ram_mib.saturating_mul(1024).saturating_mul(12)
}

/// One-line budget readout: "2.51 / 40.00 GiB", or "over cap" when the
/// protected entries alone exceed it.
pub fn budget_line(used: u64) -> String {
    let cap = cap_bytes();
    if cap == 0 {
        return format!("{} (no cap)", fmt_bytes(used));
    }
    let status = if used > cap { " · OVER CAP" } else { "" };
    format!("{} / {}{}", fmt_bytes(used), fmt_bytes(cap), status)
}

/// Live sessions sharing this engine (2026-11-18).
///
/// Each `officina` process publishes one shard under
/// `~/.vitriol/officina/routing-registry.d/<pid>.json` while it routes, so the
/// shard count is the number of sessions competing for KV states. Used only for
/// the capacity readout — never for a decision.
pub fn live_sessions() -> usize {
    let Some(home) = std::env::var_os("HOME") else {
        return 0;
    };
    let dir = PathBuf::from(home)
        .join(".vitriol")
        .join("officina")
        .join("routing-registry.d");
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .count()
}

/// Size of one KV state, estimated from the newest slot snapshot on disk.
///
/// The engine's RAM prompt cache holds one state per session; without a live
/// per-state byte count, the newest snapshot is the honest proxy (each is a
/// serialized slot state for this model).
pub fn state_bytes(dir: &Path) -> Option<u64> {
    scan(dir)
        .into_iter()
        .max_by_key(|e| e.modified)
        .map(|e| e.size)
        .filter(|n| *n > 0)
}

/// Capacity readout: how many session states the RAM prompt cache holds.
///
/// `fits = ram_bytes / state_bytes`; `sessions` is how many compete right now.
/// When the two meet, the next session evicts the oldest state and pays a full
/// re-prefill — the failure mode this line exists to make visible (2026-11-18:
/// 4 sessions x 1.7 GB against an 8 GB cache).
pub fn capacity_line(ram_mib: u64, state: Option<u64>, sessions: usize) -> String {
    let sessions_txt = format!(
        "{sessions} session{} live",
        if sessions == 1 { "" } else { "s" }
    );
    let Some(state) = state.filter(|n| *n > 0) else {
        return format!("{sessions_txt} · state size unknown");
    };
    if ram_mib == 0 {
        return format!("{sessions_txt} · {} / state", fmt_bytes(state));
    }
    let ram = ram_mib.saturating_mul(1024 * 1024);
    let fits = (ram / state).max(1);
    let tight = sessions as u64 >= fits;
    format!(
        "{sessions_txt} · ~{fits} state{} fit · {} each{}",
        if fits == 1 { "" } else { "s" },
        fmt_bytes(state),
        if tight { " · TIGHT" } else { "" }
    )
}

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

    /// Multi-session capacity (2026-11-18): four sessions x 1.7 GB against an
    /// 8 GB RAM cache is the cliff this readout exists to expose. The arithmetic
    /// must be honest in both directions — a comfortable margin must NOT read
    /// as tight, or the warning gets ignored.
    #[test]
    fn capacity_line_reports_the_eviction_cliff_honestly() {
        const GB: u64 = 1024 * 1024; // MiB
        let state = 1740 * GB; // ~1.7 GB per session state
                               // 4 sessions against an 8 GB cache: no room for a 5th.
        let tight = capacity_line(8 * 1024, Some(state), 4);
        assert!(tight.contains("4 sessions live"), "{tight}");
        assert!(tight.contains("TIGHT"), "{tight}");
        // 2 sessions against 16 GB: comfortable, and it must say so.
        let roomy = capacity_line(16 * 1024, Some(state), 2);
        assert!(roomy.contains("2 sessions live"), "{roomy}");
        assert!(!roomy.contains("TIGHT"), "{roomy}");
        // Unknown state size never invents a number.
        assert!(capacity_line(16 * 1024, None, 2).contains("state size unknown"));
        // No configured budget: report the state size only.
        assert!(capacity_line(0, Some(state), 1).contains("1 session live"));
    }
}
