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

    /// The answer must be EXACTLY this (T2, 2026-11-19), after normalisation.
    ///
    /// A substring guard cannot score a reasoning budget: an answer that reasons
    /// its way to the wrong conclusion while mentioning the right number passes
    /// `expect`. When this is set it wins over `expect`.
    pub expect_exact: Option<&'static str>,

    /// Seeded conversation history, rendered ahead of `user` (T1, 2026-11-19).
    ///
    /// `(role, content, reasoning)`. Used to price `preserve_thinking`, which
    /// re-renders every historical assistant turn's `<think>` block into every
    /// request: with history, two arms of the same conversation genuinely differ
    /// in prompt tokens. Empty = the historical single-turn shape.
    pub history: Vec<(&'static str, &'static str, Option<&'static str>)>,
}

/// Reasoning filler for the preserve-history shape. Deterministic on purpose:
/// the cost being measured is the re-rendered token count, so the text may not
/// vary between arms or repeats.
const SEEDED_REASONING: &str = "Checking the constraint against the evidence. The budget     appears in the setup message and must be quoted verbatim; recency is not the same as     correctness, so the earliest statement is the anchor. Consider the alternative readings:     a per-slot ceiling, a per-token ceiling, and a cumulative ceiling. Only the first is     stated. Therefore the fact has one value and it lives at the start of the conversation.";

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
            expect_exact: None,
            history: Vec::new(),
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
            expect_exact: None,
            history: Vec::new(),
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
            expect_exact: None,
            history: Vec::new(),
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
            expect_exact: None,
            history: Vec::new(),
        },
        // E2's consumer: a plan the operator reads through a rendered view, so a
        // compressed notation costs nothing if it decodes. The guard fact is a
        // distinctive number stated in the request.
        Shape {
            id: "internal-plan",
            note: "internal plan with a stated constraint (E2 decode target)",
            system: "You are planning an engine change. Be concrete and short.",
            user: "Plan how to cut decode latency at 70k filled context on two 12 GB GPUs. \
                   Hard constraints: no new weights, keep MTP, the mirror budget is \
                   cache_max_gb = 47. State the budget first, then the steps."
                .to_string(),
            schema: None,
            expect: Some("47"),
            expect_exact: None,
            history: Vec::new(),
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
                lines.join("\n")
            },
            schema: None,
            expect: Some("the quick brown fox"),
            expect_exact: None,
            history: Vec::new(),
        },
        // T1 (2026-11-19): the shape that makes `preserve_thinking` measurable.
        // Six assistant turns each carry a seeded `<think>` block; the engine
        // re-renders every one of them into every request unless
        // `--no-reasoning-preserve` is set, and nobody has ever priced that.
        // The fact asked for is in the FIRST turn, so a long prompt cannot be
        // mistaken for a good one.
        Shape {
            id: "preserve-history",
            note: "multi-turn with seeded reasoning — prices preserve_thinking",
            system: "You are in a long technical conversation. Answer the final question briefly.",
            user: "What KV cache budget did I state at the start? Answer with the number only."
                .to_string(),
            schema: None,
            expect: Some("61"),
            expect_exact: None,
            history: vec![
                ("user", "Setup: the KV cache budget for this bench is 61 GiB.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
                ("user", "Second note: the bench has two GPUs.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
                ("user", "Third note: the mirror lives on NVMe.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
                ("user", "Fourth note: restarts are warm.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
                ("user", "Fifth note: eviction is a fallback, not a knob.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
                ("user", "Sixth note: the recall guard matters more than the rate.", None),
                ("assistant", "Noted.", Some(SEEDED_REASONING)),
            ],
        },
        // T2 (2026-11-19): budgets need answers that are RIGHT or WRONG, not
        // answers that merely mention the right digits. Exact-match only.
        Shape {
            id: "exact-arith",
            note: "two-step arithmetic — exact answer, no partial credit",
            system: "Answer with only the integer. No words, no units.",
            user: "What is 37 * 24 + 19?".to_string(),
            schema: None,
            expect: None,
            expect_exact: Some("907"),
            history: Vec::new(),
        },
        Shape {
            id: "exact-logic",
            note: "one-hop combinatorics — exact answer",
            system: "Answer with only the integer. No words, no units.",
            user: "In a room of 5 people, every person shakes hands with every other person \
                   exactly once. How many handshakes happen in total?"
                .to_string(),
            schema: None,
            expect: None,
            expect_exact: Some("10"),
            history: Vec::new(),
        },
        // The easy fixtures above finish thinking in ~40 tokens, so every budget
        // above 64 is non-binding and the sweep says nothing (measured: identical
        // output across 0..2048). These two force real deliberation, which is
        // where a budget bound can actually bite.
        Shape {
            id: "exact-hard-mul",
            note: "five-digit product — thinking is load-bearing, answer exact",
            system: "Answer with only the integer. No words, no separators, no units.",
            user: "What is 12345 * 6789?".to_string(),
            schema: None,
            expect: None,
            expect_exact: Some("83810205"),
            history: Vec::new(),
        },
        Shape {
            id: "exact-hard-chain",
            note: "ten-step accumulation — order matters, answer exact",
            system: "Answer with only the integer. No words, no units.",
            user: "A tank starts empty. Each hour it gains that hour's number of litres \
                   (hour 1 gains 1, hour 2 gains 2, ... hour 10 gains 10) and then loses 3 \
                   litres. After 10 hours, how many litres remain?"
                .to_string(),
            schema: None,
            expect: None,
            expect_exact: Some("25"),
            history: Vec::new(),
        },
        Shape {
            id: "exact-trace",
            note: "loop trace — exact answer, wrong by one is wrong",
            system: "Answer with only the integer. No words, no units.",
            user: "Rust: `let mut x = 1; for i in 1..=4 { x = x * i + 1; }` — what is x at the \
                   end?"
                .to_string(),
            schema: None,
            expect: None,
            expect_exact: Some("65"),
            history: Vec::new(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_is_present_and_distinct() {
        let shapes = all(4);
        assert_eq!(shapes.len(), 12);
        let ids: Vec<_> = shapes.iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "structured",
                "structured-nested",
                "code-edit",
                "deep-recall",
                "internal-plan",
                "repeat-echo",
                "preserve-history",
                "exact-arith",
                "exact-logic",
                "exact-hard-mul",
                "exact-hard-chain",
                "exact-trace"
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
