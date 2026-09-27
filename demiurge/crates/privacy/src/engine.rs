//! `PrivacyEngine` — the scan: ALL messages in, ONE class out, or a refusal.
//!
//! Purpose: per request, run every recognizer over every message, union the
//! evidence, and apply the class mapping. Contract (PLAN §7 P3, LESSONS):
//! * **ALL messages** are scanned — never the last one alone (v1 bug);
//! * a scan ERROR ([`PrivacyError`]) is a refusal — the caller (gateway)
//!   must 503 and dispatch nothing; there is no "best effort" class;
//! * uncertain detection fails TOWARD privacy (context word + digit run
//!   without a valid checksum → at least `sensitive`);
//! * checksum verdicts are facts: a mod-11-validated BSN or a name glued
//!   to a mod-97-validated IBAN is `confidential`.
//!
//! Mapping table (PLAN §2; mirrored in RESEARCH.md's spike section):
//! | evidence (union over ALL messages)                       | class        |
//! |----------------------------------------------------------|--------------|
//! | valid BSN (mod-11)                                        | confidential |
//! | medical-indicator term                                    | confidential |
//! | legal-indicator term                                      | confidential |
//! | secret path / API-key token                               | confidential |
//! | valid NL IBAN + name indicator (conversation union)       | confidential |
//! | valid NL IBAN alone                                       | sensitive    |
//! | phone (mobile or 9-12-digit generic, non-date)            | sensitive    |
//! | postcode / email                                          | sensitive    |
//! | BSN-context word + digit run (checksum failed/absent)     | sensitive    |
//! | IBAN-context word + IBAN-shaped run failing mod-97        | sensitive    |
//! | nothing                                                   | safe         |
//!
//! Complexity: O(total request bytes) — one linear pass per recognizer per
//! message (a dozen cheap `is_match` runs beat index arithmetic over a
//! RegexSet for clarity at chat-message scale); checksums are O(digits).
//! Praetor: no nested loops, no O(n²).

use std::sync::OnceLock; // std's once-initialized cell — TS: a module-level lazy singleton

use regex::Regex; // compiled pattern — TS: RegExp

use crate::class::PrivacyClass; // the result type
use crate::error::PrivacyError; // the refusal taxonomy
use crate::patterns; // the recognizer constants + checksum validators

/// Default per-message scan limit (128 KiB ≈ 32k tokens of text): beyond
/// this a message is refused (`PrivacyError::TooLarge`), not scanned.
/// Rationale: bounded work per request + a bounded privacy surface.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 128 * 1024;

/// The recognizer set, compiled once at engine construction. One field per
/// pattern so every check reads by NAME, never by a positional index
/// (index arithmetic over a `RegexSet` is exactly the kind of silent-drift
/// bug this repo exists to prevent: add a pattern mid-list and every arm
/// shifts).
/// TS equivalent: `const RES = { mobile: /…/, generic: /…/, … }`.
struct Recognizers {
    /// Dutch mobile shape (06 / +31 6 / 0031 6 + 8 digits).
    mobile: Regex,
    /// Generic long digit run (needs the digit-count check in code).
    generic: Regex,
    /// Date shapes — suppresses `generic` (VITRIOL's false-positive guard).
    dateish: Regex,
    /// Dutch postcode `1234 AB`.
    postcode: Regex,
    /// Email address.
    email: Regex,
    /// NL IBAN shape (validated by mod-97 in code).
    iban: Regex,
    /// 9-digit BSN shape (validated by mod-11 in code).
    bsn: Regex,
    /// "burgerservicenummer"/"bsn" context words.
    bsn_context: Regex,
    /// bank-account context words.
    iban_context: Regex,
    /// "ten name van"/"t.n.v."/"t.a.v."/"rekeninghouder"/"naam:".
    name_indicator: Regex,
    /// medical-indicator terms.
    medical: Regex,
    /// legal-indicator terms.
    legal: Regex,
    /// secret-file paths (.env/.ssh/keys/secrets/credentials).
    secret_path: Regex,
    /// API-key token shapes (sk-…, ghp_…, AIzaSy…, xox…).
    api_key: Regex,
}

impl Recognizers {
    /// Compile every pattern; any compile failure refuses construction
    /// (fail-closed at startup, never mid-request).
    fn compile() -> Result<Self, PrivacyError> {
        // Local helper: one regex source → compiled Regex or a Compile
        // refusal carrying the regex crate's own message.
        let build = |source: &str| {
            Regex::new(source).map_err(|error| PrivacyError::Compile(error.to_string()))
        };
        Ok(Self {
            mobile: build(patterns::PHONE_MOBILE)?,
            generic: build(patterns::PHONE_GENERIC)?,
            dateish: build(patterns::DATEISH)?,
            postcode: build(patterns::POSTCODE)?,
            email: build(patterns::EMAIL)?,
            iban: build(patterns::IBAN_NL)?,
            bsn: build(patterns::BSN_SHAPE)?,
            bsn_context: build(patterns::BSN_CONTEXT)?,
            iban_context: build(patterns::IBAN_CONTEXT)?,
            name_indicator: build(patterns::NAME_INDICATOR)?,
            medical: build(patterns::MEDICAL_TERMS)?,
            legal: build(patterns::LEGAL_TERMS)?,
            secret_path: build(patterns::SECRET_PATH)?,
            api_key: build(patterns::API_KEY)?,
        })
    }
}

/// The compiled, ready-to-scan engine. Build ONCE per process (the regexes
/// cost milliseconds) and share — the gateway holds it in app state.
/// C# equivalent: a singleton `PiiScanner` registered at startup.
pub struct PrivacyEngine {
    /// Refusal threshold for any single message.
    max_message_bytes: usize,
    /// The compiled recognizer set.
    recognizers: Recognizers,
}

impl PrivacyEngine {
    /// Compile the recognizer set. Fails only if a pattern is malformed —
    /// which the committed test suite proves cannot happen — so callers
    /// may `expect` (a broken recognizer refuses STARTUP, never a request).
    pub fn new() -> Result<Self, PrivacyError> {
        Self::with_limit(DEFAULT_MAX_MESSAGE_BYTES)
    }

    /// Same, with a custom refusal limit (tests shrink it to exercise the
    /// refusal path without shipping 128 KiB payloads).
    pub fn with_limit(max_message_bytes: usize) -> Result<Self, PrivacyError> {
        Ok(Self {
            max_message_bytes,
            recognizers: Recognizers::compile()?,
        })
    }

    /// Scan ALL messages of one request. Returns the most protective class
    /// any evidence demands, or a refusal. O(total bytes).
    pub fn scan(&self, messages: &[String]) -> Result<PrivacyClass, PrivacyError> {
        // Evidence accumulates across the WHOLE conversation: PII in
        // message[0] with a clean last message still counts (v1 bug pinned
        // by test). Combos (IBAN + name indicator) are evaluated on the
        // union, not per message.
        let mut evidence = Evidence::default();
        for (index, message) in messages.iter().enumerate() {
            // Fail-closed size gate BEFORE any regex work runs.
            let bytes = message.len();
            if bytes > self.max_message_bytes {
                return Err(PrivacyError::TooLarge {
                    index,
                    bytes,
                    limit: self.max_message_bytes,
                });
            }
            self.scan_message(message, &mut evidence);
        }
        Ok(evidence.decide())
    }

    /// Run the recognizers over ONE message, OR-ing matches into `evidence`.
    /// Split into three passes so each stays well inside the Praetor
    /// complexity budget: plain indicators, checksum-validated shapes, and
    /// the generic digit-run heuristic.
    fn scan_message(&self, message: &str, evidence: &mut Evidence) {
        self.scan_plain_indicators(message, evidence);
        self.scan_checksum_shapes(message, evidence);
        self.scan_generic_phone(message, evidence);
    }

    /// Pass 1 — the plain yes/no recognizers (no per-match work). Each
    /// check reads its recognizer by name; truthiness ORs into the union
    /// (never clears a flag).
    fn scan_plain_indicators(&self, message: &str, evidence: &mut Evidence) {
        if self.recognizers.mobile.is_match(message) {
            evidence.phone = true;
        }
        if self.recognizers.postcode.is_match(message) {
            evidence.postcode = true;
        }
        if self.recognizers.email.is_match(message) {
            evidence.email = true;
        }
        if self.recognizers.dateish.is_match(message) {
            evidence.dateish = true;
        }
        if self.recognizers.bsn_context.is_match(message) {
            evidence.bsn_context = true;
        }
        if self.recognizers.iban_context.is_match(message) {
            evidence.iban_context = true;
        }
        if self.recognizers.name_indicator.is_match(message) {
            evidence.name_indicator = true;
        }
        if self.recognizers.medical.is_match(message) {
            evidence.medical = true;
        }
        if self.recognizers.legal.is_match(message) {
            evidence.legal = true;
        }
        // A secret-file path OR an API-key token both mean "credentials".
        if self.recognizers.secret_path.is_match(message)
            || self.recognizers.api_key.is_match(message)
        {
            evidence.secret = true;
        }
    }

    /// Pass 2 — the checksum-validated shapes. A shape match is split into
    /// a VERDICT: any match passing its checksum is fact (`*_valid`);
    /// matches that all fail stay an uncertain path (`*_failed`) that the
    /// context words escalate (fail toward privacy, module docs).
    fn scan_checksum_shapes(&self, message: &str, evidence: &mut Evidence) {
        // IBAN: any match passing mod-97 → fact.
        if self.recognizers.iban.is_match(message) {
            // `find_iter` → iterator of matches; `.any` short-circuits on
            // the first valid one — TS: `[...text.matchAll(re)].some(…)`.
            let any_valid = self
                .recognizers
                .iban
                .find_iter(message)
                .any(|hit| patterns::nl_iban_mod97(hit.as_str()));
            if any_valid {
                evidence.iban_valid = true;
            } else {
                evidence.iban_failed = true;
            }
        }
        // BSN: same verdict split with the mod-11 eleven-test.
        if self.recognizers.bsn.is_match(message) {
            let any_valid = self
                .recognizers
                .bsn
                .find_iter(message)
                .any(|hit| patterns::bsn_valid(hit.as_str()));
            if any_valid {
                evidence.bsn_valid = true;
            } else {
                evidence.bsn_shape_failed = true;
            }
        }
    }

    /// Pass 3 — the generic long-number heuristic, ported from VITRIOL:
    /// normalize to digits, require a phone-ish 9-12 count, and skip
    /// entirely when the message is date-shaped (the field-tested
    /// false-positive guard). 9-digit runs that are really BSN attempts
    /// were verdict-classified in pass 2; this rule catches the leftovers
    /// (landlines, international numbers) and errs toward sensitive.
    fn scan_generic_phone(&self, message: &str, evidence: &mut Evidence) {
        if evidence.phone || evidence.dateish || !self.recognizers.generic.is_match(message) {
            return; // already detected, or date-guarded, or no long run at all
        }
        // `break` after the first phone-like run — one is enough.
        for hit in self.recognizers.generic.find_iter(message) {
            let digits = patterns::normalize_digits(hit.as_str());
            if (9..=12).contains(&digits.len()) {
                evidence.phone = true;
                break;
            }
        }
    }
}

/// Evidence accumulated across all messages of one request. Plain bools —
/// the class decision is a pure function of these (unit-testable without
/// any message text).
#[derive(Debug, Default, Clone, Copy)]
// derive: Default = all-false instance (the "clean" starting point — TS: `{...emptyFlags}`)
struct Evidence {
    /// Mobile-phone shape matched.
    phone: bool,
    /// Message is date-shaped (suppresses the generic phone rule).
    dateish: bool,
    /// Dutch postcode matched (canonical uppercase shape).
    postcode: bool,
    /// Email address matched.
    email: bool,
    /// burgerservicenummer/bsn context word matched.
    bsn_context: bool,
    /// bank-account context word matched.
    iban_context: bool,
    /// name indicator matched (ten name van / t.n.v. / t.a.v. / …).
    name_indicator: bool,
    /// medical-indicator term matched.
    medical: bool,
    /// legal-indicator term matched.
    legal: bool,
    /// secret path or API-key token matched.
    secret: bool,
    /// At least one 9-digit run passed the mod-11 eleven-test.
    bsn_valid: bool,
    /// At least one NL-IBAN-shaped match passed mod-97.
    iban_valid: bool,
    /// IBAN-shaped matches exist but all failed mod-97.
    iban_failed: bool,
    /// 9-digit run present but mod-11 refused it (BSN-shaped near-miss).
    bsn_shape_failed: bool,
}

impl Evidence {
    /// Apply the mapping table (module docs) — a pure function, so the
    /// class logic is testable without regexes at all. Split into the two
    /// mapping rows so each predicate stays simple.
    fn decide(&self) -> PrivacyClass {
        if self.is_confidential() {
            return PrivacyClass::Confidential;
        }
        if self.is_sensitive() {
            return PrivacyClass::Sensitive;
        }
        PrivacyClass::Safe
    }

    /// Mapping row 1 — the cloud-must-never-see list.
    fn is_confidential(&self) -> bool {
        self.bsn_valid
            || self.medical
            || self.legal
            || self.secret
            // IBAN + name indicator anywhere in the conversation = a named
            // person's bank details (the combo rule; union across messages).
            || (self.iban_valid && self.name_indicator)
    }

    /// Mapping row 2 — any remaining personal data, plus the fail-toward-
    /// privacy paths (context words with an un-checksumable number — a
    /// failed checksum WITHOUT its context word is a definite negative,
    /// never a hit: that is what the checksums are for). The two
    /// "uncertain" conjunctions are combined via `matches!`-style named
    /// locals so each row of the mapping table stays a separate predicate.
    fn is_sensitive(&self) -> bool {
        // Fail-toward-privacy row A: BSN context word + an unverified
        // digit run (checksum refused, or a phone-like long run nearby).
        let bsn_uncertain = self.bsn_context && (self.bsn_shape_failed || self.phone);
        // Fail-toward-privacy row B: bank context word + a mod-97 failure.
        let iban_uncertain = self.iban_context && self.iban_failed;
        self.phone
            || self.postcode
            || self.email
            || self.iban_valid
            || bsn_uncertain
            || iban_uncertain
    }
}

/// Process-wide engine behind `scan()` — built once, shared by every call.
/// `OnceLock` = std's lazy singleton (first caller initializes, the rest
/// read) — TS: `const engine = getEngine()` module memoization.
static ENGINE: OnceLock<PrivacyEngine> = OnceLock::new();

/// Convenience free function: scan with the process-wide default engine.
/// Errors propagate unchanged — the refusal contract is identical.
/// (The gateway injects its own engine instance; this fn exists for callers
/// that just need the default behavior.)
pub fn scan(messages: &[String]) -> Result<PrivacyClass, PrivacyError> {
    // `get_or_init` runs the builder at most once process-wide; the static
    // pattern set is compile-verified by tests, so `expect` is sound.
    let engine = ENGINE
        .get_or_init(|| PrivacyEngine::new().expect("committed recognizer patterns must compile"));
    engine.scan(messages)
}

#[cfg(test)]
// cfg(test): compiled only under `cargo test`.
mod tests {
    use super::*; // engine internals for white-box tests

    #[test]
    fn evidence_mapping_matches_the_documented_table() {
        // Pure-function test of decide(): each row of the mapping table.
        let mut evidence = Evidence::default();
        assert_eq!(evidence.decide(), PrivacyClass::Safe);
        evidence.phone = true;
        assert_eq!(evidence.decide(), PrivacyClass::Sensitive);
        evidence = Evidence::default();
        evidence.bsn_valid = true;
        assert_eq!(evidence.decide(), PrivacyClass::Confidential);
        evidence = Evidence::default();
        evidence.iban_valid = true;
        assert_eq!(
            evidence.decide(),
            PrivacyClass::Sensitive,
            "bare IBAN is sensitive"
        );
        evidence.name_indicator = true;
        assert_eq!(
            evidence.decide(),
            PrivacyClass::Confidential,
            "IBAN + name combo"
        );
        evidence = Evidence::default();
        evidence.bsn_context = true;
        assert_eq!(
            evidence.decide(),
            PrivacyClass::Safe,
            "context word alone is not PII"
        );
        evidence.bsn_shape_failed = true;
        assert_eq!(
            evidence.decide(),
            PrivacyClass::Sensitive,
            "context + near-miss digits"
        );
    }

    #[test]
    fn evidence_from_any_message_counts_v1_bug_pinned() {
        // The v1 bug: only the LAST message was scanned. PII in message 0
        // with a clean last message must still classify.
        let engine = PrivacyEngine::new().expect("default patterns compile");
        let messages = vec![
            "Mijn BSN is 111222333.".to_string(),
            "en verder niets bijzonders".to_string(),
        ];
        let class = engine.scan(&messages).expect("clean-sized messages scan");
        assert_eq!(class, PrivacyClass::Confidential);
    }

    #[test]
    fn oversized_message_is_a_refusal_not_a_pass_through() {
        let engine = PrivacyEngine::with_limit(16).expect("default patterns compile");
        let messages = vec!["this message is much longer than sixteen bytes".to_string()];
        let error = engine.scan(&messages).expect_err("must refuse");
        assert_eq!(
            error,
            PrivacyError::TooLarge {
                index: 0,
                bytes: 46,
                limit: 16
            }
        );
        assert_eq!(error.class(), "privacy_scan_too_large");
    }
}
