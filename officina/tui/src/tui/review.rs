// /review-findings data — the contract the review extension writes to
// .officina/review.json (codex-qol stage 7). Read-only: the TUI renders it
// and never mutates it (plan invariant I3: versioned, corrupt-safe).

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Finding {
    pub severity: String,
    pub file: String,
    pub line: Option<String>,
    pub title: String,
    pub detail: String,
}

impl Finding {
    /// Sort rank: high first.
    pub fn rank(&self) -> u8 {
        match self.severity.as_str() {
            "high" => 0,
            "medium" => 1,
            "low" => 2,
            _ => 3,
        }
    }

    /// One-line locator: file:line when a line is present.
    pub fn locator(&self) -> String {
        match &self.line {
            Some(line) if !line.is_empty() => format!("{}:{}", self.file, line),
            _ => self.file.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReviewFindings {
    /// 0 = legacy/absent (still renders; the modal says nothing about it)
    pub version: u32,
    pub preset: String,
    pub created_at: String,
    pub summary: String,
    pub findings: Vec<Finding>,
}

impl ReviewFindings {
    /// Severity-sorted view (high → info, then file, then title).
    pub fn sorted(&self) -> Vec<&Finding> {
        let mut rows: Vec<&Finding> = self.findings.iter().collect();
        rows.sort_by(|a, b| {
            a.rank()
                .cmp(&b.rank())
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.title.cmp(&b.title))
        });
        rows
    }

    pub fn count_by(&self, severity: &str) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }
}

pub fn findings_path(cwd: &Path) -> PathBuf {
    cwd.join(".officina").join("review.json")
}

pub fn load_at(path: &Path) -> Option<ReviewFindings> {
    let raw = fs::read_to_string(path).ok()?;
    parse(&raw)
}

pub fn load(cwd: &Path) -> Option<ReviewFindings> {
    load_at(&findings_path(cwd))
}

pub fn parse(json: &str) -> Option<ReviewFindings> {
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1,
        "preset": "uncommitted",
        "created_at": "2026-11-12T13:00:00.000Z",
        "summary": "The cache layer races on shutdown.",
        "findings": [
            {"severity": "high", "file": "src/cache.rs", "line": "120-140",
             "title": "unlock skipped on error path", "detail": "use a guard"},
            {"severity": "info", "file": "a.txt", "title": "typo", "detail": "recieve"},
            {"severity": "medium", "file": "src/main.rs", "title": "missing test", "detail": "add one"}
        ]
    }"#;

    #[test]
    fn parses_and_sorts_by_severity() {
        let f = parse(SAMPLE).expect("sample parses");
        assert_eq!(f.version, 1);
        assert_eq!(f.preset, "uncommitted");
        let order: Vec<&str> = f.sorted().iter().map(|x| x.severity.as_str()).collect();
        assert_eq!(order, vec!["high", "medium", "info"]);
        assert_eq!(f.count_by("high"), 1);
        assert_eq!(f.sorted()[0].locator(), "src/cache.rs:120-140");
    }

    #[test]
    fn corrupt_or_missing_is_none() {
        assert!(parse("not json").is_none());
        assert!(parse(r#"{"version":1,"findings":"nope"}"#).is_none());
        assert!(load_at(Path::new("/nonexistent/review.json")).is_none());
    }

    #[test]
    fn legacy_v0_still_renders() {
        let f = parse(r#"{"summary":"s","findings":[{"severity":"low","file":"a","title":"t","detail":"d"}]}"#)
            .expect("legacy parses");
        assert_eq!(f.version, 0);
        assert_eq!(f.sorted().len(), 1);
    }
}
