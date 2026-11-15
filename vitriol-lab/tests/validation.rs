use vitriol_lab::validation::{self, Calibration};

fn fixture() -> Calibration {
    serde_json::from_value(serde_json::json!({"version":1,"measurement_origin":"synthetic","held_out":true,"engine_fingerprint":"engine-v1","hardware_fingerprint":"hardware-v1","trace_sha256":"0".repeat(64),"baseline":"base","max_relative_error":0.1,"minimum_effect_fraction":0.02,"measurements":[{"name":"base","predicted_us":1000,"measured_us":1000},{"name":"candidate","predicted_us":550,"measured_us":500}]})).unwrap()
}

#[test]
fn a_resolved_improvement_with_acceptable_error_passes() {
    let report = validation::check(&fixture()).unwrap();
    assert_eq!(report.status, "passed");
    assert_eq!(report.resolved_comparisons, 1);
    assert!(!report.hardware_verified);
}

#[test]
fn small_differences_inside_the_error_bound_remain_unresolved() {
    let mut input = fixture();
    input.measurements[1].predicted_us = 990.0;
    input.measurements[1].measured_us = 990.0;
    assert_eq!(validation::check(&input).unwrap().status, "inconclusive");
}

#[test]
fn wrong_ordering_or_large_prediction_error_fails() {
    let mut input = fixture();
    input.measurements[1].predicted_us = 1200.0;
    let report = validation::check(&input).unwrap();
    assert_eq!(report.status, "failed");
    assert_eq!(report.comparisons[0].ordering, "disagrees");
}

#[test]
fn missing_baseline_duplicate_candidates_and_invalid_numbers_are_rejected() {
    let mut input = fixture();
    input.held_out = false;
    assert!(validation::check(&input).is_err());
    let mut input = fixture();
    input.measurements[1].measured_us = f64::MIN_POSITIVE;
    assert!(validation::check(&input).is_err());
    let mut input = fixture();
    input.baseline = "missing".into();
    assert!(validation::check(&input).is_err());
    let mut input = fixture();
    input.measurements[1].name = "base".into();
    assert!(validation::check(&input).is_err());
    let mut input = fixture();
    input.measurements[1].measured_us = 0.0;
    assert!(validation::check(&input).is_err());
}
