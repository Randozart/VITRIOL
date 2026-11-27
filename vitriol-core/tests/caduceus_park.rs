// PROVENANCE: independent parity tests against this repository's Caduceus
// JavaScript (Apache-2.0 OR MIT), including numeric edge behaviour.
use serde_json::Value;
use std::collections::BTreeMap;
use vitriol_core::caduceus::{
    capped_park_ms, give_up_message, prune_parked, ParkState, DEFAULT_MAX_PARK_MS,
};

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
fn cap_matches_js_including_caps_smaller_than_the_floor() {
    for case in fixtures()["caps"].as_array().unwrap() {
        assert_eq!(
            capped_park_ms(case["hint"].as_f64(), case["max"].as_f64()),
            case["expected"].as_f64(),
            "{case}"
        );
    }
    assert_eq!(DEFAULT_MAX_PARK_MS, 900_000.0);
    assert_eq!(
        capped_park_ms(Some(1_000_000.0), Some(DEFAULT_MAX_PARK_MS)),
        Some(900_000.0)
    );
    assert_eq!(capped_park_ms(Some(f64::NAN), None), None);
    assert_eq!(capped_park_ms(Some(f64::INFINITY), None), None);
    assert_eq!(capped_park_ms(Some(f64::NEG_INFINITY), None), None);
    assert_eq!(
        capped_park_ms(Some(2000.5), Some(f64::INFINITY)),
        Some(2000.0)
    );
    assert!(capped_park_ms(Some(2000.5), Some(f64::NAN))
        .unwrap()
        .is_nan());
}
#[test]
fn pruning_preserves_input_and_carries_only_live_reasons() {
    for case in fixtures()["prunes"].as_array().unwrap() {
        let input = &case["state"];
        let state = ParkState {
            parked: parked(&input["parked"]),
            reasons: serde_json::from_value(input["reasons"].clone()).unwrap(),
        };
        let original = state.clone();
        let output = prune_parked(&state, case["nowMs"].as_f64().unwrap());
        assert_eq!(output.parked, parked(&case["expected"]["parked"]));
        assert_eq!(
            output.reasons,
            serde_json::from_value::<BTreeMap<String, String>>(case["expected"]["reasons"].clone())
                .unwrap()
        );
        assert_eq!(state, original);
    }
    let state = ParkState {
        parked: BTreeMap::from([
            ("nan".into(), f64::NAN),
            ("inf".into(), f64::INFINITY),
            ("?".into(), f64::INFINITY),
        ]),
        reasons: BTreeMap::new(),
    };
    assert_eq!(
        prune_parked(&state, 1000.0).parked,
        BTreeMap::from([("inf".into(), f64::INFINITY)])
    );
}
#[test]
fn give_up_matches_js_rounding_and_phantom_filter() {
    for case in fixtures()["reports"].as_array().unwrap() {
        assert_eq!(
            give_up_message(
                case["tried"].as_u64().unwrap(),
                &parked(&case["parked"]),
                case["nowMs"].as_f64().unwrap()
            ),
            case["expected"].as_str().unwrap(),
            "{case}"
        );
    }
    let infinite = BTreeMap::from([("inf".into(), f64::INFINITY), ("nan".into(), f64::NAN)]);
    assert_eq!(
        give_up_message(2, &infinite, 1000.0),
        "all 2 candidate(s) unavailable — earliest lane returns in Infinitys"
    );
}
