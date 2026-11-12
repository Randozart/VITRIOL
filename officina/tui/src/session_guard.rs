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

// ---- Cross-process session-store lock (flock, kernel-held) ----
//
// The lock domain is the session STORE (pi keeps one sessions/<--cwd-->
// directory per project): two TUIs in the same project contend, two in
// different projects never do. The lock is an advisory flock held by an
// open File for the TUI's whole lifetime — the kernel releases it even on
// crash/SIGKILL, so a stale lock file can never wedge startup.
//
// The pre-2026-11-11 guard only WROTE a pid into a global file and never
// checked it, which is how two TUIs ended up appending to the same session
// jsonl (the top corruption driver; see
// .opencode/plans/officina-multisession-hardening-2026-11-11.md, I2/I3).

fn lock_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OFFICINA_LOCK_DIR") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    Some(home()?.join(".vitriol").join("officina").join("locks"))
}

fn store_lock_path(store: &Path) -> Option<PathBuf> {
    let name = store.file_name()?.to_string_lossy().to_string();
    Some(lock_dir()?.join(format!("{name}.lock")))
}

/// The process-lifetime session-store lock. The `File` must stay open for
/// as long as the TUI runs: the flock (and therefore the exclusion)
/// disappears when it closes.
pub struct SessionLock {
    /// None when a live peer owns the store, or when the lock filesystem
    /// misbehaved (enforcement degrades to the old behavior rather than
    /// blocking every startup).
    file: Option<File>,
    /// The store this process locked (None when it did not acquire).
    store: Option<PathBuf>,
}

impl SessionLock {
    /// The session store THIS process holds the lock for.
    pub fn locked_store(&self) -> Option<&Path> {
        self.store.as_deref()
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // Mirror hygiene: clear the doctor's global pid file only when it
        // still names THIS process (a peer may have overwritten it after
        // our mirror write — deleting it then would un-shield the peer).
        if let Some(path) = lock_path() {
            if let Ok(text) = fs::read_to_string(&path) {
                if text.trim() == std::process::id().to_string() {
                    let _ = fs::remove_file(&path);
                }
            }
        }
        // `file` closes here; the flock releases with it (and at process
        // death regardless, which is the whole point of flock).
    }
}

/// Acquire the per-session-store lock for `cwd`'s session store. Returns a
/// lock with `locked_store() == None` when a live peer already holds the
/// store (the caller must then not resume or mutate that store's files).
pub fn acquire_session_lock(cwd: &Path) -> SessionLock {
    let store = session_dir_for(cwd);
    let mut file = None;
    if let Some(store) = &store {
        if let Some(path) = store_lock_path(store) {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(mut f) = OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)
            {
                if f.try_lock().is_err() {
                    // A live peer owns this store — we hold NOTHING.
                    return SessionLock {
                        file: None,
                        store: None,
                    };
                }
                let _ = f.set_len(0);
                let _ = writeln!(f, "{} {}", std::process::id(), store.display());
                file = Some(f);
            }
        }
    }
    // Diagnostics mirror for the offline doctor (reads the pid and probes
    // it with kill(pid, 0) — caduceus.mjs tuiRunning).
    if file.is_some() {
        if let Some(path) = lock_path() {
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            if let Ok(mut g) = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&path)
            {
                let _ = writeln!(g, "{}", std::process::id());
            }
        }
    }
    SessionLock { file, store }
}

/// True when ANOTHER live process holds the flock for `store`'s lock file.
/// This process's own lock never counts as busy. Best-effort: an unreadable
/// lock path counts as free (degraded, not wedged).
fn store_lock_busy(store: &Path, own_store: Option<&Path>) -> bool {
    if own_store.map(|p| p == store).unwrap_or(false) {
        return false;
    }
    let Some(path) = store_lock_path(store) else {
        return false;
    };
    let Ok(f) = OpenOptions::new().write(true).open(&path) else {
        return false; // no lock file yet -> nobody holds it
    };
    f.try_lock().is_err()
}

/// Refusal note when a live peer holds the store that `path` lives in.
fn peer_holds_store(path: &Path, own: &SessionLock) -> Option<String> {
    let store = path.parent()?;
    if store_lock_busy(store, own.locked_store()) {
        Some(format!(
            "another officina TUI is live on this session store ({}) — starting a fresh session (use /resume to switch later)",
            store.display()
        ))
    } else {
        None
    }
}

fn fresh_session_args(note: String) -> (Vec<String>, Option<String>) {
    (
        vec!["--session-id".to_string(), format!("officina-{}", stamp())],
        Some(note),
    )
}

// ---- Runtime peer fence (2026-11-11, runtime-fence-batch) ----
//
// The flock fences STARTUP, but /resume and session imports switch files
// while running. A peer TUI that started first holds our store; its
// ACTIVE session file shows up in our /resume list, and switching onto
// it would put two pi writers on one jsonl. The routing registry (which
// every TUI publishes per decision, sessionId = session file stem) tells
// us which file each peer is live on — that is the fence.

const PEER_TTL_MS: u64 = 60_000;

fn registry_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("OFFICINA_ROUTING_REGISTRY_DIR") {
        let p = PathBuf::from(dir);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    Some(
        home()?
            .join(".vitriol")
            .join("officina")
            .join("routing-registry.d"),
    )
}

fn legacy_registry_file() -> Option<PathBuf> {
    if let Some(f) = std::env::var_os("OFFICINA_ROUTING_REGISTRY") {
        let p = PathBuf::from(f);
        if !p.as_os_str().is_empty() {
            return Some(p);
        }
    }
    Some(
        home()?
            .join(".vitriol")
            .join("officina")
            .join("routing-registry.json"),
    )
}

/// Session-file stems (no `.jsonl`) that a LIVE peer TUI is on, read from
/// the routing registry (shards + legacy single file). Self is excluded by
/// pid; entries older than 60 s are stale by definition. Best-effort:
/// unreadable registry yields an empty list (no fence, same as before).
fn live_peer_session_stems() -> Vec<String> {
    let now = stamp();
    let mut stems = Vec::new();
    let mut sources: Vec<PathBuf> = Vec::new();
    if let Some(dir) = registry_dir() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().to_string();
                if name.ends_with(".json") {
                    sources.push(ent.path());
                }
            }
        }
    }
    if let Some(f) = legacy_registry_file() {
        sources.push(f);
    }
    for path in sources {
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(list) = v.get("entries").and_then(Value::as_array) else {
            continue;
        };
        for e in list {
            let pid = e.get("tuiPid").and_then(Value::as_u64);
            let session = e.get("sessionId").and_then(Value::as_str);
            let ts = e.get("ts").and_then(Value::as_u64);
            let (Some(pid), Some(session), Some(ts)) = (pid, session, ts) else {
                continue;
            };
            if pid == u64::from(std::process::id()) {
                continue;
            }
            if now.saturating_sub(ts) > PEER_TTL_MS {
                continue;
            }
            if !stems.iter().any(|s| s == session) {
                stems.push(session.to_string());
            }
        }
    }
    stems
}

/// True when `stem` is a session file a live peer is currently writing.
fn session_held_by_peer(stem: &str, peers: &[String]) -> bool {
    peers.iter().any(|s| s == stem)
}

/// Refusal notice when a live peer holds the session at `path`.
pub fn peer_fenced_notice(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let stem = name.strip_suffix(".jsonl")?.to_string();
    let peers = live_peer_session_stems();
    if session_held_by_peer(&stem, &peers) {
        Some(format!(
            "session is live in another officina ({stem}) — not switched"
        ))
    } else {
        None
    }
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

/// Surgical, append-only repair (2026-11-12): the only defect a restart can
/// introduce is tool calls on the active path without results (pi killed
/// mid-turn by a TUI restart). Appending one synthetic toolResult per
/// dangling call fixes the tree while dropping NOTHING — so compaction-
/// bearing and large sessions (which rewrite-based repair refuses) keep
/// their context instead of being quarantined. Returns the patched file.
pub fn patch_dangling_session(path: &Path) -> Option<PathBuf> {
    let health = validate_session(path);
    if health.valid || health.dangling_tool_calls == 0 || health.duplicate_ids > 0 {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let mut entries: Vec<Value> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    let header = entries.first()?.clone();
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }
    // Recompute the active path and the dangling call ids (validate_session's
    // reason string is coarse; we need the ids themselves, in path order).
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(id) = e.get("id").and_then(Value::as_str) {
            by_id.insert(id.to_string(), i);
        }
    }
    let leaf = entries
        .last()?
        .get("id")
        .and_then(Value::as_str)?
        .to_string();
    let mut results: HashSet<String> = HashSet::new();
    let mut dangling: Vec<String> = Vec::new();
    {
        let mut path_idx: Vec<usize> = Vec::new();
        let mut cursor = Some(leaf.clone());
        let mut seen = HashSet::new();
        while let Some(id) = cursor {
            if !seen.insert(id.clone()) {
                return None;
            }
            let Some(&idx) = by_id.get(&id) else {
                return None; // missing parent: the rewriter handles this one
            };
            path_idx.push(idx);
            cursor = entries[idx]
                .get("parentId")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        for &idx in &path_idx {
            let msg = entries[idx].get("message");
            if msg.and_then(|m| m.get("role")).and_then(Value::as_str) == Some("toolResult") {
                if let Some(id) = msg
                    .and_then(|m| m.get("toolCallId"))
                    .and_then(Value::as_str)
                {
                    results.insert(id.to_string());
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
                    if let Some(id) = part.get("id").and_then(Value::as_str) {
                        let owned = id.to_string();
                        if !results.contains(&owned) && !dangling.contains(&owned) {
                            dangling.push(owned);
                        }
                    }
                }
            }
        }
    }
    if dangling.is_empty() {
        return None; // the defect is something else — not ours to patch
    }
    let mut now = stamp();
    let mut tail = leaf;
    for call_id in &dangling {
        let rid = format!("repair-{}-{}", now, &call_id[..call_id.len().min(8)]);
        now += 1;
        entries.push(serde_json::json!({
            "type": "message",
            "id": rid,
            "parentId": tail,
            "timestamp": "1970-01-01T00:00:00.000Z",
            "message": {
                "role": "toolResult",
                "toolCallId": call_id,
                "toolName": "tool",
                "content": [{"type":"text","text":"[interrupted] Tool call was interrupted before execution and was not run."}],
                "isError": true,
                "timestamp": 0
            }
        }));
        tail = rid;
    }
    let name = path.file_name()?.to_string_lossy().to_string();
    let out = path.with_file_name(format!("{name}.patched-{}.jsonl", stamp()));
    let body: Vec<String> = entries
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
///
/// `own_lock` is this TUI's session-store lock: when a live peer holds the
/// store a target session lives in, we neither resume nor repair/quarantine
/// that file (I2/I3) — a live pi may be appending to it — and start a fresh
/// session instead.
pub fn prepare_session_args(
    cwd: &Path,
    explicit_session: Option<&str>,
    new_session: bool,
    fork: Option<&str>,
    own_lock: &SessionLock,
) -> (Vec<String>, Option<String>) {
    let mut args = Vec::new();
    if new_session || fork.is_some() {
        return (args, None);
    }
    if let Some(session) = explicit_session {
        let path = PathBuf::from(session);
        if path.is_file() {
            if let Some(note) = peer_holds_store(&path, own_lock) {
                return fresh_session_args(note);
            }
            let health = validate_session(&path);
            if health.valid {
                args.push("--session".to_string());
                args.push(session.to_string());
                return (args, None);
            }
            if let Some(patched) = patch_dangling_session(&path) {
                args.push("--session".to_string());
                args.push(patched.to_string_lossy().to_string());
                return (
                    args,
                    Some(format!(
                        "session patched (append-only, context kept): {} ({}) -> {}",
                        path.display(),
                        health.reason,
                        patched.display()
                    )),
                );
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
                return fresh_session_args(note);
            }
        }
        args.push("--session".to_string());
        args.push(session.to_string());
        return (args, None);
    }

    let Some(latest) = latest_session(cwd) else {
        return (args, None);
    };
    if let Some(note) = peer_holds_store(&latest, own_lock) {
        return fresh_session_args(note);
    }
    let health = validate_session(&latest);
    if health.valid {
        args.push("--session".to_string());
        args.push(latest.to_string_lossy().to_string());
        return (args, None);
    }
    if let Some(patched) = patch_dangling_session(&latest) {
        args.push("--session".to_string());
        args.push(patched.to_string_lossy().to_string());
        return (
            args,
            Some(format!(
                "session patched (append-only, context kept): {} ({}) -> {}",
                latest.display(),
                health.reason,
                patched.display()
            )),
        );
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
        return fresh_session_args(note);
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
    fn patch_dangling_keeps_compaction_context() {
        // 2026-11-12: the briev-TUI restart quarantined 5.6 MB because the
        // rewriter refuses compactions; the append-only patch must keep them.
        let dir = std::env::temp_dir().join(format!("guard-patch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = write_session(
            &dir,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call_1","name":"bash","arguments":{}}]}}),
                serde_json::json!({"type":"compaction","id":"c","parentId":"a","firstKeptEntryId":"u","summary":"kept going"}),
            ],
        );
        let health = validate_session(&path);
        assert!(!health.valid);
        assert_eq!(health.dangling_tool_calls, 1);
        let patched = patch_dangling_session(&path).expect("patched");
        let out = validate_session(&patched);
        assert!(out.valid, "patched session must validate: {}", out.reason);
        let text = fs::read_to_string(&patched).unwrap();
        assert!(text.contains("\"compaction\""), "compaction entry kept");
        assert!(text.contains("[interrupted]"));
        assert!(text.contains("call_1"));
        let _ = fs::remove_file(&patched);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn patch_refuses_duplicate_ids_and_valid_sessions() {
        let dir = std::env::temp_dir().join(format!("guard-patch2-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        // duplicate ids: not appendable, rewriter territory
        let dup = write_session(
            &dir,
            "dup.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi again"}]}}),
            ],
        );
        assert!(patch_dangling_session(&dup).is_none());
        // fully valid session: nothing to patch
        let ok = write_session(
            &dir,
            "ok.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
            ],
        );
        assert!(validate_session(&ok).valid);
        assert!(patch_dangling_session(&ok).is_none());
        let _ = fs::remove_dir_all(&dir);
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

    /// One body for all env-mutating lock cases: set_var is process-global
    /// and the test harness runs tests in parallel threads.
    #[test]
    fn session_store_lock_lifecycle_and_peer_refusal() {
        let tmp = std::env::temp_dir().join(format!("guard-lock-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let lock_dir = tmp.join("locks");
        let mirror = tmp.join("tui.lock");
        std::env::set_var("OFFICINA_LOCK_DIR", &lock_dir);
        std::env::set_var("OFFICINA_TUI_LOCK", &mirror);

        let cwd_a = tmp.join("proj-a");
        let cwd_b = tmp.join("proj-b");
        fs::create_dir_all(&cwd_a).unwrap();
        fs::create_dir_all(&cwd_b).unwrap();

        // 1. Same store contends (second fd in the SAME process conflicts,
        //    which is exactly what a second TUI would hit), a different
        //    store does not, and drop releases.
        let l1 = acquire_session_lock(&cwd_a);
        assert!(
            l1.locked_store().is_some(),
            "first acquire must own the store"
        );
        let l2 = acquire_session_lock(&cwd_a);
        assert!(
            l2.locked_store().is_none(),
            "peer acquire must find the store held"
        );
        let l3 = acquire_session_lock(&cwd_b);
        assert!(
            l3.locked_store().is_some(),
            "a different store must not contend"
        );
        drop(l1);
        let l4 = acquire_session_lock(&cwd_a);
        assert!(l4.locked_store().is_some(), "release must free the store");
        drop(l4);
        drop(l3);

        // 2. A live peer on the store blocks resume AND mutation of the
        //    session file: prepare refuses, starts fresh, leaves the file.
        let store_a = session_dir_for(&cwd_a).unwrap();
        fs::create_dir_all(&store_a).unwrap();
        let session = write_session(
            &store_a,
            "s.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
                serde_json::json!({"type":"message","id":"a","parentId":"u","message":{"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","arguments":{}}]}}),
            ],
        );
        let peer_path = store_lock_path(&store_a).unwrap();
        let mut peer = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&peer_path)
            .unwrap();
        peer.try_lock().expect("simulate live peer");

        let outsider = acquire_session_lock(&cwd_b); // holds a DIFFERENT store
        let (args, notice) = prepare_session_args(
            &cwd_a,
            Some(session.to_str().unwrap()),
            false,
            None,
            &outsider,
        );
        assert!(
            notice
                .as_deref()
                .unwrap_or("")
                .contains("another officina TUI"),
            "peer-held store must produce a refusal notice, got {notice:?}"
        );
        assert!(
            args.iter().any(|a| a == "--session-id"),
            "refusal must start a fresh session, got {args:?}"
        );
        assert!(
            session.is_file(),
            "quarantine must NOT run on a peer-held store"
        );

        // 3. An invalid session in a peer-held store is likewise untouched.
        let (args, notice) = prepare_session_args(
            &cwd_a,
            Some(session.to_str().unwrap()),
            false,
            None,
            &outsider,
        );
        assert!(args.iter().any(|a| a == "--session-id"));
        assert!(notice.is_some());
        assert!(session.is_file());

        // 4. A store the peer does NOT hold resumes normally.
        let store_b = session_dir_for(&cwd_b).unwrap();
        fs::create_dir_all(&store_b).unwrap();
        let ok_session = write_session(
            &store_b,
            "ok.jsonl",
            &[
                header(),
                serde_json::json!({"type":"message","id":"u","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}),
            ],
        );
        let (args, notice) = prepare_session_args(
            &cwd_b,
            Some(ok_session.to_str().unwrap()),
            false,
            None,
            &outsider,
        );
        assert!(
            notice.is_none(),
            "free store must resume cleanly, got {notice:?}"
        );
        assert_eq!(
            args,
            vec![
                "--session".to_string(),
                ok_session.to_string_lossy().to_string()
            ]
        );

        peer.unlock().unwrap();
        drop(peer);
        drop(outsider);
        std::env::remove_var("OFFICINA_LOCK_DIR");
        std::env::remove_var("OFFICINA_TUI_LOCK");
        let _ = fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod peer_fence_tests {
    use super::*;

    /// Env-mutating; isolated body like the lock lifecycle test.
    #[test]
    fn live_peer_stems_from_shards_and_legacy_file() {
        let tmp = std::env::temp_dir().join(format!("guard-fence-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let shards = tmp.join("shards");
        fs::create_dir_all(&shards).unwrap();
        std::env::set_var("OFFICINA_ROUTING_REGISTRY_DIR", &shards);
        std::env::set_var("OFFICINA_ROUTING_REGISTRY", tmp.join("legacy.json"));

        let now = stamp();
        let mine = u64::from(std::process::id());
        // Fresh peer shard, expired peer shard, our own fresh entry.
        writeFileSync(
            &shards.join("500.json"),
            &serde_json::json!({"entries": [
                {"ts": now, "tuiPid": 500, "sessionId": "peer-active", "tier": "local", "model": "m", "ctxTokens": 1},
                {"ts": now - 61_000, "tuiPid": 500, "sessionId": "peer-stale", "tier": "local", "model": "m", "ctxTokens": 1}
            ]})
            .to_string(),
        );
        writeFileSync(
            &shards.join("501.json"),
            &serde_json::json!({"entries": [
                {"ts": now - 5_000, "tuiPid": 501, "sessionId": "peer-two", "tier": "local", "model": "m", "ctxTokens": 1}
            ]})
            .to_string(),
        );
        // Legacy single file: fresh OUR-pid entry (must be excluded) + a
        // fresh foreign one (must be included — transition compat).
        writeFileSync(
            &tmp.join("legacy.json"),
            &serde_json::json!({"entries": [
                {"ts": now, "tuiPid": mine, "sessionId": "mine-own", "tier": "local", "model": "m", "ctxTokens": 1},
                {"ts": now - 1_000, "tuiPid": 502, "sessionId": "legacy-peer", "tier": "cloud", "model": "m", "ctxTokens": 1}
            ]})
            .to_string(),
        );

        let stems = live_peer_session_stems();
        assert!(
            stems.iter().any(|s| s == "peer-active"),
            "fresh peer shard missed: {stems:?}"
        );
        assert!(
            stems.iter().any(|s| s == "peer-two"),
            "second shard missed: {stems:?}"
        );
        assert!(
            stems.iter().any(|s| s == "legacy-peer"),
            "legacy file missed: {stems:?}"
        );
        assert!(
            !stems.iter().any(|s| s == "peer-stale"),
            "expired entry must be skipped: {stems:?}"
        );
        assert!(
            !stems.iter().any(|s| s == "mine-own"),
            "own pid must be skipped: {stems:?}"
        );

        assert!(session_held_by_peer("peer-active", &stems));
        assert!(!session_held_by_peer("not-a-session", &stems));

        std::env::remove_var("OFFICINA_ROUTING_REGISTRY_DIR");
        std::env::remove_var("OFFICINA_ROUTING_REGISTRY");
        let _ = fs::remove_dir_all(&tmp);
    }

    fn writeFileSync(path: &Path, body: &str) {
        use std::io::Write;
        let mut f = File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }
}
