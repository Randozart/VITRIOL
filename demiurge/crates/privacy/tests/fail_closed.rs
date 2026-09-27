//! Fail-closed behavior tests — the LESSONS-mandated error paths.
//!
//! Purpose: prove the two hard rules the P3 plan states:
//! 1. a scan ERROR is a refusal (`Result::Err`), never a pass-through;
//! 2. detection errors fail TOWARD privacy, never away.
//!
//! The gateway contract tests (`gateway/tests/privacy_lanes.rs`) build on
//! exactly these paths: the oversized message here is the same "poison
//! input" that must produce a 503 there.

use privacy::{PrivacyClass, PrivacyEngine, PrivacyError};

#[test]
fn oversized_message_is_refused_not_passed_through() {
    // The engine defines its refusal path via the size limit: a message
    // beyond it is a PrivacyError, and NO class is produced — the caller
    // cannot accidentally forward "the scan failed but here's a class".
    let engine = PrivacyEngine::with_limit(64).expect("patterns compile");
    let messages = vec!["x".repeat(65)];
    let error = engine
        .scan(&messages)
        .expect_err("over-limit message must refuse");
    assert!(
        matches!(
            error,
            PrivacyError::TooLarge {
                index: 0,
                bytes: 65,
                limit: 64
            }
        ),
        "refusal must name the offending message, got {error:?}"
    );
}

#[test]
fn refusal_names_the_first_oversized_message_index() {
    let engine = PrivacyEngine::with_limit(16).expect("patterns compile");
    let messages = vec!["clean".to_string(), "y".repeat(17), "z".repeat(99)];
    // Scanning stops at the FIRST refusal — message 0 was fine, message 1
    // is the poison one and must be named.
    let error = engine.scan(&messages).expect_err("must refuse");
    assert!(
        matches!(
            error,
            PrivacyError::TooLarge {
                index: 1,
                bytes: 17,
                limit: 16
            }
        ),
        "got {error:?}"
    );
}

#[test]
fn empty_and_multimessage_requests_scan_clean() {
    let engine = PrivacyEngine::new().expect("patterns compile");
    // Zero messages = nothing to detect = safe (and never an error).
    let empty: Vec<String> = Vec::new();
    assert_eq!(
        engine.scan(&empty).expect("empty request scans"),
        PrivacyClass::Safe
    );
    // Multi-message union: PII split across messages still combines.
    let split = vec![
        "Zet het over naar NL91ABNA0417164300.".to_string(),
        "aan".to_string(),
        "ten name van de buurman".to_string(),
    ];
    assert_eq!(
        engine.scan(&split).expect("union scan"),
        PrivacyClass::Confidential,
        "IBAN + name indicator across messages = combo"
    );
}

#[test]
fn uncertain_detection_fails_toward_privacy() {
    let engine = PrivacyEngine::new().expect("patterns compile");
    // A BSN-shaped 9-digit run that FAILS the checksum, next to the BSN
    // context word: the shape says "BSN attempt", the checksum cannot
    // confirm — the engine must NOT return safe.
    let typo = vec!["Mijn burgerservicenummer is 123456789.".to_string()];
    assert_eq!(
        engine.scan(&typo).expect("scans"),
        PrivacyClass::Sensitive,
        "context + checksum failure → sensitive, never safe"
    );
    // An IBAN context word with a mod-97-FAILING IBAN-shaped number: same
    // fail-toward-privacy rule for bank details with a typo.
    let iban_typo = vec!["Mijn rekeningnummer is NL91ABNA0417164301.".to_string()];
    assert_eq!(
        engine.scan(&iban_typo).expect("scans"),
        PrivacyClass::Sensitive,
        "bank context + mod-97 failure → sensitive"
    );
}

#[test]
fn process_wide_scan_helper_matches_engine_behavior() {
    // The free `scan()` fn uses the process-wide engine — same contract.
    let clean = vec!["gewoon een vraagje over tulpen.".to_string()];
    assert_eq!(
        privacy::scan(&clean).expect("default engine scans"),
        PrivacyClass::Safe
    );
    let with_bsn = vec!["BSN 111222333 kontroleren.".to_string()];
    assert_eq!(
        privacy::scan(&with_bsn).expect("default engine scans"),
        PrivacyClass::Confidential
    );
}
