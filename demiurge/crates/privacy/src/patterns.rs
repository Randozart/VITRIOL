//! Recognizer patterns + Dutch checksum validators — the engine's sensors.
//!
//! Purpose: every regex and every checksum in one module, each with the
//! rule it encodes and its failure mode. The Rust `regex` crate has NO
//! lookaround — patterns use `\b` word boundaries instead, which gives the
//! exact exclusion we need for free: digits embedded in alphanumeric tokens
//! (`NLABNA0417164300`, `ORD12345678`) sit between two word characters and
//! never match a `\b[0-9]+\b` run.
//! Checksums (the spike's precision lever, RESEARCH.md):
//! * BSN — the "eleven-test" (mod-11): digits × weights 9,8,7,6,5,4,3,2,−1
//!   must sum to a multiple of 11. A wrong digit breaks it; random 9-digit
//!   numbers pass with p = 1/11.
//! * NL IBAN — ISO 13616 mod-97: rearranged digits must leave remainder 1.
//!
//! Reading order: pattern constants → the validators `bsn_valid` and
//! `nl_iban_mod97` → `normalize_digits` (shared helper).

// ---------------------------------------------------------------------------
// Shape patterns. Each is compiled ONCE by `PrivacyEngine` (engine.rs);
// they live here as &'static str so the pattern, its name and its docs
// stay in one place.
// ---------------------------------------------------------------------------

/// Dutch mobile phone: `06…`, `+31 6 …`, `0031 6 …` + 8 digits, any
/// single-character space/dash separators. Word-bounded so digit runs
/// inside longer tokens never match.
pub const PHONE_MOBILE: &str = r"\b(?:\+31|0031|0)[ /-]?6(?:[ -]*[0-9]){8}\b";

/// Generic long number run (landlines, international numbers). Word-
/// bounded so digit runs embedded in alphanumeric tokens (`NLABNA…`,
/// `ORD…`) never match. Needs a normalized digit count of 9-12 (checked
/// in engine.rs) AND the message must not be date-shaped (VITRIOL's
/// exclusion, ported: the vault field test classified a filename date as
/// "sensitive").
pub const PHONE_GENERIC: &str = r"\b\+?[0-9][0-9 \-./()]{6,}[0-9]\b";

/// VITRIOL's date-shape guard, ported verbatim (yyyy-mm-dd / dd.mm.yyyy
/// shapes): a message matching this never trips PHONE_GENERIC.
pub const DATEISH: &str =
    r"\b[0-9]{4}[-/.][0-9]{1,2}[-/.][0-9]{1,2}\b|\b[0-9]{1,2}[-/.][0-9]{1,2}[-/.][0-9]{2,4}\b";

/// Dutch postcode, canonical uppercase form `1234 AB` (PLAN §7 P3 spec:
/// `^[1-9][0-9]{3} ?[A-Z]{2}$`). Leading zero is invalid by law, so
/// `0123 AB` is a negative probe, not a miss.
pub const POSTCODE: &str = r"\b[1-9][0-9]{3} ?[A-Z]{2}\b";

/// Email address (VITRIOL's pattern, ported).
pub const EMAIL: &str = r"\b[\w.+-]+@[\w-]+\.[\w.]{2,}\b";

/// Dutch IBAN shape: `NL` + 2 check digits + 4 bank letters + 10 account
/// digits, optional single spaces between groups (SEPA human format).
/// Case-insensitive: real-world IBANs arrive lowercase as often as not.
/// A match is only EVIDENCE after `nl_iban_mod97` validates it.
pub const IBAN_NL: &str = r"(?i)\bNL[0-9]{2} ?[A-Z]{4} ?[0-9]{4} ?[0-9]{4} ?[0-9]{2}\b";

/// BSN-shaped 9-digit run (word-bounded). EVIDENCE only after `bsn_valid`
/// passes; failing the checksum downgrades to the context rules.
pub const BSN_SHAPE: &str = r"\b[0-9]{9}\b";

/// BSN/BSN-context words ("burgerservicenummer", "bsn"). With any digit
/// run nearby → at least `sensitive` (fail toward privacy: a typo'd BSN
/// is still a BSN attempt).
pub const BSN_CONTEXT: &str = r"(?i)\b(burgerservicenummer|bsn)\b";

/// Bank-account context words. With an IBAN-shaped match that FAILED
/// mod-97 → `sensitive` (fail toward privacy: bank details with a typo).
pub const IBAN_CONTEXT: &str =
    r"(?i)\b(iban|bankrekeningnummer|bankrekening|rekeningnummer|rekening)\b";

/// Name indicators that turn a bare IBAN into an IBAN+person combo
/// ("ten name van …", "t.n.v. …", "t.a.v. …", "Naam: …", rekeninghouder).
/// Combo (IBAN + this, anywhere in the conversation) → `confidential`:
/// a name glued to an account number identifies a natural person.
pub const NAME_INDICATOR: &str =
    r"(?i)\b(ten name van|t\.?n\.?v\.?|t\.?a\.?v\.?|rekeninghouder|naam ?:)";

/// Medical-indicator terms (the "if cheap" list from PLAN §2). A bare
/// Dutch "recept" is deliberately ABSENT — it means recipe as often as
/// prescription; only "herhaalrecept" (repeat prescription) is unambiguous.
pub const MEDICAL_TERMS: &str = r"(?i)\b(diagnose|diagnoses|patiënt|patiënten|medisch|medische|behandeling|behandelingen|herhaalrecept|medicijn|medicijnen|medicatie|geneesmiddel|geneesmiddelen|zorgdossier|huisarts|ziekenhuis|psychiater|therapie)\b";

/// Legal-indicator terms (same "if cheap" rationale: a court/case context
/// is exactly what no cloud provider needs to see under any guarantee).
pub const LEGAL_TERMS: &str = r"(?i)\b(rechtbank|strafzaak|strafblad|advocaat|proces-verbaal|officier van justitie|vonnis|justitiedossier|clientdossier)\b";

/// Secret/credential paths — VITRIOL's SECRET_PATH_RE ported to no-lookaround
/// regex (original VITRIOL work; the pattern flags .env/.ssh/key-file paths
/// and the words secrets/credentials in path position).
pub const SECRET_PATH: &str = r#"(?i)(?:^|[\s"'(=:])(?:[\w./~-]*)(?:\.env|\.ssh|\.aws|secrets?|credentials?|\.pem|\.p12|\.p8|\.key|keystore)(?:[\w./~-]*)"#;

/// API-key token shapes — VITRIOL's API_KEY_RE ported (Google, OpenAI-
/// style `sk-…`, GitHub `ghp_`/fine-grained, Slack). A leaked credential
/// is leaked regardless of the provider's retention policy → confidential.
pub const API_KEY: &str =
    r"\b(AIzaSy[\w-]{20,}|sk-[\w-]{20,}|ghp_[\w]{20,}|github_pat_[\w]{20,}|xox[bp]-[\w-]{10,})\b";

// ---------------------------------------------------------------------------
// Checksum validators — the precision lever. A shape match without its
// checksum is only "uncertain", and uncertainty fails toward privacy
// (engine.rs); a checksum verdict is a FACT.
// ---------------------------------------------------------------------------

/// BSN "eleven-test" (mod-11): `9·d1 + 8·d2 + … + 2·d8 − 1·d9 ≡ 0 (mod 11)`.
/// Exactly 9 digits (the official form; an "8-digit BSN" is a 9-digit one
/// whose leading zero was dropped, and re-padding cannot be verified — so
/// it stays undetected by checksum and falls to the context rules).
/// O(digits). TS equivalent:
/// `digits.reduce((s, d, i) => s + (9 - i) * d, 0) % 11 === 0`.
pub fn bsn_valid(digits: &str) -> bool {
    let digits = normalize_digits(digits);
    // Official form: 9 digits, not all zero (0…0 passes mod-11 but denotes
    // no BSN the numeral system can issue).
    if digits.len() != 9 || digits.chars().all(|d| d == '0') {
        return false;
    }
    // Weights for positions 1-8 (the 9th digit is subtracted).
    let weights = [9i64, 8, 7, 6, 5, 4, 3, 2];
    let mut sum: i64 = 0;
    for (position, character) in digits.chars().enumerate() {
        // `to_digit(10)` → Option: None for a non-digit (guard above makes
        // this unreachable, so unwrap is justified — TS: parseInt).
        let value = character.to_digit(10).expect("digits-only by normalize") as i64;
        if position < 8 {
            sum += weights[position] * value;
        } else {
            sum -= value; // the −1 weight of the control digit
        }
    }
    sum % 11 == 0
}

/// ISO 13616 mod-97 check for a NL IBAN: strip spaces, move the first 4
/// characters to the end, map letters to numbers (A=10 … Z=35), then the
/// whole number must leave remainder 1 when divided by 97. O(digits).
/// (Same arithmetic as presidio-analyzer's validator — the math is the ISO
/// standard, written fresh here per PLAN §0.)
pub fn nl_iban_mod97(iban: &str) -> bool {
    // Keep ONLY alphanumeric characters (letters survive — bank codes are
    // part of the checked number; spaces/dashes do not). TS: `s.replace(/[^A-Z0-9]/g, "")`.
    let cleaned: String = iban
        .to_uppercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if cleaned.len() != 18 || !cleaned.starts_with("NL") {
        return false;
    }
    // Rearrange: check digits + country stay behind the account portion.
    let (head, tail) = cleaned.split_at(4);
    let rearranged = format!("{tail}{head}");
    // Big-number-mod-97 without bignum: fold one character at a time.
    // `try_fold` short-circuits on the first illegal character (`?`-style:
    // the helper returns Option and None means "not an IBAN character").
    let remainder = rearranged.chars().try_fold(0u32, |remainder, character| {
        mod97_step(remainder, character)
    });
    remainder == Some(1)
}

/// One mod-97 folding step: consume `character` into the running
/// remainder — digits contribute one decimal place, letters their A=10…
/// Z=35 two-digit value; anything else is `None` (fail the check).
fn mod97_step(remainder: u32, character: char) -> Option<u32> {
    if let Some(digit) = character.to_digit(10) {
        Some((remainder * 10 + digit) % 97)
    } else if character.is_ascii_uppercase() {
        let value = character as u32 - 'A' as u32 + 10;
        Some((remainder * 100 + value) % 97)
    } else {
        None // letters outside A-Z cannot appear in an IBAN
    }
}

/// Keep only ASCII digits — the shared pre-checksum normalizer.
/// TS: `s.replace(/\D/g, "")`.
pub fn normalize_digits(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

#[cfg(test)]
// cfg(test): compiled only under `cargo test`.
mod tests {
    use super::*; // the validators under test

    #[test]
    fn bsn_checksum_accepts_known_valid_numbers() {
        // 111222333 and 123456782 are the canonical published test BSNs;
        // 100000009 is generated (weights sum to 0).
        assert!(bsn_valid("111222333"));
        assert!(bsn_valid("123456782"));
        assert!(bsn_valid("100000009"));
    }

    #[test]
    fn bsn_checksum_rejects_corrupted_and_random_numbers() {
        // Transposed/typo'd digits break mod-11; random 9-digit probes fail.
        assert!(!bsn_valid("123456789"));
        assert!(!bsn_valid("999999999"));
        assert!(!bsn_valid("111111112"));
        assert!(!bsn_valid("876543210"));
        assert!(!bsn_valid("1234")); // too short
        assert!(!bsn_valid("000000009")); // weights sum to −9, and all-zero is refused
    }

    #[test]
    fn iban_mod97_accepts_published_nl_examples() {
        // NL91 ABNA 0417 1643 00 is the SEPA documentation standard example;
        // the others are generated to satisfy mod-97 (see the spike).
        assert!(nl_iban_mod97("NL91ABNA0417164300"));
        assert!(nl_iban_mod97("NL88INGB0004712345"));
        assert!(nl_iban_mod97("NL73 RABO 1234 5678 90")); // spaced human form
    }

    #[test]
    fn iban_mod97_rejects_corrupted_and_malformed() {
        assert!(!nl_iban_mod97("NL91ABNA0417164301")); // last digit off by one
        assert!(!nl_iban_mod97("NLABNA0417164300")); // missing check digits
        assert!(!nl_iban_mod97("DE89370400440532013000")); // not NL (17 digits de)
    }

    #[test]
    fn normalize_digits_strips_everything_else() {
        assert_eq!(normalize_digits("NL88 INGB 0004 7123 45"), "880004712345");
        assert_eq!(normalize_digits("06 - 12 34 56 78"), "0612345678");
    }
}
