// PROVENANCE: harvested 2026-09-27 from knooppunt
// (Desktop/Projects/knooppunt, MIT OR Apache-2.0, same author) per the
// Demiurge plan (.opencode/plans/demiurge-2026-09-27.md, M2). Copied
// verbatim except this header; the Dutch PII test set ships with it.
// VITRIOL use: fail-closed privacy gate for lane overflow - sensitive and
// confidential classes never leave the local lane.

//! Privacy crate — the PII engine behind the routing thesis (PLAN §2.1).
//!
//! Purpose: classify each chat request into one of three privacy classes —
//! [`PrivacyClass::Safe`], [`PrivacyClass::Sensitive`],
//! [`PrivacyClass::Confidential`] — by scanning **ALL** messages (the v1
//! engine scanned only the last message: LESSONS.md §"Wasstraat"; earlier
//! BSNs/IBANs went raw to the cloud). The class is the hard constraint the
//! policy crate applies before scoring: `confidential` → local lane only,
//! `sensitive` → EU+ZDR endpoints only, `safe` → full pool.
//! Fail-closed rules (LESSONS: v1's scan failure passed through silently):
//! * a scan ERROR is a refusal ([`PrivacyError`]), never a pass-through;
//! * uncertain detection (context words with an un-checksumable digit run)
//!   fails TOWARD privacy (the more protective class), never away.
//!
//! Origin: the P3 spike (RESEARCH.md `## PII engine spike (P3)`) tested
//! `redact` (disqualified: it is a `Secret<T>` log-hygiene wrapper with no
//! detectors) and `presidio-analyzer` 0.1.11 (real but: 63% class recall on
//! the Dutch set, ~25 ms/message, zero postcode/medical/legal/secret
//! coverage) — so the engine is ours: regex + Dutch checksum validators.
//! Reading order: `class.rs` → `error.rs` → `patterns.rs` → `engine.rs`.

pub mod class; // PrivacyClass: the three-lane type + ordering helpers
pub mod engine; // PrivacyEngine: the scan + evidence → class decision
pub mod error; // PrivacyError: refusal taxonomy (fail-closed)
pub mod patterns; // recognizer regexes + BSN/IBAN checksum validators

pub use class::PrivacyClass; // the type the policy crate gates on
pub use engine::{scan, PrivacyEngine}; // the process-wide helper + the compiled-once scanner
pub use error::PrivacyError; // every scan failure the gateway must refuse
