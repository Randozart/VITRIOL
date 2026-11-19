// Engine client: one non-stream request (exact timings) plus one streamed request
// (TTFT), so a sample carries both the engine's own numbers and what the operator
// actually feels.
//
// Streaming parser is deliberately tolerant: llama.cpp SSE emits `data: {...}`
// lines, may include `data: [DONE]`, and a proxy may inject keep-alives. Garbage
// lines are skipped — a benchmark that dies on one bad line is worse than one
// that reports a slightly optimistic TTFT.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::time::Instant;

use crate::arms::RequestStyle;
use crate::codebook;
use crate::sample::{satisfies, Sample, Timings};
use crate::shapes::Shape;

/// Where the engine lives.
pub struct Client {
    pub base: String,
    pub model: String,
    /// Hard cap on generated tokens so the arms cannot diverge on length alone.
    pub max_tokens: u32,
    /// Whether to also run the streamed pass (TTFT). Off on a shared engine.
    pub stream: bool,
}

/// One SSE chunk's payload, if it carries content.
pub fn content_of(line: &str) -> Option<String> {
    let data = line.strip_prefix("data:")?.trim();
    if data.is_empty() || data == "[DONE]" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(data).ok()?;
    let delta = &value["choices"][0]["delta"]["content"];
    delta.as_str().map(|s| s.to_string())
}

/// The instruction a style adds to the system message, if any.
fn style_instruction(style: RequestStyle) -> Option<&'static str> {
    match style {
        RequestStyle::Baseline | RequestStyle::Grammar => None,
        RequestStyle::Telegraphic => {
            Some("Answer as tersely as possible: no articles, no filler, one clause per line.")
        }
        RequestStyle::Codebook => Some(codebook::LEGEND),
    }
}

/// Does this style promise a decodable notation? Used to decide whether the
/// expansion gate applies.
pub fn is_codec(style: RequestStyle) -> bool {
    matches!(style, RequestStyle::Telegraphic | RequestStyle::Codebook)
}

impl Client {
    /// Request body for one style. `Baseline` is the plain call; a style may only
    /// change instructions or `response_format`, never the shape's schema (I2).
    fn body(&self, shape: &Shape, style: RequestStyle, stream: bool) -> serde_json::Value {
        let mut body = serde_json::json!({
            "messages": [
                { "role": "system", "content": shape.system },
                { "role": "user", "content": shape.user }
            ],
            "max_tokens": self.max_tokens,
            "temperature": 0,
            "stream": stream,
        });
        // A decode style appends its instruction to the system message: the shape
        // decides the question, the style decides the notation.
        if let Some(suffix) = style_instruction(style) {
            let system = body["messages"][0]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            body["messages"][0]["content"] =
                serde_json::Value::String(format!("{system} {suffix}"));
        }
        if style == RequestStyle::Grammar {
            if let Some(schema) = &shape.schema {
                body["response_format"] = serde_json::json!({
                    "type": "json_schema",
                    "json_schema": { "name": "verdict", "strict": true, "schema": schema }
                });
            }
        }
        body
    }

    fn endpoint(&self) -> String {
        format!("{}/v1/chat/completions", self.base.trim_end_matches('/'))
    }

    /// Non-streamed pass: authoritative timings + the content we validate.
    /// Non-streamed pass: timings, content, reasoning chars, finish reason.
    fn timed(
        &self,
        shape: &Shape,
        style: RequestStyle,
    ) -> Result<(Timings, String, Option<usize>, Option<String>)> {
        let body = self.body(shape, style, false);
        let res = ureq::post(&self.endpoint())
            .set("content-type", "application/json")
            .timeout(std::time::Duration::from_secs(1800))
            .send_json(body)
            .with_context(|| format!("POST {}", self.endpoint()))?;
        let value: serde_json::Value = res.into_json().context("completion is not JSON")?;
        let timings = Timings::from_body(&value);
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        // Reasoning hygiene (2026-11-19): an unbudgeted thinker returns an empty
        // answer with finish_reason=length. Both numbers travel with the sample so
        // "the model said nothing" can never be mistaken for "the model was wrong".
        let reasoning_chars = value["choices"][0]["message"]["reasoning_content"]
            .as_str()
            .map(|r| r.chars().count());
        let finish_reason = value["choices"][0]["finish_reason"]
            .as_str()
            .map(str::to_string);
        Ok((timings, content, reasoning_chars, finish_reason))
    }

    /// Streamed pass: seconds to first content delta and a delta-measured rate.
    fn streamed(&self, shape: &Shape, style: RequestStyle) -> Result<(f64, f64)> {
        let body = self.body(shape, style, true);
        let started = Instant::now();
        let res = ureq::post(&self.endpoint())
            .set("content-type", "application/json")
            .timeout(std::time::Duration::from_secs(1800))
            .send_json(body)
            .with_context(|| format!("POST {}", self.endpoint()))?;
        let reader = BufReader::new(res.into_reader());
        let mut first: Option<f64> = None;
        let mut last: Option<f64> = None;
        let mut deltas = 0u64;
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if content_of(&line).is_none() {
                continue;
            }
            let at = started.elapsed().as_secs_f64();
            if first.is_none() {
                first = Some(at);
            }
            last = Some(at);
            deltas += 1;
        }
        let ttft = first.unwrap_or(f64::NAN);
        let span = match (first, last) {
            (Some(a), Some(b)) if b > a => b - a,
            _ => 0.0,
        };
        // Exclude the first token from the span: it is already counted in TTFT.
        let tps = if span > 0.0 && deltas > 1 {
            (deltas - 1) as f64 / span
        } else {
            0.0
        };
        Ok((ttft, tps))
    }

    /// Measure one sample: engine timings, TTFT, validity.
    /// Run one sample. `label` is recorded verbatim (engine knob + style), so an
    /// artifact always says what was actually asked.
    pub fn run(&self, shape: &Shape, label: &str, style: RequestStyle) -> Result<Sample> {
        let (timings, content, reasoning_chars, finish_reason) = self.timed(shape, style)?;
        let (ttft_s, stream_tps) = if self.stream {
            self.streamed(shape, style).unwrap_or((f64::NAN, 0.0))
        } else {
            (f64::NAN, 0.0)
        };
        // Expand before every guard: a compressed answer must still satisfy the
        // question it was asked (I4).
        let (expanded, suspicious): (String, String) = if is_codec(style) {
            (codebook::expand(&content), codebook::suspicious(&content))
        } else {
            (content.clone(), String::new())
        };
        let decoded = &expanded;
        let required: Vec<&str> = shape
            .schema
            .as_ref()
            .and_then(|s| s["required"].as_array())
            .map(|keys| keys.iter().filter_map(|k| k.as_str()).collect())
            .unwrap_or_default();
        let valid = shape.schema.as_ref().map(|_| satisfies(decoded, &required));
        let recall_ok = shape
            .expect
            .map(|needle| crate::sample::contains(decoded, needle));
        let answer_head = shape
            .expect
            .map(|_| content.trim().chars().take(200).collect());
        Ok(Sample {
            shape: shape.id.to_string(),
            arm: label.to_string(),
            timings,
            ttft_s,
            stream_tps,
            content_chars: content.trim().chars().count(),
            reasoning_chars,
            finish_reason,
            valid,
            recall_ok,
            answer_head,
            expanded_chars: is_codec(style).then(|| decoded.chars().count()),
            invented: (!suspicious.is_empty()).then_some(suspicious),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_content_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"Hel"}}]}"#;
        assert_eq!(content_of(line).as_deref(), Some("Hel"));
    }

    #[test]
    fn ignores_keep_alives_roles_and_done() {
        assert!(content_of("data: [DONE]").is_none());
        assert!(content_of(": keep-alive").is_none());
        assert!(content_of(r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#).is_none());
        assert!(content_of("data: not json").is_none());
        assert!(content_of("").is_none());
    }

    #[test]
    fn i1_baseline_body_is_a_golden_plain_request() {
        // Parity invariant: with no style, the request must be exactly what it was
        // before styles existed. Any future style work that leaks into the
        // baseline shows up here as a diff, not as a mysterious benchmark drift.
        let client = Client {
            base: "http://x".into(),
            model: "m".into(),
            max_tokens: 64,
            stream: false,
        };
        let shape = &crate::shapes::all(4)[0];
        let body = client.body(shape, RequestStyle::Baseline, false);
        let expected = serde_json::json!({
            "messages": [
                { "role": "system", "content": shape.system },
                { "role": "user", "content": shape.user }
            ],
            "max_tokens": 64,
            "temperature": 0,
            "stream": false,
        });
        assert_eq!(body, expected);
    }

    #[test]
    fn i2_no_style_changes_the_declared_schema() {
        // A style may add `response_format`; it may never edit the shape's schema,
        // or two arms would be measuring different questions.
        for shape in crate::shapes::all(4) {
            let client = Client {
                base: "http://x".into(),
                model: "m".into(),
                max_tokens: 64,
                stream: false,
            };
            let body = client.body(&shape, RequestStyle::Grammar, false);
            match (&shape.schema, body.get("response_format")) {
                (Some(schema), Some(attached)) => {
                    assert_eq!(&attached["json_schema"]["schema"], schema);
                }
                (None, None) => {}
                (declared, attached) => panic!(
                    "shape {} declares {:?} but attached {:?}",
                    shape.id,
                    declared.is_some(),
                    attached.is_some()
                ),
            }
        }
    }

    #[test]
    fn grammar_arm_attaches_the_schema_and_baseline_does_not() {
        let client = Client {
            base: "http://127.0.0.1:1".into(),
            model: "m".into(),
            max_tokens: 64,
            stream: false,
        };
        let all = crate::shapes::all(1);
        let shape = all
            .iter()
            .find(|s| s.id == "structured")
            .expect("structured shape");
        let plain = client.body(shape, RequestStyle::Baseline, false);
        assert!(plain.get("response_format").is_none());
        let grammar = client.body(shape, RequestStyle::Grammar, false);
        assert_eq!(grammar["response_format"]["type"], "json_schema");
        assert_eq!(
            grammar["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
    }

    #[test]
    fn a_shape_without_a_schema_never_gets_response_format() {
        // The prose shapes must stay comparable across arms: constraining them
        // would measure a different task.
        let client = Client {
            base: "http://127.0.0.1:1".into(),
            model: "m".into(),
            max_tokens: 64,
            stream: false,
        };
        let all = crate::shapes::all(1);
        let prose = all
            .iter()
            .find(|s| s.id == "code-edit")
            .expect("prose shape");
        let body = client.body(prose, RequestStyle::Grammar, false);
        assert!(body.get("response_format").is_none());
    }
}
