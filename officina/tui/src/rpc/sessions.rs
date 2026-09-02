// Session store reader — lists resumable pi sessions for the current cwd.
//
// pi stores sessions as append-only JSONL trees under
// ~/.pi/agent/sessions/<encoded-cwd>/*.jsonl. The dir-name encoding is a
// pi implementation detail, so instead of re-encoding the cwd we scan all
// session dirs and match the `cwd` field on each file's first
// {"type":"session",...} header line. RPC mode has no session-list command,
// so this is the canonical source for the /resume picker.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};

/// One resumable session.
#[derive(Debug, Clone)]
pub struct SessionEntry {
    /// Absolute path to the .jsonl file (switch_session target).
    pub path: PathBuf,
    /// First user message text, truncated — the picker's title.
    pub title: String,
    /// File modified time (sort key).
    pub modified: SystemTime,
    /// Rough message count (lines with "type":"message").
    pub msg_count: usize,
}

fn sessions_root() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME not set")?;
    Ok(Path::new(&home).join(".pi").join("agent").join("sessions"))
}

/// List sessions for `cwd`, most recently modified first, capped at 50.
pub fn list(cwd: &Path) -> Result<Vec<SessionEntry>> {
    let root = sessions_root()?;
    let mut out: Vec<SessionEntry> = Vec::new();
    let cwd_str = cwd.to_string_lossy().to_string();

    let dirs = match fs::read_dir(&root) {
        Ok(d) => d,
        Err(_) => return Ok(out), // no store yet — empty picker
    };

    for dir in dirs.flatten() {
        if !dir.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let files = match fs::read_dir(dir.path()) {
            Ok(f) => f,
            Err(_) => continue,
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().map(|e| e != "jsonl").unwrap_or(true) {
                continue;
            }
            if let Some(entry) = parse_session_file(&path, &cwd_str) {
                out.push(entry);
            }
        }
    }

    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out.truncate(50);
    Ok(out)
}

/// Parse one session file: header cwd must match; extract title + count.
fn parse_session_file(path: &Path, cwd: &str) -> Option<SessionEntry> {
    let content = fs::read_to_string(path).ok()?;
    let mut header_cwd: Option<String> = None;
    let mut title = String::new();
    let mut msg_count = 0usize;

    for line in content.lines().take(400) {
        if header_cwd.is_none() {
            // Header line: {"type":"session","version":..,"id":..,"timestamp":..,"cwd":".."}
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if v.get("type").and_then(|t| t.as_str()) == Some("session") {
                    header_cwd = v.get("cwd").and_then(|c| c.as_str()).map(String::from);
                    continue;
                }
            }
            continue;
        }
        // After header: count messages, find first user text for the title.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            if v.get("type").and_then(|t| t.as_str()) == Some("message") {
                msg_count += 1;
                if title.is_empty() {
                    if let Some(msg) = v.get("message") {
                        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                            title = extract_text(msg.get("content").unwrap_or(&serde_json::Value::Null));
                        }
                    }
                }
            }
        }
        if msg_count > 400 {
            break; // enough for a title + rough count
        }
    }

    // Header must exist and cwd must match the picker's working directory.
    if header_cwd.as_deref() != Some(cwd) {
        return None;
    }

    let modified = fs::metadata(path).and_then(|m| m.modified()).ok()?;
    if title.is_empty() {
        title = "(empty session)".to_string();
    }
    let title: String = title.chars().take(70).collect();

    Some(SessionEntry {
        path: path.to_path_buf(),
        title,
        modified,
        msg_count,
    })
}

/// Extract text from message content (string or array of blocks).
fn extract_text(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}
