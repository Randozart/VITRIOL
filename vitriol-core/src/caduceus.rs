//! Pure decision helpers for the Caduceus routing proxy (2026-11-16).
//!
//! PROVENANCE: port of `officina/scripts/caduceus-router-lib.mjs`
//! (Apache-2.0 OR MIT). The Rust implementation is a faithful port of the
//! pure, side-effect-free policy functions from that file; no sockets, files,
//! clocks or globals.

/// Request statuses that warrant retrying on the next candidate
/// (`RETRYABLE_STATUSES` in the JS original).
const RETRYABLE_STATUSES: [u16; 11] = [401, 403, 404, 408, 410, 429, 500, 502, 503, 504, 529];

/// ECMAScript `String.prototype.trim` character set (U+0009..U+000D,
/// U+0020, U+00A0, U+FEFF, U+1680, U+2000..U+200A, U+2028, U+2029,
/// U+202F, U+205F, U+3000). Note: U+0085 (NEL) and U+180E (Mongolian Vowel
/// Separator) are NOT trimmed per the reference JS behaviour, so they are
/// excluded here.
fn is_es_whitespace(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{D}' | '\u{20}' | '\u{A0}' | '\u{FEFF}')
        || matches!(
            c,
            '\u{1680}' | '\u{2000}'
                ..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
        )
}

/// Trim per ECMAScript semantics.
fn trim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_es_whitespace(s[start..].chars().next().unwrap()) {
        start += s[start..].chars().next().unwrap().len_utf8();
    }
    let mut end = bytes.len();
    while end > start && is_es_whitespace(s[..end].chars().next_back().unwrap()) {
        end -= s[..end].chars().next_back().unwrap().len_utf8();
    }
    &s[start..end]
}

/// Is `id` parked right now? Park keys are never trimmed.
fn is_parked(id: &str, parked: &std::collections::BTreeMap<String, f64>, now_ms: f64) -> bool {
    parked.get(id).is_some_and(|until| *until > now_ms)
}

/// Ordered candidates for one request: the requested model first, then
/// configured fallbacks that are not parked. Mirrors `candidateOrder`.
///
/// Trims each ID per ECMAScript `String.trim`, skips empty IDs, deduplicates
/// after trimming, and skips lanes parked into the future (`until > now_ms`).
pub fn candidate_order(
    requested: &str,
    fallbacks: &[&str],
    parked: &std::collections::BTreeMap<String, f64>,
    now_ms: f64,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for model in std::iter::once(requested).chain(fallbacks.iter().copied()) {
        let id = trim(model);
        if id.is_empty() {
            continue;
        }
        if out.iter().any(|o| o == id) {
            continue;
        }
        if is_parked(id, parked, now_ms) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}

/// Should this attempt be retried on the next candidate? Mirrors
/// `shouldRetry`: forwarding always suppresses retries (a partially streamed
/// answer must never be replayed), timeouts retry, otherwise only the exact
/// JS status set retries.
pub fn should_retry(status: Option<u16>, forwarded: bool, timed_out: bool) -> bool {
    if forwarded {
        return false;
    }
    if timed_out {
        return true;
    }
    status.is_some_and(|s| RETRYABLE_STATUSES.contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn trims_ecma_whitespace_including_bom() {
        assert_eq!(trim("  a\u{FEFF} "), "a");
        assert_eq!(trim("\u{1680}b\u{200A}"), "b");
        // Line/paragraph separators ARE trimmed by the reference JS.
        assert_eq!(trim("\u{2028}a\u{2029}"), "a");
        // NEL (U+0085) and Mongolian Vowel Separator (U+180E) are NOT trimmed.
        assert_eq!(trim("\u{85}c\u{85}"), "\u{85}c\u{85}");
        assert_eq!(trim("\u{180E}d"), "\u{180E}d");
    }

    #[test]
    fn candidate_order_pinned_first_dedup_parked() {
        let mut parked = BTreeMap::new();
        parked.insert("b".to_owned(), 200.0);
        parked.insert("c".to_owned(), 100.0);
        let now = 150.0;
        assert_eq!(
            candidate_order("  a ", &["a", " b", "c", "  "], &parked, now),
            vec!["a", "c"]
        );
    }

    #[test]
    fn candidate_order_unicode_ids() {
        let parked = BTreeMap::new();
        assert_eq!(
            candidate_order("münchen", &["  münchen ", "berlin"], &parked, 0.0),
            vec!["münchen", "berlin"]
        );
    }

    #[test]
    fn should_retry_matrix() {
        assert!(!should_retry(Some(404), true, false)); // forwarded always wins
        assert!(!should_retry(Some(404), true, true)); // forwarding suppresses even timeouts
        assert!(should_retry(None, false, true));
        assert!(should_retry(Some(429), false, false));
        assert!(should_retry(Some(529), false, false));
        assert!(!should_retry(Some(402), false, false));
        assert!(!should_retry(Some(200), false, false));
        assert!(!should_retry(None, false, false));
    }
}
