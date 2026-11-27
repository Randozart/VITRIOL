// PROVENANCE: independent parity checks against the repository's JavaScript
// Caduceus policy (Apache-2.0 OR MIT), evaluated by generate_caduceus_vectors.mjs.
use std::collections::BTreeMap;

use serde_json::Value;
use vitriol_core::caduceus::{candidate_order, should_retry};

fn fixtures() -> Value {
    serde_json::from_str(include_str!("../testdata/caduceus-policy.json")).unwrap()
}

fn parked(value: &Value) -> BTreeMap<String, f64> {
    value
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.as_f64().unwrap()))
        .collect()
}

#[test]
fn candidate_order_matches_javascript_including_unicode_trim() {
    for case in fixtures()["orders"].as_array().unwrap() {
        let fallbacks: Vec<&str> = case["fallbacks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap())
            .collect();
        let expected: Vec<String> = case["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s.as_str().unwrap().into())
            .collect();
        assert_eq!(
            candidate_order(
                case["requested"].as_str().unwrap(),
                &fallbacks,
                &parked(&case["parked"]),
                case["nowMs"].as_f64().unwrap()
            ),
            expected,
            "{case}"
        );
    }
}

#[test]
fn retry_matches_all_statuses_and_never_replays_forwarded_output() {
    for case in fixtures()["retries"].as_array().unwrap() {
        let status = Some(case["status"].as_u64().unwrap() as u16);
        assert_eq!(
            should_retry(status, false, false),
            case["expected"].as_bool().unwrap(),
            "{case}"
        );
        assert!(should_retry(status, false, true));
        assert!(!should_retry(status, true, false));
        assert!(!should_retry(status, true, true));
    }
    assert!(!should_retry(None, false, false));
    assert!(should_retry(None, false, true));
    assert!(!should_retry(None, true, true));
}
