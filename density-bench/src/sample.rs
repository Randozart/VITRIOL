// Sample: one measured request. The metrics are the ones the engine reports, so
// the benchmark cannot drift from what the operator sees.
//
// `timings` comes from the engine (llama.cpp) in the non-stream response and is
// authoritative: prompt_n / prompt_ms / predicted_n / predicted_ms. The streamed
// pass contributes TTFT and a decode rate measured from deltas, because TTFT is
// invisible to the non-stream call.

use serde::Serialize;

/// Engine-reported timings, verbatim (seconds/ms normalised to f64 seconds).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq)]
pub struct Timings {
    /// Prompt tokens actually processed (a prefix-cache hit reports only the delta).
    pub prompt_n: u64,
    pub prompt_s: f64,
    pub predicted_n: u64,
    pub predicted_s: f64,
}

impl Timings {
    /// Prompt throughput, tok/s. Zero when the engine processed nothing.
    pub fn prompt_tps(&self) -> f64 {
        if self.prompt_s > 0.0 {
            self.prompt_n as f64 / self.prompt_s
        } else {
            0.0
        }
    }

    /// Decode throughput, tok/s.
    pub fn decode_tps(&self) -> f64 {
        if self.predicted_s > 0.0 {
            self.predicted_n as f64 / self.predicted_s
        } else {
            0.0
        }
    }

    /// Parse `timings` out of a completion body, if present.
    pub fn from_body(body: &serde_json::Value) -> Self {
        let t = &body["timings"];
        let ms = |key: &str| t[key].as_f64().unwrap_or(0.0) / 1000.0;
        let n = |key: &str| t[key].as_u64().unwrap_or(0);
        Timings {
            prompt_n: n("prompt_n"),
            prompt_s: ms("prompt_ms"),
            predicted_n: n("predicted_n"),
            predicted_s: ms("predicted_ms"),
        }
    }
}

/// One measured arm-sample.
#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub shape: String,
    pub arm: String,
    pub timings: Timings,
    /// Seconds to the first streamed content delta (NaN when streaming failed).
    pub ttft_s: f64,
    /// Decode rate measured from streamed deltas (independent of `timings`).
    pub stream_tps: f64,
    /// Characters of assistant content (the thing we are trying to shrink).
    pub content_chars: usize,
    /// True when the content parses as JSON *and* satisfies the schema's required keys.
    pub valid: Option<bool>,
}

/// Median of a list, ignoring NaN. Empty input yields NaN rather than panicking.
pub fn median(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// Does this content satisfy the schema's `required` keys?
///
/// Deliberately shallow: the grammar guarantees the shape, and this check exists
/// to catch the *baseline* failing to comply — full JSON-Schema validation would
/// add a dependency for no extra signal.
pub fn satisfies(content: &str, required: &[&str]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content.trim()) else {
        return false;
    };
    required.iter().all(|key| value.get(*key).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timings_convert_ms_to_seconds_and_rate() {
        let body = serde_json::json!({
            "timings": { "prompt_n": 100, "prompt_ms": 500.0, "predicted_n": 40, "predicted_ms": 2000.0 }
        });
        let t = Timings::from_body(&body);
        assert_eq!(t.prompt_n, 100);
        assert!((t.prompt_tps() - 200.0).abs() < 1e-6);
        assert!((t.decode_tps() - 20.0).abs() < 1e-6);
    }

    #[test]
    fn a_full_prefix_cache_hit_reports_no_prompt_tokens() {
        // The engine reports 0 prompt tokens when the prefix is cached; the rate
        // must be 0, not a division by zero.
        let t = Timings {
            prompt_n: 0,
            prompt_s: 0.0,
            predicted_n: 10,
            predicted_s: 1.0,
        };
        assert_eq!(t.prompt_tps(), 0.0);
        assert_eq!(t.decode_tps(), 10.0);
    }

    #[test]
    fn median_handles_odd_even_and_empty() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert!(median(&[]).is_nan());
        assert_eq!(median(&[f64::NAN, 5.0]), 5.0);
    }

    #[test]
    fn validity_requires_parseable_json_with_every_required_key() {
        assert!(satisfies(
            "{\"verdict\":\"safe\",\"lane\":\"local\"}",
            &["verdict", "lane"]
        ));
        // Fenced output is what an unconstrained model does; it is NOT valid.
        assert!(!satisfies(
            "```json\n{\"verdict\":\"safe\"}\n```",
            &["verdict"]
        ));
        assert!(!satisfies("{\"verdict\":\"safe\"}", &["verdict", "lane"]));
        assert!(!satisfies("Sure — safe to run.", &["verdict"]));
    }
}
