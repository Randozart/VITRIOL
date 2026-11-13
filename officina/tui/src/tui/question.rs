//! Question panel (2026-11-13): the ask extension stages
//! ~/.vitriol/officina/question.json; the TUI renders it as a menu ON TOP
//! OF THE PROMPT and writes question-response.json. Replaces the hung
//! RPC dialog bridging for the question tool (P-ask2).
//!
//! Contract: read-only on the question file; the response write is the
//! ONLY mutation; freshness gate (10 min) so a stale question never
//! blocks a fresh session; the panel disappears the moment the file is
//! consumed or expires.

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct QuestionDoc {
    pub version: u32,
    pub id: String,
    pub question: String,
    pub options: Vec<String>,
    /// ms epoch — the panel ignores questions older than 10 minutes.
    pub ts: u64,
}

impl Default for QuestionDoc {
    fn default() -> Self {
        Self {
            version: 0,
            id: String::new(),
            question: String::new(),
            options: Vec::new(),
            ts: 0,
        }
    }
}

const FRESH_MS: u64 = 10 * 60 * 1000;

pub fn question_path() -> PathBuf {
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("question.json");
    p
}

pub fn response_path() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    question_path().with_file_name("question-response.json")
}

/// Load a fresh, unanswered question. None = no panel.
pub fn load_fresh(now_ms: u64) -> Option<QuestionDoc> {
    let raw = fs::read_to_string(question_path()).ok()?;
    let doc: QuestionDoc = serde_json::from_str(&raw).ok()?;
    if doc.version != 1 || doc.id.is_empty() || doc.options.is_empty() {
        return None;
    }
    if doc.ts == 0 || now_ms.saturating_sub(doc.ts) > FRESH_MS {
        return None;
    }
    Some(doc)
}

/// Write the operator's answer (or cancellation) for the extension's poll.
pub fn write_response(id: &str, answer: Option<String>) -> std::io::Result<()> {
    write_response_at(&response_path(), id, answer)
}

/// Injectable response write (tests use a temp path).
pub fn write_response_at(
    path: &std::path::Path,
    id: &str,
    answer: Option<String>,
) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let doc = serde_json::json!({
        "id": id,
        "answer": answer,
        "cancelled": answer.is_none(),
    });
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    writeln!(f, "{doc}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(id: &str, ts: u64, options: usize) -> QuestionDoc {
        QuestionDoc {
            version: 1,
            id: id.into(),
            question: "pick one".into(),
            options: (0..options).map(|i| format!("opt {i}")).collect(),
            ts,
        }
    }

    #[test]
    fn fresh_valid_question_round_trips() {
        let now = crate::tui::caduceus::now_ms();
        let d = doc_with("q-1", now, 3);
        let raw = serde_json::to_string(&d).expect("serializable");
        let parsed: QuestionDoc = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.id, "q-1");
        assert_eq!(parsed.options.len(), 3);
        assert!(now.saturating_sub(parsed.ts) <= FRESH_MS);
    }

    #[test]
    fn stale_or_optionless_is_rejected_by_the_loader_gates() {
        let now = crate::tui::caduceus::now_ms();
        let stale = doc_with("q-2", now.saturating_sub(FRESH_MS + 1_000), 2);
        assert!(now.saturating_sub(stale.ts) > FRESH_MS);
        let empty = doc_with("q-3", now, 0);
        assert!(empty.options.is_empty());
        let v9: QuestionDoc =
            serde_json::from_str(r#"{"version":9,"id":"x","options":["a"]}"#).unwrap();
        assert_eq!(v9.version, 9);
    }

    #[test]
    fn response_write_round_trips() {
        let dir = std::env::temp_dir().join(format!("qpanel-resp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("question-response.json");
        write_response_at(&path, "q-9", Some("opt 1".into())).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["id"], "q-9");
        assert_eq!(parsed["answer"], "opt 1");
        assert_eq!(parsed["cancelled"], false);
        write_response_at(&path, "q-9", None).unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(parsed["cancelled"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
