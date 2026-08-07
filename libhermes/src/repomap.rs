//! Repo map — port of `hermetis/repomap.py`.
//!
//! Aider-style importance-ranked symbol map: rank files by in-degree in the
//! import graph, extract per-file symbols with language regexes, budget by
//! tokens. Versioned per-file nodes are stored via `Hermes::store_node`
//! (git_rev keyed), matching the Python store path.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::db::Hermes;
use crate::scorer::estimate_tokens;

const SKIP_DIRS: &[&str] = &[
    ".git",
    ".opencode",
    "__pycache__",
    "node_modules",
    "target",
    "build",
    "dist",
    ".venv",
    "venv",
    ".vitriol",
    ".cache",
];
const SKIP_EXTS: &[&str] = &[
    ".pyc", ".pyo", ".so", ".o", ".dll", ".dylib", ".exe", ".gguf", ".bin", ".lock", ".min.js",
    ".map",
];

/// Per-language symbol patterns (regex, kind).
const LANG_PATTERNS: &[(&str, &[(&str, &str)])] = &[
    (
        "python",
        &[
            (r"^\s*def\s+(\w+)\s*\(([^)]*)\)", "def"),
            (r"^\s*class\s+(\w+)\s*[(:]", "class"),
            (r"^\s+def\s+(\w+)\s*\(([^)]*)\)", "method"),
        ],
    ),
    (
        "rust",
        &[
            (
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(([^)]*)\)",
                "fn",
            ),
            (
                r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|trait)\s+(\w+)",
                "type",
            ),
            (r"^\s+fn\s+(\w+)\s*\(([^)]*)\)", "method"),
        ],
    ),
    (
        "typescript",
        &[
            (
                r"^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s+(\w+)\s*\(([^)]*)\)",
                "function",
            ),
            (r"^(?:export\s+)?(?:abstract\s+)?class\s+(\w+)", "class"),
            (r"^(?:export\s+)?(?:interface|type)\s+(\w+)", "type"),
            (r"^\s{2}(?:async\s+)?(\w+)\s*\(([^)]*)\)\s*[:\{]", "method"),
        ],
    ),
    (
        "go",
        &[
            (r"^func\s+(\w+)\s*\(([^)]*)\)", "func"),
            (r"^func\s+\([^)]*\)\s+(\w+)\s*\(([^)]*)\)", "method"),
            (r"^type\s+(\w+)\s+(?:struct|interface)", "type"),
        ],
    ),
    (
        "c_cpp",
        &[
            (
                r"^(?:static\s+|inline\s+|extern\s+)?[\w:<>*& ]+\s+(\w+)\s*\(([^)]*)\)\s*\{?",
                "func",
            ),
            (r"^class\s+(\w+)", "class"),
            (r"^struct\s+(\w+)", "struct"),
        ],
    ),
];

/// Per-language import patterns.
const IMPORT_PATTERNS: &[(&str, &[&str])] = &[
    ("python", &[r"^\s*(?:import|from)\s+([\w\.]+)"]),
    ("rust", &[r"^\s*use\s+([\w:]+)"]),
    (
        "typescript",
        &[
            r#"from\s+['"]([^\.][^'"]*)['"]"#,
            r#"require\(['"]([^\.][^'"]*)['"]\)"#,
        ],
    ),
    ("go", &[r#"^\s*"([^"]+)""#]),
    ("c_cpp", &[r#"^#include\s*[<"]([\w\./]+)[>"]"#]),
];

const LANG_BY_EXT: &[(&str, &str)] = &[
    (".py", "python"),
    (".rs", "rust"),
    (".ts", "typescript"),
    (".tsx", "typescript"),
    (".js", "typescript"),
    (".mjs", "typescript"),
    (".go", "go"),
    (".c", "c_cpp"),
    (".h", "c_cpp"),
    (".cpp", "c_cpp"),
    (".cc", "c_cpp"),
    (".hpp", "c_cpp"),
    (".hh", "c_cpp"),
];

/// Map a file path to its language key, or None.
fn lang_for(path: &str) -> Option<String> {
    let lower = path.to_lowercase();
    if lower.ends_with(".min.js") {
        return None;
    }
    let ext = std::path::Path::new(&lower).extension()?.to_str()?;
    let ext = format!(".{ext}");
    LANG_BY_EXT
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, l)| l.to_string())
}

fn patterns_for<'a>(
    lang: &str,
    table: &'a [(&str, &'a [(&'a str, &'a str)])],
) -> &'a [(&'a str, &'a str)] {
    table
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, pats)| *pats)
        .unwrap_or(&[])
}

/// Extract `(name, kind, signature)` tuples for a file's content.
pub fn extract_symbols(content: &str, lang: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for line in content.lines() {
        if let Some(sym) = scan_symbol_line(line, lang) {
            out.push(sym);
        }
    }
    out
}

/// The first symbol match on one line, if any.
fn scan_symbol_line(line: &str, lang: &str) -> Option<(String, String, String)> {
    for (pattern, kind) in patterns_for(lang, LANG_PATTERNS) {
        let re = Regex::new(pattern).ok()?;
        if let Some(caps) = re.captures(line) {
            let name = caps
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let args = caps.get(2).map(|m| m.as_str()).unwrap_or("");
            let sig = format!(
                "{}({})",
                name,
                args.split_whitespace().collect::<Vec<_>>().join(" ")
            );
            return Some((name, kind.to_string(), sig));
        }
    }
    None
}

/// Module tokens a file imports (crude) — Python `file_imports`.
fn file_imports(content: &str, lang: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    for line in content.lines() {
        scan_import_line(line, lang, &mut out);
    }
    out
}

/// Scan one line for an import token, adding it to `out`.
fn scan_import_line(line: &str, lang: &str, out: &mut HashSet<String>) {
    for pattern in patterns_for_imports(lang) {
        if let Some(token) = match_import(line, pattern) {
            out.insert(token);
            return;
        }
    }
}

/// The import token on `line` under `pattern`, or None.
fn match_import(line: &str, pattern: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(line)?;
    let m = caps.get(1)?;
    Some(normalize_import(m.as_str()))
}

/// Normalize a module token: ':' and '.' become '/'.
fn normalize_import(s: &str) -> String {
    s.chars()
        .map(|c| if c == ':' || c == '.' { '/' } else { c })
        .collect()
}

fn patterns_for_imports(lang: &str) -> Vec<&str> {
    IMPORT_PATTERNS
        .iter()
        .find(|(l, _)| *l == lang)
        .map(|(_, pats)| pats.to_vec())
        .unwrap_or_default()
}

/// Source files under root: (relpath, abspath), language-supported only.
fn iter_source_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        visit_source_dir(&dir, &mut stack, &mut out, root);
    }
    out
}

/// One directory in the source walk.
fn visit_source_dir(
    dir: &Path,
    stack: &mut Vec<PathBuf>,
    out: &mut Vec<(String, PathBuf)>,
    root: &Path,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        visit_source_entry(stack, out, root, &e.path());
    }
}

/// One directory entry in the source walk: push subdirs, collect supported files.
fn visit_source_entry(
    stack: &mut Vec<PathBuf>,
    out: &mut Vec<(String, PathBuf)>,
    root: &Path,
    p: &Path,
) {
    if p.is_dir() {
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if SKIP_DIRS.contains(&name.as_str()) || name.starts_with('.') {
            return;
        }
        stack.push(p.to_path_buf());
    } else {
        let fname = p
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(p)
            .to_string_lossy()
            .into_owned();
        if SKIP_EXTS.iter().any(|e| fname.ends_with(e)) {
            return;
        }
        if lang_for(&rel).is_none() {
            return;
        }
        out.push((rel, p.to_path_buf()));
    }
}

/// The worktree HEAD commit, or an mtime-based fallback.
pub fn git_rev(root: &Path) -> String {
    let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "HEAD"])
        .output()
    else {
        return git_rev_fallback(root);
    };
    if !out.status.success() {
        return git_rev_fallback(root);
    }
    let Ok(s) = String::from_utf8(out.stdout) else {
        return git_rev_fallback(root);
    };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return git_rev_fallback(root);
    }
    trimmed.to_string()
}

/// Fallback rev: the newest file mtime in the tree.
fn git_rev_fallback(root: &Path) -> String {
    let mut newest = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        newest = scan_dir_mtime(&dir, &mut stack, newest);
    }
    format!("mtime:{newest}")
}

/// One directory: recurse into subdirs, track newest file mtime.
fn scan_dir_mtime(dir: &Path, stack: &mut Vec<PathBuf>, mut newest: u64) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return newest;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            stack.push(p);
        } else {
            track_mtime(&mut newest, &p);
        }
    }
    newest
}

/// Update `newest` with a file's mtime seconds (if readable).
fn track_mtime(newest: &mut u64, p: &Path) {
    if let Ok(md) = std::fs::metadata(p) {
        if let Ok(mt) = md.modified() {
            if let Ok(secs) = mt.duration_since(std::time::UNIX_EPOCH) {
                *newest = (*newest).max(secs.as_secs());
            }
        }
    }
}

/// The candidate module names for an import (the import itself + each trailing
/// segment prefix).
fn candidate_paths(mod_name: &str) -> Vec<String> {
    let mut candidates = vec![mod_name.to_string()];
    let parts: Vec<&str> = mod_name.split('/').collect();
    for i in 1..parts.len() {
        candidates.push(parts[i..].join("/"));
    }
    candidates
}

/// Import-graph in-degree per file — Python `_rank_files`'s counting loop.
fn compute_in_degree(
    root: &Path,
    files: &[(String, PathBuf)],
    mod_to_file: &HashMap<String, String>,
) -> HashMap<String, usize> {
    let mut in_degree: HashMap<String, usize> =
        files.iter().map(|(rel, _)| (rel.clone(), 0usize)).collect();
    for (rel, abspath) in files {
        let Some(lang) = lang_for(rel) else {
            continue;
        };
        let Ok(content) = std::fs::read_to_string(abspath) else {
            continue;
        };
        process_file_imports(&content, &lang, mod_to_file, rel, &mut in_degree);
    }
    let _ = root;
    in_degree
}

/// Bump in-degree for every import token in one file.
fn process_file_imports(
    content: &str,
    lang: &str,
    mod_to_file: &HashMap<String, String>,
    rel: &str,
    in_degree: &mut HashMap<String, usize>,
) {
    for mod_name in file_imports(content, lang) {
        bump_candidates(&mod_name, mod_to_file, rel, in_degree);
    }
}

/// Try each candidate path of an import token against the module map.
fn bump_candidates(
    mod_name: &str,
    mod_to_file: &HashMap<String, String>,
    rel: &str,
    in_degree: &mut HashMap<String, usize>,
) {
    for cand in candidate_paths(mod_name) {
        bump_target(in_degree, mod_to_file, &cand, rel);
    }
}

/// Increment the in-degree of the file that imports `cand` (when it resolves to
/// a different file than the importer).
fn bump_target(
    in_degree: &mut HashMap<String, usize>,
    mod_to_file: &HashMap<String, String>,
    cand: &str,
    rel: &str,
) {
    if let Some(target) = mod_to_file.get(cand) {
        if target != rel {
            if let Some(d) = in_degree.get_mut(target) {
                *d += 1;
            }
        }
    }
}

/// Rank files by in-degree in the import graph (Aider-style importance).
fn rank_files(root: &Path, files: &[(String, PathBuf)]) -> Vec<(String, PathBuf)> {
    let mut mod_to_file: HashMap<String, String> = HashMap::new();
    for (rel, _) in files {
        let stem = std::path::Path::new(rel)
            .with_extension("")
            .to_string_lossy()
            .into_owned()
            .replace('\\', "/");
        mod_to_file.insert(stem, rel.clone());
    }
    let in_degree = compute_in_degree(root, files, &mod_to_file);
    let mut ranked = files.to_vec();
    ranked.sort_by(|a, b| {
        let da = in_degree.get(&a.0).copied().unwrap_or(0);
        let db = in_degree.get(&b.0).copied().unwrap_or(0);
        db.cmp(&da).then_with(|| a.0.cmp(&b.0))
    });
    ranked
}
pub fn build_repo_map(root: &Path, budget_tokens: usize, max_files: Option<usize>) -> String {
    let files = iter_source_files(root);
    let mut ranked = rank_files(root, &files);
    if let Some(mf) = max_files {
        ranked.truncate(mf);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut used = 0usize;
    for (rel, abspath) in &ranked {
        let Some(entry) = file_entry(rel, abspath) else {
            continue;
        };
        let entry_tokens: usize = entry.iter().map(|l| estimate_tokens(l)).sum::<usize>() + 1;
        if used + entry_tokens > budget_tokens && used > 0 {
            break;
        }
        used += entry_tokens;
        lines.extend(entry);
    }
    lines.join("\n")
}

/// One file's map entry lines (`rel:` + indented symbols), or None when the
/// file has no supported symbols.
fn file_entry(rel: &str, abspath: &Path) -> Option<Vec<String>> {
    let lang = lang_for(rel)?;
    let content = std::fs::read_to_string(abspath).ok()?;
    let syms = extract_symbols(&content, &lang);
    if syms.is_empty() {
        return None;
    }
    let mut entry = vec![format!("{rel}:")];
    for (_, kind, sig) in &syms {
        entry.push(format!("  {kind} {sig}"));
    }
    Some(entry)
}

/// Store per-file nodes (versioned by git_rev) — Python `store_repo_map`.
pub fn store_repo_map(
    h: &Hermes,
    project_id: &str,
    root: &Path,
    budget_tokens: usize,
    max_files: Option<usize>,
) -> (String, usize) {
    let rev = git_rev(root);
    let files = iter_source_files(root);
    let mut ranked = rank_files(root, &files);
    if let Some(mf) = max_files {
        ranked.truncate(mf);
    }
    let mut stored = 0usize;
    for (rel, abspath) in &ranked {
        if store_one(h, project_id, rel, abspath, &rev) {
            stored += 1;
        }
    }
    let map = build_repo_map(root, budget_tokens, max_files);
    (map, stored)
}

/// Re-store nodes for specific files (file-edit refresh). Returns count.
pub fn store_file_nodes(h: &Hermes, project_id: &str, root: &Path, relfiles: &[String]) -> usize {
    let rev = git_rev(root);
    let mut stored = 0usize;
    for rel in relfiles {
        let rel = rel.trim_start_matches('/');
        let abspath = root.join(rel);
        if !abspath.is_file() {
            continue;
        }
        if store_one(h, project_id, rel, &abspath, &rev) {
            stored += 1;
        }
    }
    stored
}

/// Extract one file's symbols and store its versioned node. True if stored.
fn store_one(h: &Hermes, project_id: &str, rel: &str, abspath: &Path, rev: &str) -> bool {
    let Some(lang) = lang_for(rel) else {
        return false;
    };
    let Ok(content) = std::fs::read_to_string(abspath) else {
        return false;
    };
    let syms = extract_symbols(&content, &lang);
    if syms.is_empty() {
        return false;
    }
    let summary = syms
        .iter()
        .map(|(_, kind, sig)| format!("{kind} {sig}"))
        .collect::<Vec<_>>()
        .join("\n");
    let meta = crate::NodeMeta {
        git_rev: rev.to_string(),
        ..Default::default()
    };
    let _ = h.store_node(project_id, rel, &summary, &meta);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_tree(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("hermes_repomap_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn lang_detection() {
        assert_eq!(lang_for("src/lib.rs").as_deref(), Some("rust"));
        assert_eq!(lang_for("a.py").as_deref(), Some("python"));
        assert_eq!(lang_for("b.min.js"), None);
        assert_eq!(lang_for("x.txt"), None);
    }

    #[test]
    fn extracts_rust_symbols() {
        let content =
            "pub fn foo(a: i32) -> i32 {\n    fn helper(x: i32) {}\n}\npub struct Bar {}\n";
        let syms = extract_symbols(content, "rust");
        // Python parity: the `^\s*` fn pattern catches indented fns too, so the
        // rust "method" pattern is shadowed and never fires.
        let kinds: Vec<&str> = syms.iter().map(|(_, k, _)| k.as_str()).collect();
        assert!(kinds.contains(&"fn"));
        assert!(kinds.contains(&"type"));
        assert!(!kinds.contains(&"method"));
        let foo = syms.iter().find(|(n, _, _)| n == "foo").unwrap();
        assert_eq!(foo.2, "foo(a: i32)");
        let helper = syms.iter().find(|(n, _, _)| n == "helper").unwrap();
        assert_eq!(helper.1, "fn");
    }

    #[test]
    fn imports_crude_mapping() {
        let content = "use crate::foo::bar;\nimport os\n";
        // Python parity: every ':' and '.' becomes '/', including the '::'.
        assert!(file_imports(content, "rust").contains("crate//foo//bar"));
        assert!(file_imports(content, "python").contains("os"));
    }

    #[test]
    fn builds_budgeted_map() {
        let root = tmp_tree("map");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn alpha() -> i32 { 0 }\npub struct Beta {}\n",
        )
        .unwrap();
        std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
        let map = build_repo_map(&root, 1000, None);
        assert!(map.contains("src/lib.rs"));
        assert!(map.contains("fn alpha()"));
        assert!(map.contains("type Beta()"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn store_nodes_versioned() {
        let root = tmp_tree("store");
        std::fs::write(root.join("lib.rs"), "pub fn foo() {}\npub struct S {}\n").unwrap();
        let h = Hermes::new(&std::env::temp_dir().join("hermes_repomap_home"));
        let (map, stored) = store_repo_map(&h, "p", &root, 1000, None);
        assert_eq!(stored, 1);
        assert!(map.contains("lib.rs"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
