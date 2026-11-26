//! Recognition of slot-tagged engine log messages and terminal text cleanup.

use std::borrow::Cow;

/// Accept only the engine's SLT prefix, optionally wrapped by its logger.
pub fn slot_id(line: &str) -> Option<u64> {
    if !line.contains("slot ") {
        return None;
    }
    let clean = strip_ansi(line);
    let message = unwrap_message(&clean)?;
    let (prefix, message) = message.split_once("slot ")?;
    if !prefix.split_whitespace().all(|word| {
        matches!(word, "I" | "W" | "E" | "D")
            || (word.starts_with(|c: char| c.is_ascii_digit())
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.'))
    }) {
        return None;
    }
    slot_body(message).map(|(id, _)| id)
}

/// The sidebar has room for the slot and message before the log prefix.
pub fn compact_slot(line: &str) -> Option<String> {
    let line = unwrap_message(line)?;
    let (_, body) = line.split_once("slot ")?;
    let (id, message) = slot_body(body)?;
    Some(format!("#{id} {}", message.trim()))
}

fn unwrap_message(line: &str) -> Option<Cow<'_, str>> {
    if !line.starts_with('{') {
        return Some(Cow::Borrowed(line));
    }
    let event: serde_json::Value = serde_json::from_str(line).ok()?;
    if event["type"] != "log" {
        return None;
    }
    Some(Cow::Owned(event["msg"].as_str()?.to_owned()))
}

fn slot_body(body: &str) -> Option<(u64, &str)> {
    let (function, rest) = body.split_once(": id ")?;
    if function.trim().is_empty()
        || !function
            .trim()
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    let (id, task) = rest.split_once(" | task ")?;
    let (task_id, message) = task.split_once(" | ")?;
    task_id.parse::<i64>().ok()?;
    Some((id.trim().parse().ok()?, message))
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            skip_escape(&mut chars);
        } else {
            out.push(c);
        }
    }
    out
}

/// Consume the remainder of a CSI/OSC escape sequence from the iterator.
fn skip_escape(chars: &mut std::str::Chars<'_>) {
    for n in chars {
        if n == '\u{7}' || n == '~' || n.is_ascii_alphabetic() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_real_prefix_color_and_json_without_guessing_slot_ids() {
        assert_eq!(
            slot_id("0.31.288.992 I slot print_timing: id  0 | task 0 | draft acceptance = 0.14"),
            Some(0)
        );
        assert_eq!(
            slot_id("\x1b[31mE \x1b[0mslot      release: id 12 | task -1 | stopped"),
            Some(12)
        );
        assert_eq!(
            slot_id(
                r#"{"type":"log","msg":"slot launch_slot_: id  1 | task 7 | processing task\n"}"#
            ),
            Some(1)
        );
        assert_eq!(
            slot_id("srv notice: request mentions slot release: id  1 | task 3 | message"),
            None
        );
        assert_eq!(slot_id("slot release: id  1 | task bad | invalid"), None);
        assert_eq!(slot_id("slot release: id  1 | task 3"), None);
    }
}
