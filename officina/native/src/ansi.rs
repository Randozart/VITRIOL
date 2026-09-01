//! ANSI-aware string helpers — Rust port of the OfficinaSplit JS helpers in
//! runtime/build-patch.mjs (officinaStripZeroWidth / officinaVisibleWidth /
//! officinaCut). Byte-scanning replaces per-line JS regex allocation; these
//! run per rendered line at 60+ fps during generation.
//!
//! Provenance: original work, this repo (Apache-2.0 OR MIT) — port of our own
//! TypeScript in the same workspace.

/// Length of the escape sequence starting at `chars[i]`, if any.
/// Recognizes the same three classes as the JS regexes:
///   CSI:  ESC [ <0-9;?>* <letter>
///   OSC:  ESC ] <not BEL/ESC>* (BEL | ESC \)
///   APC:  ESC _ <not BEL/ESC>* (BEL | ESC \)
/// A partial/invalid sequence matches nothing (the ESC is then a plain char),
/// exactly like the JS regex behavior.
fn escape_len(chars: &[char], i: usize) -> Option<usize> {
    if chars[i] != '\x1b' {
        return None;
    }
    let n = chars.len();
    let next = *chars.get(i + 1)?;
    match next {
        '[' => {
            let mut j = i + 2;
            while j < n && (chars[j].is_ascii_digit() || chars[j] == ';' || chars[j] == '?') {
                j += 1;
            }
            if j < n && chars[j].is_ascii_alphabetic() {
                Some(j - i + 1)
            } else {
                None
            }
        }
        ']' | '_' => {
            let mut j = i + 2;
            while j < n {
                match chars[j] {
                    '\x07' => return Some(j - i + 1),
                    '\x1b' => {
                        if j + 1 < n && chars[j + 1] == '\\' {
                            return Some(j - i + 2);
                        }
                        return None; // regex requires ESC \\ ; otherwise no match
                    }
                    _ => j += 1,
                }
            }
            None
        }
        _ => None,
    }
}

/// Wide-char classification — identical ranges to the JS port (CJK, Hangul,
/// fullwidth forms). Deliberately conservative: matches build-patch.mjs so
/// native and JS widths agree cell-for-cell.
fn char_width(cp: u32) -> u32 {
    if (0x1100..=0x115f).contains(&cp)
        || (0x2e80..=0xa4cf).contains(&cp)
        || (0xac00..=0xd7a3).contains(&cp)
        || (0xff00..=0xff60).contains(&cp)
    {
        2
    } else {
        1
    }
}

/// Remove CSI/OSC/APC sequences (the JS `officinaStripZeroWidth`).
pub fn strip_ansi(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        match escape_len(&chars, i) {
            Some(len) => i += len,
            None => {
                out.push(chars[i]);
                i += 1;
            }
        }
    }
    out
}

/// Visible terminal width of `line`, escape-sequence-aware.
pub fn visible_width(line: &str) -> u32 {
    let stripped = strip_ansi(line);
    stripped.chars().map(|c| char_width(c as u32)).sum()
}

/// Truncate `line` to `width` visible cells, preserving escape sequences
/// (including trailing color resets past the cut point) — the JS `officinaCut`.
pub fn cut(line: &str, width: u32) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len());
    let mut vis: u32 = 0;
    let mut i = 0;
    while i < chars.len() {
        if let Some(len) = escape_len(&chars, i) {
            for k in 0..len {
                out.push(chars[i + k]);
            }
            i += len;
            continue;
        }
        if vis >= width {
            i += 1;
            continue;
        }
        let cw = char_width(chars[i] as u32);
        if vis + cw > width {
            i += 1;
            continue;
        }
        out.push(chars[i]);
        vis += cw;
        i += 1;
    }
    out
}

/// Whole OfficinaSplit row-merge loop in one call — the actual per-render hot
/// path (per row: cut, width, pad main; cut, pad sidebar, bg wrap, concat).
/// Semantics mirror OfficinaSplit.render in runtime/build-patch.mjs exactly,
/// including the P4 BOTTOM-ANCHOR: `sb_pad` rows of empty sidebar precede
/// `sb_lines` so the panel pins to the bottom of the emitted block.
pub fn merge_split_rows(
    main_lines: &[String],
    main_w: u32,
    sb_lines: &[String],
    sb_w: u32,
    sb_pad: u32,
    gap: u32,
    bg: &str,
    reset: &str,
) -> Vec<String> {
    let total = main_lines.len().max(sb_lines.len() + sb_pad as usize);
    let mut out = Vec::with_capacity(total);
    for r in 0..total {
        let mut left = main_lines.get(r).map(|s| s.as_str()).unwrap_or("").to_string();
        let lw = visible_width(&left);
        if lw > main_w {
            left = cut(&left, main_w);
        }
        let mut line = String::with_capacity(main_w as usize + sb_w as usize + 32);
        line.push_str(&left);
        for _ in 0..main_w.saturating_sub(visible_width(&left)) {
            line.push(' ');
        }
        if sb_w > 0 {
            let right = match usize::try_from(sb_pad) {
                Ok(pad) if r >= pad => sb_lines.get(r - pad).map(|s| s.as_str()).unwrap_or(""),
                _ => "",
            }
            .to_string();
            let rw = visible_width(&right);
            let right = if rw > sb_w { cut(&right, sb_w) } else { right };
            for _ in 0..gap {
                line.push(' ');
            }
            line.push_str(bg);
            line.push_str(&right);
            for _ in 0..sb_w.saturating_sub(visible_width(&right)) {
                line.push(' ');
            }
            line.push_str(reset);
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_sgr_osc_apc() {
        assert_eq!(strip_ansi("\x1b[38;2;255;95;31mhi\x1b[0m"), "hi");
        assert_eq!(strip_ansi("\x1b]0;title\x07hi"), "hi");
        assert_eq!(strip_ansi("\x1b_APC body\x1b\\hi"), "hi");
        assert_eq!(strip_ansi("plain"), "plain");
        // lone ESC followed by non-sequence char stays
        assert_eq!(strip_ansi("\x1bZhi"), "\x1bZhi");
    }

    #[test]
    fn widths_match_js_semantics() {
        assert_eq!(visible_width(""), 0);
        assert_eq!(visible_width("\x1b[1mbold\x1b[0m"), 4);
        assert_eq!(visible_width("ab"), 2);
        // Hangul syllable (AC00-D7A3) is wide
        assert_eq!(visible_width("한"), 2);
        // fullwidth form (FF00-FF60)
        assert_eq!(visible_width("！"), 2);
    }

    #[test]
    fn cut_preserves_escapes_and_resets() {
        let line = "\x1b[31mabcdef\x1b[0m";
        assert_eq!(cut(line, 3), "\x1b[31mabc\x1b[0m");
        assert_eq!(cut("plain", 10), "plain");
        assert_eq!(cut("plain", 0), "");
        // wide char that would overflow is skipped, later reset still kept
        let w = "\x1b[32m한\x1b[0m";
        assert_eq!(cut(w, 1), "\x1b[32m\x1b[0m");
    }

    #[test]
    fn cut_matches_js_wide_overflow() {
        // vis=1, wide char would make 3 > 2 → skipped (JS: i++ continue)
        assert_eq!(cut("a한b", 2), "ab");
    }

    #[test]
    fn merge_split_rows_matches_split_render() {
        let main = vec!["hello".to_string(), "long line that overflows the column".to_string()];
        let sb = vec!["◈ coupling".to_string(), "ctx gauge".to_string()];
        let out = merge_split_rows(&main, 12, &sb, 20, 0, 1, "\x1b[48;2;22;27;34m", "\x1b[0m");
        assert_eq!(out.len(), 2);
        // row 0: "hello" + 7 pad + 1 gap + bg + "◈ coupling" + 10 pad + reset
        assert!(out[0].starts_with("hello        \x1b[48;2;22;27;34m◈ coupling"));
        assert!(out[0].ends_with("\x1b[0m"));
        assert_eq!(visible_width(&out[0]), 12 + 1 + 20);
        // row 1 main is cut to 12 visible cells
        let cut_main = cut(&main[1], 12);
        assert!(out[1].starts_with(&cut_main));
        // sidebar shorter than main → last row keeps main content, empty right
        let main_only = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let sb_short = vec!["x".to_string()];
        let out2 = merge_split_rows(&main_only, 4, &sb_short, 4, 0, 1, "\x1b[48m", "\x1b[0m");
        assert_eq!(out2.len(), 3);
        assert!(out2[2].starts_with("c   "));
        assert_eq!(visible_width(&out2[2]), 4 + 1 + 4);
    }

    #[test]
    fn merge_split_rows_bottom_anchors_via_sb_pad() {
        // P4: 1 sidebar row, 3 main rows, pad=2 → sidebar content on LAST row
        let main = vec!["r1".to_string(), "r2".to_string(), "r3".to_string()];
        let sb = vec!["SB".to_string()];
        let out = merge_split_rows(&main, 6, &sb, 4, 2, 1, "\x1b[48m", "\x1b[0m");
        assert_eq!(out.len(), 3);
        assert!(out[0].starts_with("r1     \x1b[48m    "));
        assert!(out[2].starts_with("r3     \x1b[48mSB  "));
    }

    #[test]
    fn merge_split_rows_empty_sidebar() {
        let out = merge_split_rows(&["only".to_string()], 10, &[], 0, 0, 1, "", "");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], "only      ");
    }
}
