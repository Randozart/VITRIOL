//! Scorer/compact bit-exact parity vs Python (captured 2026-08-07 from
//! `hermetis/scorer.py` + `hermetis/compact.py`, semantic-off path).

use libhermes::compact::{format_compact, format_episode, format_node};
use libhermes::scorer::{estimate_tokens, keyword_overlap};
use serde_json::json;

#[test]
fn keyword_overlap_bit_exact() {
    let cases: [(&str, &str, f64); 7] = [
        ("the quick fox", "the fox", 0.6666666666666666),
        ("write a rust function", "a rust function for parsing", 0.5),
        ("", "x", 0.0),
        (
            "alpha beta gamma delta",
            "beta gamma delta epsilon zeta",
            0.5,
        ),
        (
            "Fix the crash in the parser",
            "the parser crash is fixed now",
            0.375,
        ),
        ("multi  word   spacing", "spacing word multi", 1.0),
        ("unicode héllo wörld", "wörld héllo", 0.6666666666666666),
    ];
    for (q, c, expected) in cases {
        assert_eq!(
            keyword_overlap(q, c),
            expected,
            "keyword_overlap({q:?},{c:?})"
        );
    }
}

#[test]
fn estimate_tokens_bit_exact() {
    assert_eq!(estimate_tokens(""), 1);
    assert_eq!(estimate_tokens("abcdefgh"), 2);
    assert_eq!(estimate_tokens("abcdefghijklmnop"), 4);
}

#[test]
fn formatters_bit_exact() {
    assert_eq!(
        format_episode(
            &json!({"content": "hi", "role": "user", "created_at": "2026-08-07 10:00:00", "session_label": "s"}),
            None,
        ),
        "[2026-08-07] [s] user: hi"
    );
    assert_eq!(
        format_episode(
            &json!({"content": "abcdefghijklmno", "role": "a", "created_at": ""}),
            Some(4)
        ),
        " a: abcd…"
    );
    assert_eq!(
        format_node(&json!({"label": "l", "summary": "s", "strength": 0.9})),
        "[Consolidated: l] (●) s"
    );
    assert_eq!(
        format_node(&json!({"label": "l", "summary": "s", "strength": 0.3})),
        "[Consolidated: l] (○) s"
    );
    assert_eq!(
        format_compact(
            &json!({"content": "abcdefghijklmnopqrstuvwxyz\nsecond", "role": "user", "created_at": "2026-08-07 10:00:00"})
        ),
        "[2026-08-07] user: abcdefghijklmnopqrstuvwxyz second"
    );
}
