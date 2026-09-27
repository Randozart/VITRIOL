//! `PrivacyError` — the refusal taxonomy (fail-closed by construction).
//!
//! Purpose: every way a scan can REFUSE rather than answer. The gateway
//! maps any `PrivacyError` to an HTTP 503 with nothing dispatched and no
//! upstream call — the v1 bug where a scanner failure passed the request
//! through silently (LESSONS.md "Wasstraat") is structurally impossible:
//! `scan()` has no path that both errors and yields a class.
//! C# equivalent: `class PrivacyError : Exception`; TS: a tagged union.

use thiserror::Error; // derive Display + std::error::Error from the attributes — TS: hand-writing .message plumbing

/// Every way the PII scan can fail. Deliberately tiny: the engine has no
/// I/O and no panic paths — only size limits and (at construction) regex
/// compilation can fail.
#[derive(Debug, Error, PartialEq, Eq)]
// derive: Debug = {:#?} logs; Error = `?`/Box<dyn Error> plumbing;
// PartialEq = assert_eq! on the error shape in tests.
pub enum PrivacyError {
    /// A regex failed to COMPILE — engine construction refused. Unreachable
    /// for the static pattern set (they are compile-time constants reviewed
    /// by tests), but the type keeps `PrivacyEngine::new` honest: a broken
    /// recognizer refuses startup, it never silently shrinks coverage.
    #[error("recognizer compilation failed: {0}")]
    Compile(String),

    /// A message exceeded the scan size limit — refused, nothing scanned,
    /// nothing dispatched. Unbounded input on the hot path is a
    /// denial-of-service on the scanner AND an unreviewable privacy
    /// surface; refusing is the fail-closed answer (LESSONS: v1 passed
    /// anything through when scanning was hard).
    #[error("message {index} of {bytes} bytes exceeds the {limit}-byte scan limit — refused")]
    TooLarge {
        /// 0-based index of the oversized message in the request.
        index: usize,
        /// The oversized message's size in bytes.
        bytes: usize,
        /// The configured limit that was exceeded.
        limit: usize,
    },
}

impl PrivacyError {
    /// Stable machine-readable id for the ledger `status` column and logs
    /// (same convention as `DispatchError::class`) — never the Display text.
    pub fn class(&self) -> &'static str {
        match self {
            PrivacyError::Compile(_) => "privacy_engine_broken",
            PrivacyError::TooLarge { .. } => "privacy_scan_too_large",
        }
    }
}
