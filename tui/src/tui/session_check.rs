//! Session-format self-check for `/doctor` (2026-11-14).
//!
//! Mirrors the TS `validateSessionText` rules on the Rust side: header
//! version, unique ids, an unbroken parent chain, valid roles, and every
//! toolCall paired with its toolResult (both directions). Read-only; the
//! importer/compactor own repairs.

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct SessionCheck {
    pub path: Option<PathBuf>,
    pub entries: usize,
    pub messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    pub bad_lines: usize,
    pub issues: Vec<String>,
}

impl SessionCheck {
    pub fn ok(&self) -> bool {
        self.issues.is_empty() && self.bad_lines == 0
    }
}

/// Structural check over the JSONL text.
pub fn check_session_text(text: &str) -> SessionCheck {
    let mut out = SessionCheck::default();
    let mut header_version: Option<i64> = None;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut prev: Option<String> = None;
    let mut calls: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut results: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                out.bad_lines += 1;
                if out.issues.len() < 8 {
                    out.issues.push(format!("line {}: invalid JSON", index + 1));
                }
                continue;
            }
        };
        let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if kind == "session" {
            header_version = value.get("version").and_then(|v| v.as_i64());
            continue;
        }
        out.entries += 1;
        let Some(id) = value.get("id").and_then(|v| v.as_str()) else {
            if out.issues.len() < 8 {
                out.issues.push(format!("entry {}: missing id", index + 1));
            }
            continue;
        };
        if !seen.insert(id.to_string()) && out.issues.len() < 8 {
            out.issues.push(format!("duplicate id {id}"));
        }
        let parent = value.get("parentId").and_then(|v| v.as_str());
        if parent != prev.as_deref() && out.issues.len() < 8 {
            out.issues.push(format!("chain break at {id}"));
        }
        prev = Some(id.to_string());

        let message = value.get("message");
        let role = message.and_then(|m| m.get("role")).and_then(|v| v.as_str());
        if kind == "message" {
            out.messages += 1;
            match role {
                Some("user") | Some("assistant") | Some("toolResult") | Some("system") => {}
                other => {
                    if out.issues.len() < 8 {
                        out.issues
                            .push(format!("entry {id}: bad role {}", other.unwrap_or("?")));
                    }
                }
            }
        }
        if role == Some("assistant") {
            if let Some(content) = message
                .and_then(|m| m.get("content"))
                .and_then(|v| v.as_array())
            {
                for part in content {
                    if part.get("type").and_then(|v| v.as_str()) == Some("toolCall") {
                        if let Some(call) = part.get("id").and_then(|v| v.as_str()) {
                            calls.insert(call.to_string());
                        }
                    }
                }
            }
        }
        if role == Some("toolResult") {
            if let Some(call) = message
                .and_then(|m| m.get("toolCallId"))
                .and_then(|v| v.as_str())
            {
                results.insert(call.to_string());
            }
        }
    }

    match header_version {
        Some(3) => {}
        Some(other) => out.issues.push(format!("session version {other} != 3")),
        None => out.issues.push("missing session header".to_string()),
    }
    out.tool_calls = calls.len();
    out.tool_results = results.len();
    for call in calls.iter() {
        if !results.contains(call) && out.issues.len() < 8 {
            out.issues
                .push(format!("toolCall {call} has no toolResult"));
        }
    }
    for result in results.iter() {
        if !calls.contains(result) && out.issues.len() < 8 {
            out.issues
                .push(format!("toolResult {result} has no toolCall"));
        }
    }
    out
}

/// Directory pi uses for a cwd (sanitized path appears in the name).
fn sessions_root() -> PathBuf {
    let mut root = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    root.push(".pi");
    root.push("agent");
    root.push("sessions");
    root
}

pub fn find_session_dir(cwd: &Path) -> Option<PathBuf> {
    let root = sessions_root();
    let sanitized = cwd.to_string_lossy().replace('/', "-");
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(&root).ok()?.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.contains(&sanitized) {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, path)| path)
}

fn latest_jsonl(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if best.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            best = Some((mtime, path));
        }
    }
    best.map(|(_, path)| path)
}

/// Check the newest session file of `cwd` (None when no session exists yet).
pub fn check_current_session(cwd: &Path) -> SessionCheck {
    let Some(dir) = find_session_dir(cwd) else {
        return SessionCheck::default();
    };
    let Some(path) = latest_jsonl(&dir) else {
        return SessionCheck::default();
    };
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut check = check_session_text(&text);
    check.path = Some(path);
    check
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_valid_trace() {
        let text = [
            r#"{"type":"session","version":3}"#,
            r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"hi"}}"#,
            r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":[{"type":"toolCall","id":"c1"}]}}"#,
            r#"{"type":"message","id":"c","parentId":"b","message":{"role":"toolResult","toolCallId":"c1","content":[]}}"#,
        ]
        .join("\n");
        let check = check_session_text(&text);
        assert!(check.ok(), "{:?}", check.issues);
        assert_eq!(check.messages, 3);
        assert_eq!((check.tool_calls, check.tool_results), (1, 1));
    }

    #[test]
    fn detects_chain_breaks_bad_json_and_orphans() {
        let text = [
            r#"{"type":"session","version":3}"#,
            r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"hi"}}"#,
            "not json",
            r#"{"type":"message","id":"b","parentId":"zzz","message":{"role":"assistant","content":[]}}"#,
            r#"{"type":"message","id":"c","parentId":"b","message":{"role":"toolResult","toolCallId":"missing","content":[]}}"#,
        ]
        .join("\n");
        let check = check_session_text(&text);
        assert!(!check.ok());
        assert_eq!(check.bad_lines, 1);
        assert!(check.issues.iter().any(|i| i.contains("chain break")));
        assert!(check.issues.iter().any(|i| i.contains("no toolCall")));
    }

    #[test]
    fn rejects_wrong_version_and_missing_header() {
        let wrong = check_session_text(r#"{"type":"session","version":2}"#);
        assert!(wrong.issues.iter().any(|i| i.contains("version 2")));
        let missing = check_session_text("");
        assert!(missing
            .issues
            .iter()
            .any(|i| i.contains("missing session header")));
    }
}
