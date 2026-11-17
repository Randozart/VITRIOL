// Display width, in terminal columns.
//
// Every layout decision in this TUI — truncation, padding, cell budgets, the
// composer's row count — must use CELLS, not `chars().count()`. A CJK glyph or
// an emoji occupies two cells while counting as one char, so character
// arithmetic silently overflows: borders misalign, text is pushed past the
// pane, and the model picker's columns drift (2026-11-17 render audit,
// finding WIDTH-1: eleven call sites shared this assumption).
//
// Braille, box drawing and the alchemical glyphs are all one cell wide, so the
// fire and the panel chrome are unaffected — but they go through here too, so
// the rule is uniform and a future glyph swap cannot reintroduce the bug.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Display width of `s` in terminal columns.
pub fn cols(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Width of one character in cells (0 for combining marks, 2 for wide).
pub fn char_cols(c: char) -> usize {
    UnicodeWidthChar::width(c).unwrap_or(0)
}

/// Truncate to at most `w` COLUMNS, appending `…` when anything was cut.
///
/// The ellipsis is part of the budget, and a double-width glyph that would
/// straddle the boundary is dropped rather than half-printed: exceeding `w` is
/// the one thing this function may never do, because callers use it to build
/// fixed-width cells and to keep a line inside a rectangle.
pub fn trunc_cols(s: &str, w: usize) -> String {
    if cols(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let budget = w.saturating_sub(1); // one column for the ellipsis
    let mut used = 0usize;
    let mut out = String::new();
    for c in s.chars() {
        let cw = char_cols(c);
        if used + cw > budget {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push('…');
    out
}

/// Pad with spaces to exactly `w` columns, truncating first when needed.
#[allow(dead_code)] // wired in by the composer/header stages (render audit S5/S6)
pub fn pad_cols(s: &str, w: usize) -> String {
    let mut out = trunc_cols(s, w);
    let width = cols(&out);
    if width < w {
        out.push_str(&" ".repeat(w - width));
    }
    out
}

/// Right-align in `w` columns, truncating from the left when needed.
#[allow(dead_code)] // wired in by the header stage (render audit S6)
pub fn pad_cols_right(s: &str, w: usize) -> String {
    let out = trunc_cols(s, w);
    let width = cols(&out);
    if width < w {
        format!("{}{}", " ".repeat(w - width), out)
    } else {
        out
    }
}

/// Visual rows `text` needs at `width` columns under HARD (character) wrapping:
/// (used by the composer sizing stage, render audit S5)
/// the number of rows a `Paragraph` with `Wrap { trim: false }` draws when the
/// text contains no spaces — a lower bound for word-wrapped prose.
#[allow(dead_code)] // wired in by the composer sizing stage (render audit S5)
pub fn hard_wrap_rows(text: &str, width: usize) -> u16 {
    let width = width.max(1);
    let mut rows = 1u16;
    let mut used = 0usize;
    for c in text.chars() {
        if c == '\n' {
            rows += 1;
            used = 0;
            continue;
        }
        let cw = char_cols(c);
        if used + cw > width {
            rows += 1;
            used = cw;
        } else {
            used += cw;
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // ASCII, braille, box drawing and the alchemical glyphs must stay 1 cell —
    // the whole panel chrome depends on it.
    #[test]
    fn single_cell_glyphs_are_one_column() {
        for g in ["a", "·", "─", "_", "⠁", "◈", "🜖", "…"] {
            assert_eq!(cols(g), 1, "{g:?} must be one column");
        }
    }

    // WIDTH-1: the bug was that these counted as their char count.
    #[test]
    fn wide_glyphs_count_two_columns() {
        assert_eq!(cols("漢"), 2);
        assert_eq!(cols("漢字"), 4);
        assert_eq!(cols("🜖漢"), 3);
        assert_eq!(cols("a漢b"), 4);
        // Combining marks add no width.
        assert_eq!(cols("e\u{0301}"), 1);
    }

    #[test]
    fn trunc_never_exceeds_the_budget() {
        let samples = [
            "",
            "abc",
            "漢字漢字",
            "🜖漢a漢",
            "a\u{0301}bc",
            &"x".repeat(100),
        ];
        for s in samples {
            for w in 0..12usize {
                let out = trunc_cols(s, w);
                assert!(
                    cols(&out) <= w,
                    "trunc_cols({s:?}, {w}) produced {out:?} of width {}",
                    cols(&out)
                );
            }
        }
    }

    #[test]
    fn trunc_marks_a_cut_and_leaves_a_fit_alone() {
        assert_eq!(trunc_cols("abc", 3), "abc");
        assert_eq!(trunc_cols("abc", 9), "abc");
        let cut = trunc_cols("abcdef", 3);
        assert!(cut.ends_with('…'), "{cut:?}");
        assert_eq!(cols(&cut), 3);
        // A double-width glyph that would straddle the boundary is dropped.
        assert_eq!(trunc_cols("漢漢漢", 4), "漢…");
    }

    #[test]
    fn pad_builds_exact_cells() {
        assert_eq!(pad_cols("ab", 5), "ab   ");
        assert_eq!(cols(&pad_cols("漢", 5)), 5);
        assert_eq!(cols(&pad_cols("漢字漢字", 5)), 5);
        assert_eq!(pad_cols_right("ab", 5), "   ab");
        assert_eq!(cols(&pad_cols_right("漢", 5)), 5);
    }

    #[test]
    fn hard_wrap_rows_matches_char_wrapping() {
        assert_eq!(hard_wrap_rows("", 10), 1);
        assert_eq!(hard_wrap_rows("abcdef", 10), 1);
        assert_eq!(hard_wrap_rows("abcdefghij", 5), 2);
        assert_eq!(hard_wrap_rows("a\nb", 10), 2);
        // Wide glyphs consume two cells: 5 CJK glyphs = 10 cells = 3 rows at 4.
        assert_eq!(hard_wrap_rows("漢漢漢漢漢", 4), 3);
    }
}
