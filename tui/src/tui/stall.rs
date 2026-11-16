// Stall trail + evidence-gated stall decision (2026-11-15).
//
// Why this module exists: a `bash` call that pi reported as running could sit
// with zero output for 300 s, then the TUI aborted and self-heal respawned pi,
// losing the turn — and nothing on disk said what the tool had been doing.
// The trail is the post-mortem; the decision keeps a demonstrably working tool
// alive and fails *fast* when a tool that claims to be running has no process
// at all (it can never start).

use std::io::Write;
use std::path::{Path, PathBuf};

/// One trail file per TUI process. Diagnosis aid, never a log sink.
const TRAIL_CAP: u64 = 2 * 1024 * 1024;

/// Silence budget for a tool that reports running but has spawned nothing.
/// Short on purpose: this case has no way to make progress (2026-11-15).
pub const UNSTARTED_SECS: u64 = 60;

/// `~/.vitriol/officina/stalls/trail-<pid>.jsonl` (override: `OFFICINA_STALL_TRAIL_DIR`).
pub fn trail_path() -> PathBuf {
    trail_path_in(&trail_root())
}

fn trail_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("OFFICINA_STALL_TRAIL_DIR") {
        return PathBuf::from(dir);
    }
    let mut p = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    p.push(".vitriol");
    p.push("officina");
    p.push("stalls");
    p
}

fn trail_path_in(root: &Path) -> PathBuf {
    root.join(format!("trail-{}.jsonl", std::process::id()))
}

/// Append one JSONL breadcrumb. Observability must never fail a turn, so every
/// error is swallowed; the size cap keeps the file bounded.
pub fn trail(event: &str, fields: serde_json::Value) {
    trail_at(&trail_path(), event, fields);
}

fn trail_at(path: &Path, event: &str, fields: serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path)
        .map(|m| m.len() > TRAIL_CAP)
        .unwrap_or(false)
    {
        return;
    }
    let line = serde_json::json!({
        "ts": crate::tui::caduceus::now_ms(),
        "event": event,
        "fields": fields,
    });
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    else {
        return;
    };
    let _ = writeln!(f, "{line}");
}

/// Liveness evidence available at stall time. All fields describe *observed*
/// facts; unknown never counts as evidence of life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Evidence {
    /// pi's subtree consumed no CPU in the last sample window.
    pub pi_flat: bool,
    /// The local engine reports `is_processing` (a long prefill or decode).
    pub engine_busy: bool,
    /// Live descendants under pi; `None` when /proc is unavailable.
    pub descendants: Option<usize>,
}

impl Evidence {
    /// Nothing we can observe is working.
    pub fn flat(&self) -> bool {
        self.pi_flat && !self.engine_busy
    }
}

/// What the watchdog should do with a tool call that outlived its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallVerdict {
    /// Started, silent, and no process exists: it can never make progress.
    NeverStarted,
    /// Something demonstrably works; extend instead of aborting.
    Working,
    /// First silent threshold: one bounded grace before any abort.
    Grace,
    /// Grace spent and the evidence is flat: abort the turn.
    Abort,
}

/// Decide the fate of a running tool.
///
/// Order matters: the "no process at all" case is checked first because it is
/// decisive (a started tool always has a live descendant while it runs), then
/// live evidence, then the bounded grace, and only then the abort.
pub fn stall_verdict(
    silent: bool,
    single_running_tool: bool,
    grace_used: bool,
    ev: Evidence,
) -> StallVerdict {
    if silent && single_running_tool && ev.descendants == Some(0) {
        return StallVerdict::NeverStarted;
    }
    if !ev.flat() {
        return StallVerdict::Working;
    }
    if !grace_used {
        return StallVerdict::Grace;
    }
    StallVerdict::Abort
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(pi_flat: bool, engine_busy: bool, descendants: Option<usize>) -> Evidence {
        Evidence {
            pi_flat,
            engine_busy,
            descendants,
        }
    }

    #[test]
    fn never_started_beats_every_other_arm() {
        // Silent, alone, and no descendant process: nothing can ever start.
        assert_eq!(
            stall_verdict(true, true, true, ev(true, false, Some(0))),
            StallVerdict::NeverStarted
        );
        // Output arrived, so it did run: the fast path must not claim otherwise.
        assert_eq!(
            stall_verdict(false, true, true, ev(true, false, Some(0))),
            StallVerdict::Abort
        );
        // A sibling tool is running; the count is not about this call.
        assert_eq!(
            stall_verdict(true, false, true, ev(true, false, Some(0))),
            StallVerdict::Abort
        );
        // /proc unavailable: unknown is not proof of the fast path.
        assert_eq!(
            stall_verdict(true, true, true, ev(true, false, None)),
            StallVerdict::Abort
        );
    }

    #[test]
    fn live_evidence_grants_time_before_the_grace() {
        assert_eq!(
            stall_verdict(false, true, true, ev(false, false, Some(3))),
            StallVerdict::Working
        );
        assert_eq!(
            stall_verdict(false, true, true, ev(true, true, Some(3))),
            StallVerdict::Working
        );
    }

    #[test]
    fn flat_evidence_gets_exactly_one_grace_then_aborts() {
        assert_eq!(
            stall_verdict(false, true, false, ev(true, false, Some(2))),
            StallVerdict::Grace
        );
        assert_eq!(
            stall_verdict(false, true, true, ev(true, false, Some(2))),
            StallVerdict::Abort
        );
    }

    #[test]
    fn trail_path_is_session_bounded() {
        let path = trail_path();
        assert!(path.ends_with(format!("trail-{}.jsonl", std::process::id())));
    }

    #[test]
    fn trail_appends_parseable_jsonl_and_stops_at_the_cap() {
        let dir = std::env::temp_dir().join(format!("officina-trail-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = trail_path_in(&dir);
        trail_at(&path, "tool.start", serde_json::json!({ "id": "call-1" }));
        trail_at(&path, "tool.end", serde_json::json!({ "id": "call-1" }));
        let body = std::fs::read_to_string(&path).expect("trail written");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value = serde_json::from_str(lines[0]).expect("json line");
        assert_eq!(first["event"], "tool.start");
        assert_eq!(first["fields"]["id"], "call-1");
        assert!(first["ts"].as_u64().is_some());
        // Bounded: an oversized trail accepts nothing more.
        std::fs::write(&path, vec![b'x'; (TRAIL_CAP + 1) as usize]).unwrap();
        trail_at(&path, "tool.start", serde_json::json!({}));
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            TRAIL_CAP + 1,
            "cap must stop appends"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
