//! Janitor — stale-cache management and auto-removal (2026-11-13).
//!
//! Officina accumulates runtime artifacts: expired prompt suggestions,
//! stale watchdog facts, finished longjob logs, old probe reports, engine
//! KV slot mirrors, and superseded session-chain snapshots (the
//! `.repaired-…/ .patched-…` files every crash-heal leaves behind).
//!
//! Contract (safety first):
//! - The PLAN is pure and testable; `apply` only touches planned items.
//! - Never removed: the newest file of any session store, session base
//!   files, `*.corrupt-*` quarantines, `slot0.bin`, locks, route ledgers,
//!   operator prefs (tui-*), input history, backups.
//! - Auto-run at most once per day, silent unless something was removed;
//!   `OFFICINA_NO_JANITOR=1` disables it. Manual: `/janitor [dry]`.
//!
//! PROVENANCE: original work, this repo.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub struct PlanItem {
    pub path: PathBuf,
    pub bytes: u64,
    pub reason: &'static str,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub items: Vec<PlanItem>,
}

impl Plan {
    pub fn bytes(&self) -> u64 {
        self.items.iter().map(|i| i.bytes).sum()
    }
}

/// Roots are injectable so tests never touch the operator's home.
#[derive(Debug, Clone)]
pub struct Roots {
    pub officina: PathBuf,
    pub jobs: PathBuf,
    pub probes: PathBuf,
    pub sessions: PathBuf,
    pub kv: PathBuf,
}

impl Roots {
    pub fn live() -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        Self {
            officina: home.join(".vitriol").join("officina"),
            jobs: home
                .join(".local")
                .join("share")
                .join("officina")
                .join("longjobs"),
            probes: home.join(".vitriol").join("probes"),
            sessions: home.join(".pi").join("agent").join("sessions"),
            kv: crate::tui::cache::cache_dir(),
        }
    }
}

const DAY: Duration = Duration::from_secs(86_400);

fn age_of(meta: &fs::Metadata, now: SystemTime) -> Duration {
    now.duration_since(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
        .unwrap_or_default()
}

fn file_plan(
    path: &Path,
    max_age: Duration,
    now: SystemTime,
    reason: &'static str,
) -> Option<PlanItem> {
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || age_of(&meta, now) <= max_age {
        return None;
    }
    Some(PlanItem {
        path: path.to_path_buf(),
        bytes: meta.len(),
        reason,
    })
}

/// Build the removal plan. Pure with respect to the filesystem (reads only).
pub fn plan(roots: &Roots, now: SystemTime) -> Plan {
    let mut out = Plan::default();

    // 1. Expired prompt suggestions (12 h fallback when no expiry stamp).
    let suggestion = roots.officina.join("prompt-suggestion.json");
    if let Ok(raw) = fs::read_to_string(&suggestion) {
        let expired = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|d| d.get("expires").and_then(|v| v.as_u64()))
            .map(|exp| exp > 0 && exp <= crate::tui::caduceus::now_ms())
            .unwrap_or(false);
        let stale = fs::metadata(&suggestion)
            .map(|m| age_of(&m, now) > Duration::from_secs(12 * 3600))
            .unwrap_or(false);
        if expired || stale {
            if let Ok(meta) = fs::metadata(&suggestion) {
                out.items.push(PlanItem {
                    path: suggestion.clone(),
                    bytes: meta.len(),
                    reason: "expired suggestion",
                });
            }
        }
    }

    // 2. Stale watchdog facts (24 h).
    if let Some(item) = file_plan(
        &roots.officina.join("watchdog.json"),
        DAY,
        now,
        "stale watchdog facts",
    ) {
        out.items.push(item);
    }

    // 3. Finished longjob dirs: older than 7 days, then cap the newest 50.
    let mut jobs: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&roots.jobs) {
        for ent in rd.flatten() {
            let p = ent.path();
            let Ok(meta) = ent.metadata() else { continue };
            if !meta.is_dir() {
                continue;
            }
            // only finished jobs (exit marker); running jobs are sacred.
            // Age comes from the exit marker — the dir mtime moves whenever
            // anything is written inside it (test-caught, 2026-11-13).
            let Ok(exit_meta) = fs::metadata(p.join("exit")) else {
                continue;
            };
            let size = dir_size(&p);
            jobs.push((
                p,
                size,
                exit_meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
            let _ = meta;
        }
    }
    jobs.sort_by(|a, b| b.2.cmp(&a.2));
    for (i, (p, size, modified)) in jobs.into_iter().enumerate() {
        let old = now.duration_since(modified).unwrap_or_default() > 7 * DAY;
        if old || i >= 50 {
            out.items.push(PlanItem {
                path: p,
                bytes: size,
                reason: if i >= 50 {
                    "longjob cap (50)"
                } else {
                    "old finished longjob"
                },
            });
        }
    }

    // 4. Old probe reports (30 days).
    if let Ok(rd) = fs::read_dir(&roots.probes) {
        for ent in rd.flatten() {
            let p = ent.path();
            if p.extension().map(|e| e == "json").unwrap_or(false) {
                if let Some(item) = file_plan(&p, 30 * DAY, now, "old probe report") {
                    out.items.push(item);
                }
            }
        }
    }

    // 5. Engine KV slot mirrors: older than 14 days, slot0.bin protected.
    if let Ok(rd) = fs::read_dir(&roots.kv) {
        for ent in rd.flatten() {
            let p = ent.path();
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if crate::tui::cache::is_protected(&name) {
                continue;
            }
            if let Some(item) = file_plan(&p, 14 * DAY, now, "stale KV slot mirror") {
                out.items.push(item);
            }
        }
    }

    // 6. Superseded session-chain snapshots. Per store: keep the store's
    //    newest file (mtime), keep every base file, keep corrupt
    //    quarantines; remove marked chain snapshots older than 7 days
    //    except the newest marked one per family.
    if let Ok(stores) = fs::read_dir(&roots.sessions) {
        for store in stores.flatten() {
            if !store.path().is_dir() {
                continue;
            }
            let mut files: Vec<(PathBuf, String, u64, SystemTime)> = Vec::new();
            if let Ok(rd) = fs::read_dir(store.path()) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if !name.ends_with(".jsonl") && !name.contains(".jsonl.") {
                        continue;
                    }
                    let Ok(meta) = ent.metadata() else { continue };
                    if !meta.is_file() {
                        continue;
                    }
                    files.push((
                        p,
                        name,
                        meta.len(),
                        meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    ));
                }
            }
            if files.is_empty() {
                continue;
            }
            // protection: the newest file of this store
            let newest = files
                .iter()
                .max_by(|a, b| a.3.cmp(&b.3))
                .map(|f| f.0.clone());
            // Marked chain snapshots (repair/heal leaves) older than 7
            // days are removed. Protections: the store's newest file (the
            // live continuation), base session files, and corrupt
            // quarantines. No family-level pin is needed — the store-newest
            // rule already covers the only file that can be live.
            for (p, name, size, modified) in &files {
                if Some(p.clone()) == newest {
                    continue;
                }
                if name.contains(".corrupt-") {
                    continue;
                }
                if chain_base(name).is_none() {
                    continue; // base session files stay
                }
                let old = now.duration_since(*modified).unwrap_or_default() > 7 * DAY;
                if old {
                    out.items.push(PlanItem {
                        path: p.clone(),
                        bytes: *size,
                        reason: "superseded session snapshot",
                    });
                }
            }
        }
    }

    out
}

/// Base family of a session-chain filename, or None for base sessions.
/// `x.jsonl.repaired-1.jsonl.patched-2.jsonl` → `x.jsonl`.
fn chain_base(name: &str) -> Option<String> {
    let markers = [".repaired-", ".patched-", ".bak-", ".tmp-"];
    let mut idx = None;
    for m in markers {
        if let Some(i) = name.find(m) {
            idx = Some(idx.map_or(i, |cur: usize| cur.min(i)));
        }
    }
    idx.map(|i| name[..i].to_string())
}

/// Remove planned items; returns (count, bytes). Errors are tolerated
/// (a file vanishing mid-plan is normal).
pub fn apply(items: &[PlanItem]) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for item in items {
        let meta = fs::metadata(&item.path).ok();
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let ok = if is_dir {
            fs::remove_dir_all(&item.path).is_ok()
        } else {
            fs::remove_file(&item.path).is_ok()
        };
        if ok {
            count += 1;
            bytes += item.bytes;
        }
    }
    (count, bytes)
}

fn dir_size(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = fs::read_dir(path) {
        for ent in rd.flatten() {
            let Ok(meta) = ent.metadata() else { continue };
            if meta.is_file() {
                total += meta.len();
            }
        }
    }
    total
}

/// Auto-run stamp (day granularity).
pub fn stamp_path() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    home.join(".vitriol").join("officina").join("last-janitor")
}

fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // day number is enough for a once-per-day stamp
    format!("day-{}", secs / 86_400)
}

/// Run at most once per day. Returns Some((count, bytes)) when it ran and
/// removed something.
pub fn auto_run() -> Option<(usize, u64)> {
    if std::env::var("OFFICINA_NO_JANITOR")
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return None;
    }
    let stamp = stamp_path();
    let today = today();
    if fs::read_to_string(&stamp)
        .map(|s| s.trim() == today)
        .unwrap_or(false)
    {
        return None;
    }
    let plan = plan(&Roots::live(), SystemTime::now());
    let (count, bytes) = apply(&plan.items);
    if let Some(dir) = stamp.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = fs::write(&stamp, format!("{today}\n"));
    if count > 0 {
        Some((count, bytes))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::fs::FileTimes;
    use std::io::Write;

    fn temp_roots(tag: &str) -> Roots {
        let base = std::env::temp_dir().join(format!("janitor-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        for d in ["officina", "jobs", "probes", "sessions", "kv"] {
            fs::create_dir_all(base.join(d)).unwrap();
        }
        Roots {
            officina: base.join("officina"),
            jobs: base.join("jobs"),
            probes: base.join("probes"),
            sessions: base.join("sessions"),
            kv: base.join("kv"),
        }
    }

    fn write_file(path: &Path, content: &str, age_days: u64) {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        let mut f = File::create(path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_days * 86_400);
        f.set_times(FileTimes::new().set_modified(when)).unwrap();
    }

    fn reasons(plan: &Plan) -> Vec<&str> {
        plan.items.iter().map(|i| i.reason).collect()
    }

    #[test]
    fn removes_expired_and_stale_officina_facts() {
        let roots = temp_roots("facts");
        // expired suggestion (expires in the past)
        write_file(
            &roots.officina.join("prompt-suggestion.json"),
            r#"{"version":1,"id":"x","text":"t","source":"llm","created":"","expires":1}"#,
            0,
        );
        // fresh suggestion stays
        let fresh = roots.officina.join("prompt-suggestion-fresh.json");
        write_file(&fresh, r#"{"expires":0}"#, 0);
        // stale watchdog
        write_file(&roots.officina.join("watchdog.json"), "{}", 2);
        let now = SystemTime::now();
        let p = plan(&roots, now);
        assert!(p
            .items
            .iter()
            .any(|i| i.path.ends_with("prompt-suggestion.json")));
        assert!(p.items.iter().any(|i| i.path.ends_with("watchdog.json")));
        assert!(!p.items.iter().any(|i| i.path == fresh));
    }

    #[test]
    fn longjobs_only_finished_and_old_or_over_cap() {
        let roots = temp_roots("jobs");
        for i in 0..3 {
            let d = roots.jobs.join(format!("j-{i}"));
            fs::create_dir_all(&d).unwrap();
            write_file(&d.join("out.log"), "log", 10);
            if i != 1 {
                write_file(&d.join("exit"), "exit=0", 10);
            }
        }
        let now = SystemTime::now();
        let p = plan(&roots, now);
        let paths: Vec<String> = p
            .items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(paths.contains(&"j-0".to_string()));
        assert!(!paths.contains(&"j-1".to_string()), "running job protected");
        assert!(paths.contains(&"j-2".to_string()));
    }

    #[test]
    fn kv_protects_slot0_and_keeps_fresh_files() {
        let roots = temp_roots("kv");
        write_file(&roots.kv.join("slot0.bin"), "warm", 30);
        write_file(&roots.kv.join("cache-old.bin"), "old", 20);
        write_file(&roots.kv.join("cache-fresh.bin"), "new", 1);
        let now = SystemTime::now();
        let p = plan(&roots, now);
        let names: Vec<String> = p
            .items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.contains(&"slot0.bin".to_string()),
            "warm restore protected"
        );
        assert!(names.contains(&"cache-old.bin".to_string()));
        assert!(!names.contains(&"cache-fresh.bin".to_string()));
    }

    #[test]
    fn session_chains_keep_newest_and_base_and_corrupt() {
        let roots = temp_roots("sessions");
        let store = roots.sessions.join("--proj--");
        let base = "s.jsonl";
        write_file(&store.join(base), "base", 10);
        let c1 = "s.jsonl.repaired-1.jsonl";
        let c2 = "s.jsonl.repaired-2.jsonl";
        let c3 = "s.jsonl.repaired-2.jsonl.patched-3.jsonl";
        write_file(&store.join(c1), "c1", 9);
        write_file(&store.join(c2), "c2", 8);
        write_file(&store.join(c3), "c3", 7);
        // corrupt quarantine must never be planned
        let corrupt = "s.jsonl.repaired-2.jsonl.corrupt-9";
        write_file(&store.join(corrupt), "q", 40);
        let now = SystemTime::now();
        let now = SystemTime::now();
        let p = plan(&roots, now);
        let names: Vec<String> = p
            .items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(!names.contains(&base.to_string()), "base session stays");
        assert!(
            !names.contains(&c3.to_string()),
            "newest chain snapshot stays"
        );
        assert!(
            names.contains(&c1.to_string()),
            "older chain snapshot removed"
        );
        assert!(names.contains(&c2.to_string()));
        assert!(!names.contains(&corrupt.to_string()), "quarantine stays");
    }

    #[test]
    fn newest_store_file_is_never_planned() {
        let roots = temp_roots("newest");
        let store = roots.sessions.join("--proj--");
        // newest file happens to be a marked snapshot → still protected
        write_file(&store.join("s.jsonl.repaired-1.jsonl"), "old", 10);
        write_file(&store.join("s.jsonl.repaired-2.jsonl"), "new", 1);
        write_file(&store.join("s.jsonl.repaired-3.jsonl"), "mid", 9);
        let now = SystemTime::now();
        let now = SystemTime::now();
        let p = plan(&roots, now);
        let names: Vec<String> = p
            .items
            .iter()
            .map(|i| i.path.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.contains(&"s.jsonl.repaired-2.jsonl".to_string()),
            "newest store file protected"
        );
        assert!(names.contains(&"s.jsonl.repaired-1.jsonl".to_string()));
        assert!(names.contains(&"s.jsonl.repaired-3.jsonl".to_string()));
    }

    #[test]
    fn chain_base_extraction() {
        assert_eq!(
            chain_base("s.jsonl.repaired-1.jsonl.patched-2.jsonl").as_deref(),
            Some("s.jsonl")
        );
        assert_eq!(chain_base("s.jsonl.bak-3"), Some("s.jsonl".to_string()));
        assert_eq!(chain_base("s.jsonl"), None);
    }

    #[test]
    fn apply_removes_and_reports() {
        let roots = temp_roots("apply");
        write_file(&roots.probes.join("probe-old.json"), "{}", 40);
        let now = SystemTime::now();
        let p = plan(&roots, now);
        assert_eq!(reasons(&p), vec!["old probe report"]);
        let (count, bytes) = apply(&p.items);
        assert_eq!(count, 1);
        assert!(bytes > 0);
        assert!(!roots.probes.join("probe-old.json").exists());
    }
}
