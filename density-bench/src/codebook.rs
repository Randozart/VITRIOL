// Codebook: the compressed notation an internal lane may use, and the decoder.
//
// E2's hypothesis is that internal lanes can spend fewer output tokens by writing
// in a fixed shorthand. That is only a *saving* if the shorthand can be read back:
// a compression whose expansion is guesswork is corruption with better marketing.
//
// So the codec is deliberately tiny, explicit, and round-trippable:
//
//   LEGEND (`=>` means)  ⇒  because   ¬  not      ∴  therefore
//                        +  add       -  remove   →  leads to
//
// `expand()` rewrites every legend glyph to its word. `suspicious()` reports
// glyphs the model invented, which no legend can decode — the I4 guard.

/// The legend shown to the model, one line, cheap (it is prompt tokens too).
pub const LEGEND: &str = "Use this shorthand: ⇒ because, ¬ not, ∴ therefore, + add, \
                          - remove, → leads to. No other symbols.";

/// Symbol -> words. Kept as pairs so `expand` and the legend cannot drift.
pub const MAPPING: [(char, &str); 6] = [
    ('⇒', "because"),
    ('¬', "not"),
    ('∴', "therefore"),
    ('→', "leads to"),
    ('+', "add"),
    ('-', "remove"),
];

/// Expand the legend glyphs back to words.
///
/// `+`/`-` are only expanded when they stand alone, so arithmetic and hyphenated
/// words survive; the rest are unambiguous.
pub fn expand(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '⇒' | '¬' | '∴' | '→' => {
                let word = MAPPING
                    .iter()
                    .find(|(sym, _)| sym == c)
                    .map(|(_, w)| *w)
                    .unwrap_or_default();
                push_word(&mut out, word);
            }
            '+' | '-' => {
                let standalone = chars
                    .get(i.wrapping_sub(1))
                    .map(|p| p.is_whitespace())
                    .unwrap_or(true)
                    && chars
                        .get(i + 1)
                        .map(|n| n.is_whitespace() || *n == '\n')
                        .unwrap_or(true);
                if standalone {
                    let word = if *c == '+' { "add" } else { "remove" };
                    push_word(&mut out, word);
                } else {
                    out.push(*c);
                }
            }
            // Collapse whitespace runs the expansion may have doubled.
            ' ' | '\n' | '\t' if out.ends_with(' ') => {}
            other => out.push(*other),
        }
    }
    out
}

/// Append a word, keeping single spacing.
fn push_word(out: &mut String, word: &str) {
    if !out.ends_with(' ') && !out.is_empty() {
        out.push(' ');
    }
    out.push_str(word);
    out.push(' ');
}

/// Symbols the model used that the legend does not define.
///
/// This is the guard: invented notation cannot be decoded, so a "saving" that
/// relies on it is rejected rather than rewarded.
pub fn suspicious(text: &str) -> String {
    let mut found: Vec<char> = Vec::new();
    for c in text.chars() {
        // `·` is a benign list separator: the legend itself used to use it, and a
        // model copying that habit is not inventing notation.
        let known = MAPPING.iter().any(|(sym, _)| *sym == c) || c == '·';
        if known || c.is_ascii() || c.is_whitespace() {
            continue;
        }
        // Non-ASCII punctuation/symbols only: ordinary CJK or accented prose is
        // not shorthand and must not be flagged.
        if !c.is_alphanumeric() && !found.contains(&c) {
            found.push(c);
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i4_every_legend_glyph_expands_to_its_word() {
        let expanded = expand("A ⇒ B ¬ C ∴ D → E");
        assert!(expanded.contains("because"), "{expanded}");
        assert!(expanded.contains("not"), "{expanded}");
        assert!(expanded.contains("therefore"), "{expanded}");
        assert!(expanded.contains("leads to"), "{expanded}");
        // No glyph survives expansion.
        assert!(!expanded.contains('⇒') && !expanded.contains('∴'));
    }

    #[test]
    fn i4_standalone_plus_and_minus_expand_but_arithmetic_survives() {
        assert!(expand("+ new weights - old ones").contains("add new weights"));
        // A hyphen inside a word and a numeric sign are not shorthand.
        assert_eq!(expand("cache-ram = -1"), "cache-ram = -1");
    }

    #[test]
    fn i4_invented_notation_is_flagged_and_plain_prose_is_not() {
        // The failure mode that makes compression unsafe: the model invents its
        // own symbols. They cannot be decoded, so the arm must fail I4.
        assert_eq!(suspicious("A ✗ B ※ C"), "✗※");
        assert_eq!(suspicious("plain english, commas; and (parens)."), "");
        // Legend glyphs and list separators are known, not suspicious.
        assert_eq!(suspicious("A ⇒ B"), "");
        assert_eq!(suspicious("step one · step two"), "");
        // Accented and CJK prose is not shorthand.
        assert_eq!(suspicious("déjà vu 中文"), "");
    }

    #[test]
    fn expansion_is_stable_for_both_notation_and_prose() {
        // Idempotence matters because the harness expands before checking guards.
        let once = expand("A ⇒ B");
        assert_eq!(expand(&once), once);
    }
}
