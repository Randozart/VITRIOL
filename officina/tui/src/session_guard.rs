// Session startup guard: validate the newest session for the cwd before pi
// loads it, quarantine corrupt files, and hold a cross-process lock so offline
// recovery cannot mutate a session while the TUI is running.
//
// Why: interrupted tool calls + external repair branches made pi's turn_end
// boundary fail ("could not resolve persisted assistant entry ID") and silently
// stall the agent before tool execution. Validate and fail over to a fresh
// session instead of resuming a broken tree.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn lock_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OFFICINA_TUI_LOCK") {
        let p = PathBuf::from(path);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    Some(home()?.join(".vitriol").join("officina").join("tui.lock"))
}

/// Acquire the global TUI lock. A live PID in the file blocks offline repair.
pub struct LockGuard(Option<PathBuf>);

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            release_lock(path);
        }
    }
}

pub fn acquire_lock() -> LockGuard {
    LockGuard(acquire_lock_path())
}

fn acquire_lock_path() -> Option<PathBuf> {
    let path = lock_path()?;
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let pid = std::process::id();
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&path)
        .ok()?;
    let _ = writeln!(file, "{pid}");
    Some(path)
}

pub fn release_lock(path: &Path) {
    let _ = fs::remove_file(path);
}

pub fn session_dir_for(cwd: &Path) -> Option<PathBuf> {
    let home = home()?;
    let text = cwd.to_string_lossy();
    let encoded = format!("--{}--", text.trim_start_matches('/').replace('/', "-"));
    Some(
        home.join(".pi")
            .join("agent")
            .join("sessions")
            .join(encoded),
    )
}

fn is_session_file(name: &str) -> bool {
    name.ends_with(".jsonl")
        && !name.contains(".bak-")
        && !name.contains(".corrupt-")
        && !name.contains(".recovered-")
        && !name.contains(".tmp-")
}

pub fn latest_session(cwd: &Path) -> Option<PathBuf> {
    let dir = session_dir_for(cwd)?;
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for ent in fs::read_dir(&dir).ok()?.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        if !is_session_file(&name) {
            continue;
        }
        let meta = ent.metadata().ok()?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| modified > *t).unwrap_or(true) {
            best = Some((modified, ent.path()));
        }
    }
    best.map(|(_, p)| p)
}

#[derive(Debug, Default)]
pub struct SessionHealth {
    pub valid: bool,
    pub entries: usize,
    pub dangling_tool_calls: usize,
    pub duplicate_ids: usize,
    pub missing_parents: usize,
    pub bad_compactions: usize,
    pub reason: String,
}

/// Structural validation of the active leaf path. A dangling tool call or a
/// broken compaction reference is fatal: pi can resume such a file but the
/// turn boundary stalls later.
pub fn validate_session(path: &Path) -> SessionHealth {
    let mut health = SessionHealth::default();
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            health.reason = format!("unreadable: {e}");
            return health;
        }
    };
    let mut entries: Vec<Value> = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) => entries.push(v),
            Err(e) => {
                health.reason = format!("line {}: invalid JSON: {e}", idx + 1);
                return health;
            }
        }
    }
    health.entries = entries.len();
    if entries.is_empty() {
        health.reason = "empty session".to_string();
        return health;
    }
    if entries[0].get("type").and_then(Value::as_str) != Some("session") {
        health.reason = "missing session header".to_string();
        return health;
    }

    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(id) = entry.get("id").and_then(Value::as_str) else {
            continue;
        };
        if by_id.insert(id.to_string(), i).is_some() {
            health.duplicate_ids += 1;
        }
    }
    if health.duplicate_ids > 0 {
        health.reason = format!("{} duplicate entry id(s)", health.duplicate_ids);
        return health;
    }

    let leaf = entries
        .last()
        .and_then(|e| e.get("id"))
        .and_then(Value::as_str);
    let Some(leaf) = leaf else {
        health.reason = "missing leaf entry id".to_string();
        return health;
    };
    let mut path_idx: Vec<usize> = Vec::new();
    let mut cursor = Some(leaf.to_string());
    let mut seen = HashSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id.clone()) {
            health.reason = "cycle in parent chain".to_string();
            return health;
        }
        let Some(&idx) = by_id.get(&id) else {
            health.reason = format!("missing parent entry {id}");
            health.missing_parents += 1;
            return health;
        };
        path_idx.push(idx);
        cursor = entries[idx]
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string);
    }
    path_idx.reverse();

    let mut tool_results = HashSet::new();
    for &idx in &path_idx {
        let msg = entries[idx].get("message");
        if msg.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("toolResult") {
            if let Some(id) = msg
                .and_then(|m| m.get("toolCallId"))
                .and_then(Value::as_str)
            {
                tool_results.insert(id.to_string());
            }
        }
    }
    for &idx in &path_idx {
        let Some(msg) = entries[idx].get("message") else {
            continue;
        };
        if msg.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        if let Some(content) = msg.get("content").and_then(Value::as_array) {
            for part in content {
                if part.get("type").and_then(Value::as_str) != Some("toolCall") {
                    continue;
                }
                let Some(id) = part.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if !tool_results.contains(id) {
                    health.dangling_tool_calls += 1;
                }
            }
        }
    }
    for &idx in &path_idx {
        let Some(first) = entries[idx].get("firstKeptEntryId").and_then(Value::as_str) else {
            continue;
        };
        if !by_id.contains_key(first) {
            health.bad_compactions += 1;
        }
    }

    if health.dangling_tool_calls > 0 {
        health.reason = format!(
            "{} dangling tool call(s) on the active path",
            health.dangling_tool_calls
        );
        return health;
    }
    if health.bad_compactions > 0 {
        health.reason = format!("{} broken compaction reference(s)", health.bad_compactions);
        return health;
    }
    health.valid = true;
    health
}

fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn quarantine(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let dst = path.with_file_name(format!("{name}.corrupt-{}", stamp()));
    fs::rename(path, &dst).ok()?;
    Some(dst)
}

/// Conservative offline repair for small sessions whose only defect is a
/// dangling tool call (the usual result of aborting a turn). Complex
/// sessions (compactions, synthetic recovery entries, thousands of rows) are
/// quarantined instead — guessing there caused more damage.
pub fn repair_simple_session(path: &Path) -> Option<PathBuf> {
    let health = validate_session(path);
    if health.valid || health.dangling_tool_calls == 0 {
        return None;
    }
    if health.entries > 1000 || health.bad_compactions > 0 || health.duplicate_ids > 0 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let entries: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let header = entries.first()?.clone();
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    let mut by_id: HashMap<String, Value> = HashMap::new();
    for entry in entries.iter().skip(1) {
        if let Some(id) = entry.get("id").and_then(Value::as_str) {
            by_id.insert(id.to_string(), entry.clone());
        }
    }
    let leaf = entries.last()?.get("id")?.as_str()?.to_string();
    let mut path_entries = Vec::new();
    let mut cursor = Some(leaf);
    let mut seen = HashSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id.clone()) {
            return None;
        }
        let entry = by_id.get(&id)?.clone();
        cursor = entry
            .get("parentId")
            .and_then(Value::as_str)
            .map(str::to_string);
        path_entries.push(entry);
    }
    path_entries.reverse();
    // Only compactions make the tree too risky to rewrite. Prior synthetic
    // `repair-`/`recover-` entries are ordinary tool results: refusing them
    // made a repaired session UNREPAIRABLE (a second dangling tail got the
    // whole file quarantined, losing all context).
    if path_entries
        .iter()
        .any(|e| e.get("type").and_then(Value::as_str) == Some("compaction"))
    {
        return None;
    }
    let mut results = HashSet::new();
    for entry in &path_entries {
        if let Some(msg) = entry.get("message") {
            if msg.get("role").and_then(Value::as_str) == Some("toolResult") {
                if let Some(id) = msg.get("toolCallId").and_then(Value::as_str) {
                    results.insert(id.to_string());
                }
            }
        }
    }
    let mut output = vec![header];
    let mut tail: Option<String> = None;
    for entry in &path_entries {
        let mut cloned = entry.clone();
        cloned["parentId"] = tail.clone().map(Value::String).unwrap_or(Value::Null);
        let id = cloned.get("id").and_then(Value::as_str)?.to_string();
        output.push(cloned.clone());
        tail = Some(id);
        if let Some(content) = entry
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
        {
            for call in content {
                if call.get("type").and_then(Value::as_str) != Some("toolCall") {
                    continue;
                }
                let Some(call_id) = call.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if results.contains(call_id) {
                    continue;
                }
                let rid = format!("repair-{}-{}", stamp(), &call_id[..call_id.len().min(8)]);
                let payload = serde_json::json!({
                    "type": "message",
                    "id": rid.clone(),
                    "parentId": tail,
                    "timestamp": "1970-01-01T00:00:00.000Z",
                    "message": {
                        "role": "toolResult",
                        "toolCallId": call_id,
                        "toolName": call.get("name").and_then(Value::as_str).unwrap_or("tool"),
                        "content": [{"type":"text","text":"[interrupted] Tool call was interrupted before execution and was not run."}],
                        "isError": true,
                        "timestamp": 0
                    }
                });
                output.push(payload);
                tail = Some(rid);
            }
        }
    }
    let name = path.file_name()?.to_string_lossy().to_string();
    let out = path.with_file_name(format!("{name}.repaired-{}.jsonl", stamp()));
    let body: Vec<String> = output
        .iter()
        .map(|v| serde_json::to_string(v).unwrap_or_default())
        .collect();
    fs::write(&out, format!("{}\n", body.join("\n"))).ok()?;
    if validate_session(&out).valid {
        Some(out)
    } else {
        let _ = fs::remove_file(&out);
        None
    }
}

/// Prepare extra pi args: explicit session validation, or auto-resume of the
/// newest valid session. Returns a reason notice for the TUI diagnostics.
pub fn prepare_session_args(
    cwd: &Path,
    explicit_session: Option<&str>,
    new_session: bool,
    fork: Option<&str>,
) -> (Vec<String>, Option<String>) {
    let mut args = Vec::new();
    if new_session || fork.is_some() {
        return (args, None);
    }
    if let Some(session) = explicit_session {
        let path = PathBuf::from(session);
        if path.is_file() {
            let health = validate_session(&path);
            if health.valid {
                args.push("--session".to_string());
                args.push(session.to_string());
                return (args, None);
            }
            if let Some(repaired) = repair_simple_session(&path) {
                args.push("--session".to_string());
                args.push(repaired.to_string_lossy().to_string());
                return (
                    args,
                    Some(format!(
                        "session repaired: {} ({}) -> {}",
                        path.display(),
                        health.reason,
                        repaired.display()
                    )),
                );
            }
            if let Some(q) = quarantine(&path) {
                let note = format!(
                    "session quarantined: {} ({}) -> {}",
                    path.display(),
                    health.reason,
                    q.display()
                );
                // Start a fresh session rather than resuming a corrupt tree.
                args.push("--session-id".to_string());
                args.push(format!("officina-{}", stamp()));
                return (args, Some(note));
            }
        }
        args.push("--session".to_string());
        args.push(session.to_string());
        return (args, None);
    }

    let Some(latest) = latest_session(cwd) else {
        return (args, None);
    };
    let health = validate_session(&latest);
    if health.valid {
        args.push("--session".to_string());
        args.push(latest.to_string_lossy().to_string());
        return (args, None);
    }
    if let Some(repaired) = repair_simple_session(&latest) {
        args.push("--session".to_string());
        args.push(repaired.to_string_lossy().to_string());
        return (
            args,
            Some(format!(
                "session repaired: {} ({}) -> {}",
                latest.display(),
                health.reason,
                repaired.display()
            )),
        );
    }
    if let Some(q) = quarantine(&latest) {
        let note = format!(
            "session quarantined: {} ({}) -> {}; starting fresh",
            latest.display(),
            health.reason,
            q.display()
        );
        args.push("--session-id".to_string());
        args.push(format!("officina-{}", stamp()));
        return (args, Some(note));
    }
    (args, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_session(dir: &Path, name: &str, lines: &[serde_json::Value]) -> PathBuf {
        let path = dir.join(name);
        let mut file = File::create(&path).unwrap();
        for line in lines {
            writeln!(file, "{}", serde_json::to_string(line).unwrap()).unwrap();
        }
        path
    }

    fn header() -> serde_json::Value {
        serde_json::json!({"type":"session","version":3,"id":"s","timestamp":"2026-01-01T00:00:00Z","cwd":"/tmp"})
    }

    #[test]
    fn dangling_tool_call_is_invalid() {
        let dir = std::env::temp_dir().join(format!("guard-dangling-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","arguments":{}}]}}),
            ],
        );
        let health = validate_session(&path);
        assert!(!health.valid);
        assert_eq!(health.dangling_tool_calls, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn complete_tool_result_is_valid() {
        let dir = std::env::temp_dir().join(format!("guard-valid-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","arguments":{}}]}}),
                serde_json::json!({"type":"message","id":"r","parentId":"a","message":{"role":"toolResult","toolCallId":"call","toolName":"bash","content":[{"type":"text","text":"ok"}],"isError":false}}),
            ],
        );
        assert!(validate_session(&path).valid);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn simple_dangling_tail_is_repaired() {
        let dir = std::env::temp_dir().join(format!("guard-repair-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","arguments":{}}]}}),
            ],
        );
        let repaired = repair_simple_session(&path).expect("repair");
        assert!(validate_session(&repaired).valid);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn already_repaired_session_can_be_repaired_again() {
        let dir = std::env::temp_dir().join(format!("guard-repair2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a1","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call1","name":"bash","arguments":{}}]}}),
                serde_json::json!({"type":"message","id":"repair-old-call1","parentId":"a1","message":{"role":"toolResult","toolCallId":"call1","toolName":"bash","content":[{"type":"text","text":"[interrupted]"}],"isError":true}}),
                serde_json::json!({"type":"message","id":"a2","parentId":"repair-old-call1","message":{"role":"assistant","content":[{"type":"toolCall","id":"call2","name":"bash","arguments":{}}]}}),
            ],
        );
        let repaired = repair_simple_session(&path).expect("re-repair");
        assert!(validate_session(&repaired).valid);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicate_ids_are_invalid() {
        let dir = std::env::temp_dir().join(format!("guard-dup-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"x","parentId":null,"message":{"role":"user","content":[]}}),
                serde_json::json!({"type":"message","id":"x","parentId":"x","message":{"role":"user","content":[]}}),
            ],
        );
        assert!(!validate_session(&path).valid);
        let _ = fs::remove_dir_all(&dir);
    }
}
