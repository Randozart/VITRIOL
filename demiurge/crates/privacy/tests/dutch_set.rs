//! The Dutch test set through the REAL engine — the P3 verification
//! (PLAN §7 P3: "per-category recall assertions, false-positive probes").
//!
//! The fixture (`dutch_testset.json`, committed) carries ~50 labeled cases:
//! per category the suite asserts RECALL (every expected-PII case lands in
//! at least its expected class) and the negative probes assert PRECISION
//! (checksum-failing lookalikes stay `safe`). This is the same fixture the
//! spike harness ran presidio-analyzer against — the numbers in
//! RESEARCH.md's spike section and the engine's green run here come from
//! one dataset, so the verdict is reproducible.

use std::collections::BTreeMap; // ordered category → counters, stable failure output

use privacy::{PrivacyClass, PrivacyEngine}; // the engine under test
use serde::Deserialize; // parse the fixture cases

/// The fixture's top-level shape: a description plus the cases array.
#[derive(Deserialize)]
struct Fixture {
    /// Human documentation of the mapping (not machine-checked).
    #[allow(dead_code)]
    description: String,
    /// The labeled cases.
    cases: Vec<Case>,
}

/// One fixture case — mirrors the JSON shape exactly.
#[derive(Deserialize)]
// derive Deserialize: serde_json::from_str parses each case — TS: a zod-parsed row
struct Case {
    /// Stable id for failure messages ("bsn-valid-01").
    #[allow(dead_code)] // kept for human readers of the fixture; used in msgs below via fmt
    id: String,
    /// Recognizer family the case exercises ("bsn", "negative", …).
    category: String,
    /// The request messages (multi-message cases exercise the union rule).
    messages: Vec<String>,
    /// The class the engine MUST return for this case.
    expected_class: String,
}

/// Per-category counters: (positives, recalled, exact, negatives, false_positives).
#[derive(Default)]
struct Counters {
    /// Cases whose expected class is not "safe".
    positives: usize,
    /// Positives that reached AT LEAST the expected class.
    recalled: usize,
    /// Positives that landed EXACTLY on the expected class.
    exact: usize,
    /// Cases whose expected class is "safe".
    negatives: usize,
    /// Negatives that produced any PII class instead.
    false_positives: usize,
}

#[test]
fn dutch_test_set_recalls_every_category_and_keeps_probes_safe() {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/dutch_testset.json"
    ))
    // concat! + env!: embed the fixture path at compile time — TS: path.resolve(__dirname, …)
    .expect("committed test set must be readable");
    let fixture: Fixture = serde_json::from_str(&raw).expect("committed test set must parse");
    let cases = fixture.cases;
    assert!(
        cases.len() >= 40,
        "the P3 fixture spec calls for 40-60 labeled cases, got {}",
        cases.len()
    );

    let engine = PrivacyEngine::new().expect("committed recognizer patterns compile");
    let mut per_category: BTreeMap<String, Counters> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        let class = engine
            .scan(&case.messages)
            .expect("fixture messages are within the scan limit");
        let expected = parse_class(&case.expected_class);
        let counters = per_category.entry(case.category.clone()).or_default();
        if expected == PrivacyClass::Safe {
            counters.negatives += 1;
            if class != PrivacyClass::Safe {
                counters.false_positives += 1;
                failures.push(format!(
                    "{} [{}]: expected safe, got {}",
                    case.id,
                    case.category,
                    class.as_str()
                ));
            }
        } else {
            counters.positives += 1;
            if class >= expected {
                counters.recalled += 1;
            }
            if class == expected {
                counters.exact += 1;
            }
            if class < expected {
                failures.push(format!(
                    "{} [{}]: expected at least {}, got {}",
                    case.id,
                    case.category,
                    expected.as_str(),
                    class.as_str()
                ));
            }
        }
    }

    // Recall gate per category: 100% of positives must reach at least the
    // expected class — the thesis depends on NEVER missing these.
    for (category, counters) in &per_category {
        assert_eq!(
            counters.recalled, counters.positives,
            "category '{category}' missed cases: {failures:?}"
        );
    }
    // Precision gate: NO negative probe may light up — checksums exist
    // precisely so lookalikes stay safe.
    assert!(
        failures.is_empty(),
        "test-set failures (recall or false-positive): {failures:?}"
    );
}

/// Parse the fixture's class string into the enum (fixture uses lowercase).
fn parse_class(text: &str) -> PrivacyClass {
    match text {
        "safe" => PrivacyClass::Safe,
        "sensitive" => PrivacyClass::Sensitive,
        "confidential" => PrivacyClass::Confidential,
        other => panic!("fixture carries an unknown class '{other}'"),
    }
}

#[test]
fn per_category_counts_match_the_fixture_spec() {
    // Guard against silently shrinking the fixture: every category from
    // the P3 plan must stay represented with at least the committed count.
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/dutch_testset.json"
    ))
    .expect("committed test set must be readable");
    let fixture: Fixture = serde_json::from_str(&raw).expect("fixture parses");
    let cases = fixture.cases;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for case in &cases {
        *counts.entry(case.category.clone()).or_default() += 1;
    }
    for (category, minimum) in [
        ("bsn", 6),
        ("bsn_context", 3),
        ("iban", 4),
        ("iban_name_combo", 4),
        ("phone", 5),
        ("postcode", 3),
        ("email", 2),
        ("medical", 3),
        ("legal", 3),
        ("secret", 3),
        ("negative", 8),
        ("mixed", 2),
    ] {
        assert!(
            counts.get(category).copied().unwrap_or(0) >= minimum,
            "category '{category}' must keep at least {minimum} cases (fixture shrinkage)"
        );
    }
}
