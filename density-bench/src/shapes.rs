// Shapes: the three request kinds that dominate Officina's traffic (2026-11-18).
//
// A benchmark is only as good as its prompt set. These are deliberately shaped
// after what the harness actually sends: a machine-consumed verdict (the judge /
// decision lane / route lane), a realistic code-edit turn, and deep-context
// recall. Each carries the JSON schema the structured lanes would constrain to,
// so the baseline and grammar arms differ in exactly one field of the request.

use serde_json::{json, Value};

/// One benchmark case.
pub struct Shape {
    /// Stable id used in the results table.
    pub id: &'static str,
    /// What this case stands for, in the operator's terms.
    pub note: &'static str,
    pub system: &'static str,
    pub user: String,
    /// JSON schema for the machine-consumed lanes (`None` = prose answer).
    pub schema: Option<Value>,
    /// Substring the answer must contain (a *recall* guard).
    ///
    /// Eviction tuning (E6) trades memory for speed; without a fact the model
    /// must still retrieve after a sweep, a speed win is indistinguishable from
    /// amnesia. `None` = no recall requirement.
    pub expect: Option<&'static str>,
}

/// The fact the deep-recall guard asks for. It lives in **section 1** — as far
/// back as the prompt goes — so an answer proves retrieval, not recency.
pub const RECALL_FACT: &str = "cache_max_gb = 61";
/// The recalled token; distinctive enough that a lucky number cannot match.
pub const RECALL_EXPECT: &str = "61";

/// Filler that makes a deep-context prompt without being nonsense.
///
/// Section 1 carries [`RECALL_FACT`]; every other section is filler. Eviction
/// drops cells by score, and position 1 is exactly the kind of far-back cell a
/// score-driven sweep is tempted to drop — which is why the guard sits there.
fn deep_context(filler: &str, repeats: usize) -> String {
    (0..repeats)
        .map(|i| {
            if i == 1 {
                format!("### section {i}\n{filler} The budget is {RECALL_FACT}.\n")
            } else {
                format!("### section {i}\n{filler}\n")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The three shapes, at a requested context size.
///
/// `deep_sections` controls the deep-recall prompt length: the harness measures
/// FILLED tokens, so the caller states the depth rather than the token count.
pub fn all(deep_sections: usize) -> Vec<Shape> {
    vec![
        Shape {
            id: "structured",
            note: "machine-consumed verdict (judge / decision / route lane)",
            system: "You are a routing judge. Answer ONLY with JSON matching the schema. \
                     No prose, no code fences.",
            user: "Tool call: bash {\"command\": \"cargo test --lib\"} in a Rust project. \
                   Verdict: is this safe to run unattended, and which lane should serve it? \
                   Fields: verdict (safe|unsafe), lane (local|cloud), confidence (0-1), why (<=12 words)."
                .to_string(),
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "verdict": { "type": "string", "enum": ["safe", "unsafe"] },
                    "lane": { "type": "string", "enum": ["local", "cloud"] },
                    "confidence": { "type": "number" },
                    "why": { "type": "string" }
                },
                "required": ["verdict", "lane", "confidence", "why"],
                "additionalProperties": false
            })),
            expect: None,
        },
        Shape {
            id: "structured-nested",
            note: "nested verdict (array of findings) — where compliance usually breaks",
            system: "You are an auditor. Answer ONLY with JSON matching the schema. \
                     No prose, no code fences.",
            user: "Review these three diffs for risk. Return every finding with a rule id, \
                   severity and a one-line detail. Diffs: (1) a pre-commit hook is bypassed, \
                   (2) a bundled dependency version drops below 3, (3) a pre-existing hook \
                   is preserved untouched."
                .to_string(),
            schema: Some(json!({
                "type": "object",
                "properties": {
                    "findings": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "rule": { "type": "string" },
                                "severity": { "type": "string", "enum": ["low", "medium", "high"] },
                                "detail": { "type": "string" }
                            },
                            "required": ["rule", "severity", "detail"],
                            "additionalProperties": false
                        }
                    },
                    "summary": { "type": "string" }
                },
                "required": ["findings", "summary"],
                "additionalProperties": false
            })),
            expect: None,
        },
        Shape {
            id: "code-edit",
            note: "realistic patch request",
            system: "You are a careful Rust engineer. Patch the file; show only the changed lines.",
            user: "File src/tui/cache.rs currently formats a byte count with a helper `fmt_bytes`. \
                   Add a capacity readout that says how many KV states fit in the RAM cache and \
                   flags TIGHT when live sessions reach that count. Keep the existing style, \
                   no new dependencies, and explain the arithmetic in one comment."
                .to_string(),
            schema: None,
            expect: None,
        },
        Shape {
            id: "deep-recall",
            note: "deep-context question (states the depth it was built at)",
            system: "Answer from the document. If the fact is absent, say MISSING.",
            user: format!(
                "{}\n### question\nWhat is the exact cache budget value (cache_max_gb) stated in \
                 the second section? Reply with the number only, then one short sentence about \
                 the eviction policy.",
                deep_context(
                    "The engine mirrors slot states to disk under a budget, prunes by last access, \
                     and exempts the newest snapshot and the in-flight file. Config keys ride the \
                     fingerprint so drift is visible.",
                    deep_sections
                )
            ),
            schema: None,
            // The recall guard: the number appears in the filler, so an answer
            // that lost its memory cannot contain it.
            expect: Some(RECALL_EXPECT),
        },
        // Lookup-friendly shape for E3: the answer is largely a copy of the
        // prompt, which is the only condition where an n-gram drafter can pay
        // (it can only propose text it has already seen).
        Shape {
            id: "repeat-echo",
            note: "verbatim reuse — the case n-gram drafting is designed for",
            system: "Repeat the numbered lines the user gives, exactly, one per line.                      No commentary.",
            user: {
                let lines: Vec<String> = (0..24)
                    .map(|i| format!("{i}: the quick brown fox jumps over the lazy dog while the \
                                      engine mirrors slot states to disk under a budget"))
                    .collect();
                format!("{}", lines.join("\n"))
            },
            schema: None,
            expect: Some("the quick brown fox"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_is_present_and_distinct() {
        let shapes = all(4);
        assert_eq!(shapes.len(), 5);
        let ids: Vec<_> = shapes.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "structured",
                "structured-nested",
                "code-edit",
                "deep-recall",
                "repeat-echo"
            ]
        );
        for s in &shapes {
            assert!(!s.user.trim().is_empty(), "{} has no prompt", s.id);
            assert!(!s.system.trim().is_empty(), "{} has no system prompt", s.id);
        }
    }

    #[test]
    fn only_the_structured_shape_carries_a_schema() {
        let shapes = all(2);
        assert!(shapes[0].schema.is_some());
        assert!(shapes[1].schema.is_some());
        assert!(shapes[2].schema.is_none());
        assert!(shapes[3].schema.is_none());
        // The schema must be a real object schema, or the grammar arm is a no-op.
        let schema = shapes[0].schema.clone().unwrap();
        assert_eq!(schema["type"], "object");
        assert!(schema["required"].as_array().unwrap().len() >= 3);
    }

    #[test]
    fn deep_depth_scales_with_the_requested_sections() {
        let small = all(2)[3].user.len();
        let large = all(40)[3].user.len();
        assert!(
            large > small * 10,
            "depth must actually grow: {small} -> {large}"
        );
    }
}
