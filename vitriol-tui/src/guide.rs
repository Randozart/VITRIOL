//! GUIDE tab: curated optimization docs.
//!
//! Discovers markdown under the repo `docs/optimizations/` — one doc per
//! optimization (lever → config key, silicon rationale, measured result, status,
//! undo path). This is intentionally a curated set, NOT the whole `docs/` tree.
//! A selected row's markdown renders (CommonMark) into a scrolled reader pane.
//! This module only reads and classifies; it never spawns or edits.

use std::fs;
use std::path::PathBuf;

use crate::config::Config;

/// One discoverable guide entry.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Display title (first `# ` heading, else the file name).
    pub title: String,
    /// Absolute path of the markdown file.
    pub path: PathBuf,
    /// One-line summary (first paragraph), for the index pane.
    pub summary: Option<String>,
    /// The first `PROVENANCE:` line content, when present.
    pub provenance: Option<String>,
}

/// The directory of curated optimization docs.
fn optimizations_dir(cfg: &Config) -> PathBuf {
    cfg.repo_root.join("docs/optimizations")
}

/// Discover every optimization doc, sorted by title. Missing directory yields
/// an empty list (never errors).
pub fn discover(cfg: &Config) -> Vec<Doc> {
    let Ok(entries) = fs::read_dir(optimizations_dir(cfg)) else {
        return Vec::new();
    };
    let mut out: Vec<Doc> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .map(load_doc)
        .collect();
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

/// Read a doc and derive its title, summary, and provenance line.
fn load_doc(path: PathBuf) -> Doc {
    let text = fs::read_to_string(&path).unwrap_or_default();
    let title = first_heading(&text).unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    });
    Doc {
        title,
        summary: first_paragraph(&text),
        provenance: provenance_line(&text),
        path,
    }
}

/// The first `# ` heading, trimmed.
fn first_heading(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("# "))
        .map(|l| l.trim_start_matches('#').trim().to_string())
}

/// The first non-heading, non-comment, non-fenced paragraph, single-lined.
fn first_paragraph(text: &str) -> Option<String> {
    let mut in_fence = false;
    text.lines().find_map(|l| {
        let t = l.trim();
        if t.starts_with("```") {
            in_fence = !in_fence;
            return None;
        }
        if in_fence || t.starts_with('#') || t.starts_with("//") {
            return None;
        }
        if t.is_empty() || t.starts_with('|') || t.starts_with("- ") || t.starts_with("```") {
            return None;
        }
        Some(t.to_string())
    })
}

/// The first `PROVENANCE:` comment line, trimmed of the marker.
fn provenance_line(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let marker = "PROVENANCE:";
        l.find(marker).map(|i| {
            l[i + marker.len()..]
                .trim_start_matches(':')
                .trim()
                .to_string()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_summary_and_provenance_parse() {
        let text = "// PROVENANCE: paper-spec — Blah, §2, Eq. 3\n# My Doc\n\nBody line.\n- item\n";
        assert_eq!(first_heading(text).as_deref(), Some("My Doc"));
        assert_eq!(first_paragraph(text).as_deref(), Some("Body line."));
        assert!(provenance_line(text)
            .map(|p| p.contains("paper-spec"))
            .unwrap_or(false));
    }

    #[test]
    fn first_paragraph_skips_code_fence() {
        let text = "```\ncode\n```\n\nReal body.\n";
        assert_eq!(first_paragraph(text).as_deref(), Some("Real body."));
    }

    #[test]
    fn no_heading_falls_back_to_filename() {
        let path = std::env::temp_dir().join("plain.md");
        std::fs::write(&path, "no heading here").unwrap();
        let doc = load_doc(path);
        assert_eq!(doc.title, "plain.md");
        assert!(doc.summary.is_some());
    }

    #[test]
    fn discover_reads_repo_optimizations() {
        let mut cfg = Config::from_env();
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        cfg.repo_root = manifest.parent().unwrap().to_path_buf();
        let docs = discover(&cfg);
        assert!(docs.len() >= 10);
    }
}
