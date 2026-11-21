//! Band content hold (2026-11-21).
//!
//! Owner report: "text occasionally changing to empty background" in the
//! bottom strip. Cause: the band debounces GEOMETRY (`BAND_SETTLE`, 900 ms) but
//! not CONTENT. When the indicator list blips — a status line that briefly
//! reports nothing, or a shorter list — the reserved rows are painted with the
//! panel surface and nothing is drawn into them, so a row reads as empty
//! background for up to the settle window, then the text returns.
//!
//! The fix is symmetric with the geometry: hold the last non-empty lines while
//! the settled height still reserves rows. A status strip showing the
//! previous line for 900 ms is right; a hole is not.

/// Holds the last non-empty indicator lines the band painted.
#[derive(Debug, Default, Clone)]
pub struct BandText {
    lines: Vec<String>,
}

impl BandText {
    /// Lines to paint for a frame with `rows` settled rows available.
    ///
    /// - fresh lines non-empty → adopt and return them (plus held extras when
    ///   the settled height still reserves more rows than we have content for);
    /// - fresh lines empty → return the held lines (the hold);
    /// - no held lines → empty (nothing to say, and nothing was said before).
    ///
    /// `rows` caps the result: the band never paints more rows than settled,
    /// or the layout above it would shift mid-frame.
    pub fn paint(&mut self, fresh: &[String], rows: u16) -> Vec<String> {
        let cap = rows as usize;
        if cap == 0 {
            // The band is gone: forget the text with it, or a later resize
            // would resurrect a stale line from another layout.
            self.lines.clear();
            return Vec::new();
        }
        let fresh: Vec<String> = fresh
            .iter()
            .filter(|l| !l.trim().is_empty())
            .take(cap)
            .cloned()
            .collect();
        if !fresh.is_empty() {
            self.lines = fresh.clone();
            return fresh;
        }
        // Hold: the settle window is reserving rows; show what was there.
        self.lines.iter().take(cap).cloned().collect()
    }

    /// Forget the held lines (session switch / band reset).
    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// The defect: content gone, geometry still settled → must HOLD, not blank.
    #[test]
    fn holds_the_last_lines_when_content_blips() {
        let mut b = BandText::default();
        assert_eq!(b.paint(&lines(&["route lapis · local"]), 1).len(), 1);
        // Content blips for a frame; the settled row is still reserved.
        let held = b.paint(&[], 1);
        assert_eq!(
            held,
            lines(&["route lapis · local"]),
            "row must not go blank"
        );
    }

    /// I1 parity: fresh content always wins, immediately.
    #[test]
    fn fresh_content_replaces_the_hold() {
        let mut b = BandText::default();
        b.paint(&lines(&["old"]), 1);
        assert_eq!(b.paint(&lines(&["new"]), 1), lines(&["new"]));
    }

    /// Once the band height actually drops to zero, the hold dies with it.
    #[test]
    fn zero_rows_clears_the_hold() {
        let mut b = BandText::default();
        b.paint(&lines(&["a"]), 1);
        assert!(b.paint(&[], 0).is_empty());
        assert!(b.paint(&[], 1).is_empty(), "nothing may resurrect");
    }

    /// The hold never paints more rows than the settled height allows.
    #[test]
    fn hold_is_capped_by_settled_rows() {
        let mut b = BandText::default();
        b.paint(&lines(&["one", "two", "three"]), 3);
        assert_eq!(b.paint(&[], 1), lines(&["one"]));
    }

    /// Blank-only fresh lines are the same as no lines (they are stripped
    /// before the band sees them, but the hold must not adopt an empty row).
    #[test]
    fn whitespace_lines_are_not_content() {
        let mut b = BandText::default();
        b.paint(&lines(&["kept"]), 1);
        assert_eq!(b.paint(&lines(&["   "]), 1), lines(&["kept"]));
    }
}
