//! Pymander — the reference mind: static, curated, domain-specific knowledge.
//!
//! Port of `libvitriol/pymander.py`. Each domain is a distinct Hermetis memory
//! root `~/.vitriol/pymander/<domain>/memory.db` (project_id =
//! `pymander/<domain>`), so node versioning, the embedding cache, and strength
//! come from the existing db machinery. Ingest format: `## Heading` starts an
//! atomic node; the body up to the next `##` is its summary.

use std::path::{Path, PathBuf};

use regex::Regex;

use crate::db::Hermes;
use crate::retrieval::{retrieve, RetrieveParams};
use crate::scorer::estimate_tokens;
use crate::NodeMeta;

const DOMAIN_PREFIX: &str = "pymander/";

/// Lazy domain-name validator: `^[A-Za-z0-9][A-Za-z0-9._-]*$`.
fn domain_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*$").unwrap())
}

/// Validate + normalize a domain name (raises a string error).
pub fn sanitize_domain(domain: &str) -> Result<String, String> {
    if domain_re().is_match(domain) {
        Ok(domain.to_string())
    } else {
        Err(format!(
            "invalid domain {domain:?}: use letters/digits/._- , no slashes"
        ))
    }
}

/// The Hermetis project_id backing a Pymander domain.
pub fn domain_project_id(domain: &str) -> Result<String, String> {
    Ok(format!("{DOMAIN_PREFIX}{}", sanitize_domain(domain)?))
}

fn selection_path(memory_root: &Path) -> PathBuf {
    memory_root.join("pymander/selection.json")
}

fn candidates_path(memory_root: &Path) -> PathBuf {
    memory_root.join("pymander/candidates.json")
}

/// Installed Pymander domains (dirs holding a memory.db), sorted.
pub fn list_domains(memory_root: &Path) -> Vec<String> {
    let root = memory_root.join("pymander");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() && p.join("memory.db").exists() {
            if let Some(name) = p.file_name() {
                out.push(name.to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

/// Split markdown into (label, summary) atomic nodes — Python `_parse_markdown`.
pub fn parse_markdown(md_text: &str) -> Vec<(String, String)> {
    let mut nodes = Vec::new();
    let mut label: Option<String> = None;
    let mut body: Vec<String> = Vec::new();
    for line in md_text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            flush_node(&mut nodes, label.take(), &mut body);
            label = Some(rest.trim().to_string());
        } else if label.is_some() {
            body.push(line.to_string());
        }
    }
    flush_node(&mut nodes, label, &mut body);
    nodes
}

fn flush_node(nodes: &mut Vec<(String, String)>, label: Option<String>, body: &mut Vec<String>) {
    if let Some(l) = label {
        let summary = body.join("\n").trim().to_string();
        if !summary.is_empty() {
            nodes.push((l, summary));
        }
        body.clear();
    }
}

/// Ingest a markdown corpus as atomic nodes for a domain.
pub fn ingest_markdown(
    h: &Hermes,
    domain: &str,
    md_text: &str,
    git_rev: &str,
) -> Result<serde_json::Value, String> {
    let domain = sanitize_domain(domain)?;
    let pid = domain_project_id(&domain)?;
    let nodes = parse_markdown(md_text);
    let conn = h.conn(&pid).map_err(|e| e.to_string())?;
    let mut stored = 0usize;
    let mut refreshed = 0usize;
    for (label, summary) in &nodes {
        let exists = conn
            .query_row(
                "SELECT 1 FROM knowledge_nodes WHERE label=?1 AND git_rev=?2",
                rusqlite::params![label, git_rev],
                |_| Ok(()),
            )
            .is_ok();
        h.store_node(
            &pid,
            label,
            summary,
            &NodeMeta {
                git_rev: git_rev.to_string(),
                strength: 1.0,
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?;
        if exists {
            refreshed += 1;
        } else {
            stored += 1;
        }
    }
    Ok(serde_json::json!({
        "domain": domain,
        "nodes": nodes.len(),
        "stored": stored,
        "refreshed": refreshed,
        "embedded": 0, // semantic-off: no embed provider
    }))
}

/// Current (superseded=0) nodes of a domain.
pub fn list_nodes(h: &Hermes, domain: &str) -> Result<Vec<serde_json::Value>, String> {
    let pid = domain_project_id(domain)?;
    h.fetch_nodes(&pid, 100_000, false)
        .map_err(|e| e.to_string())
}

/// The most relevant nodes of a domain for a query.
pub fn search(
    h: &Hermes,
    domain: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<serde_json::Value>, String> {
    let pid = domain_project_id(domain)?;
    let params = RetrieveParams {
        top_k,
        cascade_depth: 0,
        include_history: false,
        candidate_multiplier: 10,
    };
    let candidates = retrieve(h, &pid, query, &params);
    Ok(candidates
        .into_iter()
        .filter(|c| c.get("_type").and_then(|t| t.as_str()) == Some("node"))
        .collect())
}

/// Persisted selection.json read (missing/corrupt -> empty).
fn load_selection(memory_root: &Path) -> serde_json::Value {
    let path = selection_path(memory_root);
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Atomically write selection.json.
fn save_selection(memory_root: &Path, data: &serde_json::Value) -> Result<(), String> {
    let path = selection_path(memory_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(data).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Set the active Pymander domains for a project (persisted).
pub fn set_selection(
    memory_root: &Path,
    project_id: &str,
    domains: &[String],
) -> Result<serde_json::Value, String> {
    let mut data = load_selection(memory_root);
    let clean: Vec<String> = domains
        .iter()
        .map(|d| sanitize_domain(d))
        .collect::<Result<_, _>>()?;
    data[project_id] = serde_json::json!(clean);
    save_selection(memory_root, &data)?;
    Ok(serde_json::json!({"project_id": project_id, "domains": clean}))
}

/// The active Pymander domains for a project (empty if unset).
pub fn get_selection(memory_root: &Path, project_id: &str) -> Vec<String> {
    load_selection(memory_root)
        .get(project_id)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|d| d.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Best-effort git HEAD of the corpus file's repo ('' when not in a repo).
pub fn repo_rev(path: &str) -> String {
    let p = Path::new(path);
    if path == "-" || !p.is_file() {
        return String::new();
    }
    let base = p.parent().unwrap_or(Path::new("."));
    if let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(base)
        .args(["rev-parse", "HEAD"])
        .output()
    {
        if out.status.success() {
            if let Ok(s) = String::from_utf8(out.stdout) {
                return s.trim().to_string();
            }
        }
    }
    String::new()
}

/// Doctrine builder options.
#[derive(Debug, Clone)]
pub struct DoctrineOpts {
    pub query: String,
    pub budget_tokens: usize,
    pub top_k: usize,
}

/// Build a budgeted doctrine block for the project's selected domains.
pub fn build_doctrine(
    h: &Hermes,
    memory_root: &Path,
    project_id: &str,
    o: &DoctrineOpts,
) -> String {
    let domains = get_selection(memory_root, project_id);
    let domains = if domains.is_empty() {
        list_domains(memory_root).into_iter().take(1).collect()
    } else {
        domains
    };
    let mut sections = Vec::new();
    let mut used = 0usize;
    for domain in domains {
        let text = domain_section(
            h,
            &domain,
            &o.query,
            o.top_k,
            o.budget_tokens.saturating_sub(used),
        );
        if text.is_empty() {
            continue;
        }
        let text_toks = estimate_tokens(&text) + 1;
        if used + text_toks > o.budget_tokens && used > 0 {
            break;
        }
        used += text_toks;
        sections.push(text);
    }
    sections.join("\n\n")
}

/// One domain's doctrine lines under a per-domain budget.
fn domain_section(h: &Hermes, domain: &str, query: &str, top_k: usize, budget: usize) -> String {
    let Ok(hits) = search(h, domain, query, top_k) else {
        return String::new();
    };
    if hits.is_empty() {
        return String::new();
    }
    let mut parts = vec![format!("## {domain}")];
    let mut used = 0usize;
    for hit in &hits {
        let label = hit.get("label").and_then(|l| l.as_str()).unwrap_or("");
        let summary = hit.get("summary").and_then(|s| s.as_str()).unwrap_or("");
        let body = format!("- {label}: {summary}");
        let toks = estimate_tokens(&body) + 1;
        if used + toks > budget && used > 0 {
            break;
        }
        used += toks;
        parts.push(body);
    }
    parts.join("\n")
}

/// Add a promotion candidate (curated, never auto-merged).
pub fn add_candidate(
    memory_root: &Path,
    domain: &str,
    label: &str,
    summary: &str,
    source: &str,
) -> Result<serde_json::Value, String> {
    let domain = sanitize_domain(domain)?;
    let path = candidates_path(memory_root);
    let mut data = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    if data.get(&domain).and_then(|v| v.as_array()).is_none() {
        data[&domain] = serde_json::json!([]);
    }
    if let Some(arr) = data.get_mut(&domain).and_then(|v| v.as_array_mut()) {
        arr.push(serde_json::json!({"label": label, "summary": summary, "source": source}));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(&data).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({"domain": domain, "candidate": label}))
}

/// List promotion candidates (all domains, or one domain).
pub fn list_candidates(memory_root: &Path, domain: &str) -> serde_json::Value {
    let data = std::fs::read_to_string(candidates_path(memory_root))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if domain.is_empty() {
        data
    } else {
        serde_json::json!({domain: data.get(domain).cloned().unwrap_or(serde_json::json!([]))})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_home(name: &str) -> (PathBuf, Hermes) {
        let root = std::env::temp_dir().join(format!("hermes_pymander_{name}"));
        let _ = std::fs::remove_dir_all(&root);
        (root.clone(), Hermes::new(&root))
    }

    #[test]
    fn domain_validation() {
        assert_eq!(sanitize_domain("systems").unwrap(), "systems");
        assert_eq!(sanitize_domain("c-firmware_v2").unwrap(), "c-firmware_v2");
        assert!(sanitize_domain("bad/name").is_err());
        assert!(sanitize_domain("-leading").is_err());
    }

    #[test]
    fn parse_markdown_splits_nodes() {
        let md = "# Domain header\nprose skipped\n\n## First\nbody one\n\n## Second\nbody two\n";
        let nodes = parse_markdown(md);
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].0, "First");
        assert!(nodes[0].1.contains("body one"));
        assert!(nodes[1].1.contains("body two"));
    }

    #[test]
    fn ingest_and_search_roundtrip() {
        let (root, h) = tmp_home("ingest");
        let md = "## widget\nwidgets are fast\n## parser\nparser handles tokens\n";
        let res = ingest_markdown(&h, "systems", md, "abc").unwrap();
        assert_eq!(res["stored"], 2);
        assert_eq!(res["refreshed"], 0);
        // re-ingest same rev -> refreshed
        let res = ingest_markdown(&h, "systems", md, "abc").unwrap();
        assert_eq!(res["stored"], 0);
        assert_eq!(res["refreshed"], 2);
        // search finds the parser node for a token query
        let hits = search(&h, "systems", "token parser", 5).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0]["label"], "parser");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn selection_and_doctrine() {
        let (root, h) = tmp_home("doctrine");
        let md = "## rust\nownership is key\n";
        ingest_markdown(&h, "rust", md, "abc").unwrap();
        let res = set_selection(&root, "proj", &["rust".to_string()]).unwrap();
        assert_eq!(res["domains"][0], "rust");
        assert_eq!(get_selection(&root, "proj"), vec!["rust".to_string()]);
        let block = build_doctrine(
            &h,
            &root,
            "proj",
            &DoctrineOpts {
                query: "ownership".into(),
                budget_tokens: 1000,
                top_k: 3,
            },
        );
        assert!(block.contains("## rust"));
        assert!(block.contains("ownership"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
