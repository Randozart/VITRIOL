//! `PrivacyClass` — the three-lane type every routing decision starts from.
//!
//! Purpose: one enum the gateway logs, the ledger stores and the policy
//! crate gates on. Ordered: `Safe < Sensitive < Confidential` — combining
//! evidence is a max() over this order, so "any PII → at least sensitive"
//! and "cloud-must-never-see → confidential" collapse into one comparison.
//! C# equivalent: `enum PrivacyClass { Safe, Sensitive, Confidential }`;
//! TS equivalent: `'safe' | 'sensitive' | 'confidential'`.

use serde::Serialize; // derive: the class prints as JSON in logs/ledger — TS: JSON.stringify

/// The privacy class of one chat request (the WHOLE request, not per
/// message — the scan unions evidence across all messages, see engine.rs).
///
/// Mapping (PLAN §2, also documented at `engine::decide`):
/// * any PII found → at minimum `Sensitive`;
/// * cloud-must-never-see categories (valid BSN, medical/legal indicators,
///   IBAN+name combo, credential/secret patterns) → `Confidential`;
/// * nothing found → `Safe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
// derive explained:
//   Debug      = {:#?} printing in logs — TS: console.dir
//   Clone/Copy = tiny enum passed by value, no borrow ceremony — TS: a plain string
//   PartialEq/Eq = `==` for tests — TS: ===
//   PartialOrd/Ord = the max() ordering (Safe < Sensitive < Confidential) —
//                TS has no direct equivalent; like a Java enum implementing Comparable
//   Serialize  = lowercase string form for logs + ledger (rename_all below)
#[serde(rename_all = "lowercase")]
// rename_all: serialize Confidential as "confidential" (logging contract) — TS: the union literal itself
pub enum PrivacyClass {
    /// No PII detected — the full model pool is eligible.
    Safe,
    /// Personal data found — EU + zero-data-retention endpoints only.
    Sensitive,
    /// Cloud-must-never-see data — local lane only, structurally.
    Confidential,
}

impl PrivacyClass {
    /// Lowercase wire/log name — the ledger column and tracing fields use
    /// this (TS: the string union value itself).
    pub fn as_str(self) -> &'static str {
        match self {
            // match: exhaustive over the enum — the compiler forces a new
            // variant to extend this mapping (TS: switch without default).
            PrivacyClass::Safe => "safe",
            PrivacyClass::Sensitive => "sensitive",
            PrivacyClass::Confidential => "confidential",
        }
    }
}

#[cfg(test)]
// cfg(test): compiled only under `cargo test` — TS: a test-only describe block
mod tests {
    use super::*; // the enum under test

    #[test]
    fn ordering_goes_safe_then_sensitive_then_confidential() {
        // The max()-combination contract the engine relies on: a higher
        // variant always dominates. TS equivalent: ['safe','sensitive',
        // 'confidential'].indexOf(a) <= indexOf(b).
        assert!(PrivacyClass::Safe < PrivacyClass::Sensitive);
        assert!(PrivacyClass::Sensitive < PrivacyClass::Confidential);
    }

    #[test]
    fn serializes_lowercase_for_logging() {
        // Logging/ledger contract: never Debug noise, always the lowercase
        // wire form (serde rename_all above).
        assert_eq!(
            serde_json_string(PrivacyClass::Confidential),
            "\"confidential\""
        );
    }

    /// Serialize through the real serde path (requires the derive) —
    /// helper kept tiny so the assert above reads like the wire.
    fn serde_json_string(class: PrivacyClass) -> String {
        serde_json::to_string(&class).expect("enum serialization is infallible")
    }
}
