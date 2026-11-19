// Naming the empty turn (2026-11-19).
//
// Measured on the live engine: with thinking enabled and no token budget the reply
// arrives as `finish_reason=length` with `content: ""` and a full reasoning trace —
// thinking and the answer share the same `n_predict`. The engine now refuses that
// launch (scripts/vitriol reasoning_guard) and the client marker always pairs
// enable_thinking with a budget (`.pi/extensions/_shared/thinking.ts`), but an
// engine started by hand can still do it, and the operator must not be left staring
// at a blank assistant turn wondering whether the model failed.
//
// So the UI says what happened. This module owns that judgement and lives in its
// own file because `tui/state.rs` is at its size-ratchet budget.

use crate::tui::state::{AppState, ChatEntry};

/// Reasoning characters in the current turn that produced no visible answer.
///
/// Returns `None` when the turn answered, when nothing was thought, or while a tool
/// is still running (the turn is not over, so this is not a failure).
pub fn unanswered_reasoning(state: &AppState) -> Option<usize> {
    let mut reasoning = 0usize;
    for entry in state.entries.iter().rev() {
        match entry {
            // Still working: not a stalled turn.
            ChatEntry::Tool { running: true, .. } => return None,
            ChatEntry::Assistant(text) if !text.trim().is_empty() => return None,
            ChatEntry::Assistant(_) => {}
            ChatEntry::Thinking { text, .. } => reasoning += text.chars().count(),
            ChatEntry::User(_) => break,
            _ => {}
        }
    }
    (reasoning > 0).then_some(reasoning)
}

/// The notice to raise, or `None` when the turn is fine.
///
/// The text names the cause and the fix, because "empty reply" with no explanation
/// is what made this bug so hard to see for months.
pub fn empty_turn_notice(state: &AppState) -> Option<String> {
    let chars = unanswered_reasoning(state)?;
    Some(format!(
        "empty answer: thinking used the output budget ({chars} chars of reasoning, no reply) \
         — set reasoning_budget, or /engine for the flags"
    ))
}

/// Raise the notice on the state, if the finished turn needs one.
///
/// One line at the call site keeps `tui/mod.rs` (the largest file, at its ratchet
/// budget) from growing for a diagnostic.
pub fn raise_notice(state: &mut AppState) {
    if let Some(note) = empty_turn_notice(state) {
        state.notice = Some((note, "warn".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with(entries: Vec<ChatEntry>) -> AppState {
        let mut s = AppState::default();
        s.entries = entries;
        s
    }

    #[test]
    fn thinking_without_an_answer_is_named_not_silent() {
        // The bug this names: thinking consumed the budget and the turn was blank.
        let s = state_with(vec![
            ChatEntry::User("plan it".into()),
            ChatEntry::Thinking {
                text: "x".repeat(266),
                collapsed: true,
            },
            ChatEntry::Assistant(String::new()),
        ]);
        assert_eq!(unanswered_reasoning(&s), Some(266));
        let notice = empty_turn_notice(&s).expect("a notice");
        assert!(notice.contains("266"), "{notice}");
        assert!(notice.contains("reasoning_budget"), "{notice}");
    }

    #[test]
    fn a_normal_answer_never_raises_the_notice() {
        // Negative control: thinking plus a real answer is the healthy case.
        let s = state_with(vec![
            ChatEntry::User("plan it".into()),
            ChatEntry::Thinking {
                text: "thinking".into(),
                collapsed: true,
            },
            ChatEntry::Assistant("Red, blue, and yellow.".into()),
        ]);
        assert_eq!(unanswered_reasoning(&s), None);
        assert_eq!(empty_turn_notice(&s), None);

        // A turn that thought nothing is not our business either.
        let s = state_with(vec![
            ChatEntry::User("hi".into()),
            ChatEntry::Assistant(String::new()),
        ]);
        assert_eq!(empty_turn_notice(&s), None);
    }

    #[test]
    fn a_running_tool_suppresses_the_notice() {
        // Mid-tool is not stalled; a false alarm would train the operator to ignore it.
        let mut s = AppState::default();
        s.entries.push(ChatEntry::User("go".into()));
        s.entries.push(ChatEntry::Thinking {
            text: "hm".into(),
            collapsed: true,
        });
        s.tool_start(
            Some("c1"),
            "bash",
            &serde_json::json!({"command": "sleep 5"}),
        );
        assert_eq!(unanswered_reasoning(&s), None);
    }

    #[test]
    fn reasoning_from_a_previous_turn_does_not_count() {
        // The scan stops at the user message: an old thinking block must not make a
        // later short answer look like a budget failure.
        let s = state_with(vec![
            ChatEntry::User("first".into()),
            ChatEntry::Thinking {
                text: "old reasoning".into(),
                collapsed: true,
            },
            ChatEntry::Assistant("answered".into()),
            ChatEntry::User("second".into()),
            ChatEntry::Assistant("also answered".into()),
        ]);
        assert_eq!(unanswered_reasoning(&s), None);
    }
}
