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
    s.trim_matches(is_es_whitespace)
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

/// Upper bound on hint-driven parks: one flaky 429 must not bench a lane for
/// hours (`DEFAULT_MAX_PARK_S * 1000` in the JS original). Milliseconds.
pub const DEFAULT_MAX_PARK_MS: f64 = 900_000.0;

/// Parked-lane state: `parked` maps model → until-timestamp (ms), `reasons`
/// maps model → operator-facing reason.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParkState {
    pub parked: std::collections::BTreeMap<String, f64>,
    pub reasons: std::collections::BTreeMap<String, String>,
}

/// Park duration for a hint-driven failure: the hint floored to an integer and
/// at least 1000 ms, clamped to the operator's ceiling
/// (`cappedParkMs` in the JS original).
///
/// `None` hint (JS non-number) yields `None`. Non-finite hints are rejected.
/// `None` max (JS non-number) or `max <= 0` disables the cap; otherwise
/// `min(floor, max)`, matching JS `Math.min` — including `Some(NaN)` for a NaN
/// max. A `+Infinity` max leaves the finite floor unchanged.
pub fn capped_park_ms(hint_ms: Option<f64>, max_park_ms: Option<f64>) -> Option<f64> {
    let hint = hint_ms?;
    if !hint.is_finite() {
        return None;
    }
    let floor = 1_000_f64.max(hint.floor());
    match max_park_ms {
        // JS Math.min propagates NaN: a NaN max must yield Some(NaN).
        Some(max) if max.is_nan() => Some(max),
        Some(max) if max > 0.0 => Some(floor.min(max)),
        _ => Some(floor),
    }
}

/// Drop entries whose time has passed (`pruneParked` in the JS original).
///
/// Copies only future numeric timestamps (`until > now_ms`; `+Infinity` is a
/// future timestamp, `NaN` is not), drops the exact phantom key `"?"`, carries
/// the matching reason (empty string when absent), and does not mutate the
/// input.
pub fn prune_parked(state: &ParkState, now_ms: f64) -> ParkState {
    let mut next = ParkState::default();
    for (model, until) in &state.parked {
        if model == "?" {
            continue;
        }
        // The `> now_ms` test already excludes NaN and admits +Infinity.
        if *until > now_ms {
            next.parked.insert(model.clone(), *until);
            next.reasons.insert(
                model.clone(),
                state.reasons.get(model).cloned().unwrap_or_default(),
            );
        }
    }
    next
}

/// One line for the operator when every candidate is parked or failed
/// (`giveUpMessage` in the JS original).
///
/// Ignores the phantom key `"?"`, picks the earliest future timestamp, rounds
/// the remaining seconds with `Math.round` semantics and a 1-second minimum,
/// and formats exactly `all N candidate(s) unavailable — earliest lane returns
/// in Ss` or `all N candidate(s) unavailable — no lane is parked`. A sole
/// infinite future timestamp renders as `Infinitys`.
///
/// Compatibility quirk of the JS original: a selected zero timestamp is
/// falsy, so `giveUpMessage({parked: {a: 0}, nowMs: -1})` reports
/// "no lane is parked". Preserved here.
pub fn give_up_message(
    tried: u64,
    parked: &std::collections::BTreeMap<String, f64>,
    now_ms: f64,
) -> String {
    let soonest = parked
        .iter()
        .filter_map(|(model, until)| {
            // Numeric future timestamp only: `NaN` is not numeric; the
            // phantom key "?" is never a lane.
            if model.as_str() != "?" && *until > now_ms {
                Some(*until)
            } else {
                None
            }
        })
        .min_by(|a, b| a.total_cmp(b));
    let when = match soonest {
        // JS `soonest ? ... : "no lane is parked"`: a zero timestamp is
        // falsy and takes the "no lane" branch.
        Some(t) if t != 0.0 => {
            let seconds = ((t - now_ms) / 1000.0).round().max(1.0);
            let text = if t.is_infinite() {
                "Infinity".to_string()
            } else {
                seconds.to_string()
            };
            format!("earliest lane returns in {text}s")
        }
        _ => "no lane is parked".to_string(),
    };
    format!("all {tried} candidate(s) unavailable — {when}")
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
