//! GUIDE tab: render VITRIOL documentation, provenance, and the Pymander corpus.
//!
//! Discovers markdown under the repo `docs/`, `docs/provenance/`, and
//! `docs/pymander/`, each yielding an index row (title + file path + kind). A
//! selected row's markdown renders in a scrolled reader pane. "Open" intents
//! (edit the doc in `$EDITOR`, or open a paper URL in a browser) are returned as
//! an `OpenAction` so the UI can shell out — this module only reads.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::config::Config;

/// One discoverable guide entry.
#[derive(Debug, Clone)]
pub struct Doc {
    /// Display title (first `# ` heading, else the file name).
    pub title: String,
    /// Absolute path of the markdown file.
    pub path: PathBuf,
    /// Category: settings | sweep | provenance | corpus.
    pub kind: Kind,
    /// The first `PROVENANCE:` line content, when present.
    pub provenance: Option<String>,
}

/// Guide-entry category (used for filtering and display grouping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Settings / operation docs.
    Settings,
    /// Hardware tuning.
    Sweep,
    /// Cleanroom provenance records.
    Provenance,
    /// Pymander reference corpus.
    Corpus,
}

impl Kind {
    /// Short label.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Settings => "settings",
            Kind::Sweep => "sweep",
            Kind::Provenance => "provenance",
            Kind::Corpus => "corpus",
        }
    }

    /// Sort priority for the index (settings first, corpus last).
    fn rank(self) -> u8 {
        match self {
            Kind::Settings => 0,
            Kind::Sweep => 1,
            Kind::Provenance => 2,
            Kind::Corpus => 3,
        }
    }
}

/// Discover every guide entry under the repo docs tree, sorted by kind then
/// title. Missing directories yield an empty list (never errors).
pub fn discover(cfg: &Config) -> Vec<Doc> {
    let docs = cfg.repo_root.join("docs");
    let mut out = Vec::new();
    collect_subtree(&docs.join("provenance"), Kind::Provenance, &mut out);
    collect_subtree(&docs.join("pymander"), Kind::Corpus, &mut out);
    collect_docs_root(&docs, &mut out);
    out.sort_by(|a, b| {
        a.kind
            .rank()
            .cmp(&b.kind.rank())
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

/// Collect every `.md` in a subdirectory subtree.
fn collect_subtree(dir: &PathBuf, kind: Kind, out: &mut Vec<Doc>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if e.file_type().map(|t| t.is_file()).unwrap_or(false)
            && path.extension().and_then(|x| x.to_str()) == Some("md")
        {
            out.push(load_doc(path, kind));
        }
    }
}

/// Collect markdown files directly in docs/ (its subdirectories are handled by
/// `collect_subtree` per category to avoid double-counting).
fn collect_docs_root(dir: &PathBuf, out: &mut Vec<Doc>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if path.extension().and_then(|x| x.to_str()) == Some("md") {
            let kind = kind_of(&path);
            out.push(load_doc(path, kind));
        }
    }
}

/// Best-effort kind for a docs-root file based on its name.
fn kind_of(path: &Path) -> Kind {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.contains("sweep") || name.contains("autotun") {
        Kind::Sweep
    } else {
        Kind::Settings
    }
}

/// Read a doc and derive its title + provenance line.
fn load_doc(path: PathBuf, kind: Kind) -> Doc {
    let text = fs::read_to_string(&path).unwrap_or_default();
    let title = first_heading(&text).unwrap_or_else(|| {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "untitled".into())
    });
    let provenance = provenance_line(&text);
    Doc {
        title,
        path,
        kind,
        provenance,
    }
}

/// The first `# ` heading, trimmed.
fn first_heading(text: &str) -> Option<String> {
    text.lines()
        .find(|l| l.starts_with("# "))
        .map(|l| l.trim_start_matches('#').trim().to_string())
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

/// The markdown body as raw lines (each line preserved; the reader scrolls).
pub fn render_markdown(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_and_provenance_parse() {
        let text = "// PROVENANCE: paper-spec — Blah, §2, Eq. 3\n# My Doc\n\nBody.\n";
        assert_eq!(first_heading(text).as_deref(), Some("My Doc"));
        assert!(provenance_line(text)
            .map(|p| p.contains("paper-spec"))
            .unwrap_or(false));
    }

    #[test]
    fn markdown_lines_preserved() {
        let text = "# T\n\npara\n\n- item\n";
        let lines = render_markdown(text);
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "# T");
        assert_eq!(lines[2], "para");
    }

    #[test]
    fn no_heading_falls_back_to_filename() {
        let path = std::env::temp_dir().join("plain.md");
        std::fs::write(&path, "no heading here").unwrap();
        let doc = load_doc(path, Kind::Settings);
        assert_eq!(doc.title, "plain.md");
        assert!(doc.provenance.is_none());
    }
}
