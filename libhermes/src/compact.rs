//! Compact — token-budgeted formatting, port of `hermetis/compact.py`.

use crate::scorer::estimate_tokens;

/// Default active budget (env `MEMORY_ACTIVE_BUDGET`).
pub fn default_active_budget() -> usize {
    std::env::var("MEMORY_ACTIVE_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(4000)
}

/// Format an episode for injection — Python `format_episode`.
pub fn format_episode(episode: &serde_json::Value, max_chars: Option<usize>) -> String {
    let mut content = episode
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(mc) = max_chars {
        // Char-safe truncation: byte-slice panics on multi-byte UTF-8.
        let char_count = content.chars().count();
        if char_count > mc {
            content = content.chars().take(mc).collect::<String>() + "…";
        }
    }
    let created = episode
        .get("created_at")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let created = if created.is_empty() {
        String::new()
    } else {
        // "YYYY-MM-DD" prefix from the ISO/SQLite timestamp (Python strftime).
        created.chars().take(10).collect()
    };
    let role = episode
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("user");
    let session_label = episode
        .get("session_label")
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let mut prefix = if created.is_empty() {
        String::new()
    } else {
        format!("[{created}]")
    };
    if !session_label.is_empty() {
        prefix += &format!(" [{session_label}]");
    }
    format!("{prefix} {role}: {content}")
}

/// Format a knowledge node — Python `format_node`.
pub fn format_node(node: &serde_json::Value) -> String {
    let label = node
        .get("label")
        .and_then(|l| l.as_str())
        .unwrap_or("memory");
    let summary = node.get("summary").and_then(|s| s.as_str()).unwrap_or("");
    let strength = node.get("strength").and_then(|s| s.as_f64()).unwrap_or(1.0);
    let marker = if strength > 0.7 { "●" } else { "○" };
    format!("[Consolidated: {label}] ({marker}) {summary}")
}

/// Ultra-compact format when budget is tight — Python `format_compact`.
pub fn format_compact(episode: &serde_json::Value) -> String {
    let content = episode
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let compact: String = content
        .chars()
        .take(120)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    let compact = compact.trim().to_string();
    let compact = if compact.chars().count() > 120 {
        format!("{compact}…")
    } else {
        compact
    };
    let created = episode
        .get("created_at")
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let created = created.chars().take(10).collect::<String>();
    let role = episode
        .get("role")
        .and_then(|r| r.as_str())
        .unwrap_or("user");
    format!("[{created}] {role}: {compact}")
}

/// Options for `compact_context` (bundled to stay under the param gate).
#[derive(Debug, Clone)]
pub struct CompactOptions<'a> {
    pub project_id: &'a str,
    pub session_id: &'a str,
    pub query: &'a str,
    pub recent_episodes: Option<&'a [serde_json::Value]>,
    pub budget: usize,
}

/// Build the context header block.
fn build_header(o: &CompactOptions<'_>) -> String {
    let mut header = "[Memory Context — VITRIOL Emulated Memory]".to_string();
    if !o.project_id.is_empty() {
        header += &format!("\nProject: {}", o.project_id);
    }
    if !o.session_id.is_empty() {
        header += &format!(" | Session: {}", o.session_id);
    }
    header += &format!("\nQuery: {}\n", o.query);
    header
}

/// Build the recent-session context block.
fn build_recent_section(recent: &[serde_json::Value]) -> String {
    let mut section = "\n## Recent Context\n".to_string();
    for ep in recent {
        section += &format_episode(ep, None);
        section.push('\n');
    }
    section
}

/// Format one candidate (node vs episode) with its score.
fn format_candidate(candidate: &serde_json::Value) -> String {
    let ctype = candidate
        .get("_type")
        .and_then(|t| t.as_str())
        .unwrap_or("episode");
    let body = if ctype == "node" {
        format_node(candidate)
    } else {
        format_episode(candidate, None)
    };
    let score = candidate
        .get("_score")
        .and_then(|s| s.as_f64())
        .unwrap_or(0.0);
    format!("{body} (score: {score:.2})")
}

/// Fill the relevant-context section within the budget; returns (section, used).
fn append_relevant(
    candidates: &[serde_json::Value],
    budget: usize,
    mut tokens_used: usize,
) -> (String, usize) {
    let mut section = "\n## Relevant Context\n".to_string();
    for candidate in candidates {
        if tokens_used >= budget {
            break;
        }
        let formatted = format_candidate(candidate);
        let tokens = estimate_tokens(&formatted);
        if tokens_used + tokens <= budget {
            section += &formatted;
            section.push('\n');
            tokens_used += tokens;
        } else {
            let compact = format_compact(candidate);
            if tokens_used + estimate_tokens(&compact) <= budget {
                section += &compact;
                section.push('\n');
                tokens_used += estimate_tokens(&compact);
            }
        }
    }
    (section, tokens_used)
}

/// Build the injected context block list — Python `compact_context`.
pub fn compact_context(candidates: &[serde_json::Value], o: &CompactOptions<'_>) -> Vec<String> {
    let mut injected = Vec::new();
    let mut tokens_used = 0usize;

    let header = build_header(o);
    injected.push(header.clone());
    tokens_used += estimate_tokens(&header);

    if let Some(recent) = o.recent_episodes {
        if !recent.is_empty() {
            let recent_section = build_recent_section(recent);
            let recent_tokens = estimate_tokens(&recent_section);
            if tokens_used + recent_tokens <= o.budget {
                injected.push(recent_section.clone());
                tokens_used += recent_tokens;
            }
        }
    }

    if !candidates.is_empty() {
        let (past_section, used) = append_relevant(candidates, o.budget, tokens_used);
        tokens_used = used;
        injected.push(past_section);
    }

    if tokens_used > 0 {
        injected.push("[End Memory Context]\n".to_string());
    }
    injected
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn format_episode_prefix_role_content() {
        let ep = json!({"content": "hi", "role": "user", "created_at": "2026-08-07 10:00:00"});
        assert_eq!(format_episode(&ep, None), "[2026-08-07] user: hi");
        let ep2 = json!({"content": "hi", "role": "user", "created_at": "2026-08-07 10:00:00", "session_label": "s"});
        assert_eq!(format_episode(&ep2, None), "[2026-08-07] [s] user: hi");
    }

    #[test]
    fn format_episode_truncates() {
        let ep = json!({"content": "abcdefghij", "role": "a", "created_at": ""});
        let out = format_episode(&ep, Some(4));
        assert!(out.contains("abcd…"));
    }

    #[test]
    fn format_node_marker() {
        let strong = json!({"label": "l", "summary": "s", "strength": 0.9});
        assert!(format_node(&strong).contains("●"));
        let weak = json!({"label": "l", "summary": "s", "strength": 0.3});
        assert!(format_node(&weak).contains("○"));
    }

    #[test]
    fn compact_context_budget_flow() {
        let cands = vec![
            json!({"_type": "episode", "content": "a", "role": "user", "created_at": "", "_score": 0.9}),
        ];
        let out = compact_context(
            &cands,
            &CompactOptions {
                project_id: "p",
                session_id: "s",
                query: "q",
                recent_episodes: None,
                budget: 4000,
            },
        );
        let joined = out.join("\n");
        assert!(joined.contains("Memory Context"));
        assert!(joined.contains("score: 0.90"));
        assert!(joined.contains("End Memory Context"));
    }
}
