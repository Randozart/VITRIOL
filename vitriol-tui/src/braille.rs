//! Braille-dot gauge glyphs.
//!
//! A gauge is a horizontal row of braille cells (Unicode block U+2800..U+28FF).
//! Each cell holds six dots (2 cols × 3 rows); each dot is one "percentage
//! point", so a cell spans six points. A cell fills from its bottom-left dot
//! upward, and cells fill left to right as the ratio grows. The caller colors
//! each cell from its position fraction `t`; see `theme::BrailleRamp`.
//!
//! Dot-to-bit map (6-dot braille):
//! - dot1 = bit0 (top-left), dot2 = bit1 (mid-left), dot3 = bit2 (bottom-left)
//! - dot4 = bit3 (top-right), dot5 = bit4 (mid-right), dot6 = bit5 (bottom-right)

/// Fill order of the six dots within one cell, bottom-left first, rising.
/// Bit values: dot3 `0x04` → dot6 `0x20` → dot2 `0x02` → dot5 `0x10` →
/// dot1 `0x01` → dot4 `0x08`.
pub const FILL_ORDER: [u8; 6] = [0x04, 0x20, 0x02, 0x10, 0x01, 0x08];

/// One rendered braille cell.
#[derive(Debug, Clone, Copy)]
pub struct BarCell {
    /// The braille glyph for this cell.
    pub ch: char,
    /// Position fraction in `[0, 1]` across the bar (left = 0, right = 1).
    pub t: f64,
    /// True when at least one dot is lit (colorable); empty cells render blank.
    pub lit: bool,
}

/// The 6-dot mask for a cell with `lit` dots lit, in `FILL_ORDER`.
pub fn cell_mask(lit: usize) -> u8 {
    FILL_ORDER
        .iter()
        .take(lit.min(FILL_ORDER.len()))
        .fold(0u8, |acc, b| acc | b)
}

/// The braille glyph (U+2800 + mask) for the given dot mask.
pub fn glyph(mask: u8) -> char {
    char::from_u32(0x2800 + mask as u32).unwrap_or('\u{2800}')
}

/// Build a bar of `cells` braille cells for the given ratio in `[0, 1]`.
/// `filled` dots = `round(ratio × cells × 6)`, clamped; cell `i` gets the next
/// `clamp(filled − i×6, 0, 6)` dots, left to right.
pub fn bar(ratio: f64, cells: usize) -> Vec<BarCell> {
    let total = cells * FILL_ORDER.len();
    let filled = (ratio.clamp(0.0, 1.0) * total as f64).round() as usize;
    (0..cells)
        .map(|i| {
            let lit = filled
                .saturating_sub(i * FILL_ORDER.len())
                .min(FILL_ORDER.len());
            let t = if cells > 1 {
                i as f64 / (cells - 1) as f64
            } else {
                0.0
            };
            BarCell {
                ch: glyph(cell_mask(lit)),
                t,
                lit: lit > 0,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_order_is_bottom_left_first() {
        assert_eq!(FILL_ORDER, [0x04, 0x20, 0x02, 0x10, 0x01, 0x08]);
    }

    #[test]
    fn cell_mask_lit_counts() {
        assert_eq!(cell_mask(0), 0x00);
        assert_eq!(cell_mask(1), 0x04);
        assert_eq!(cell_mask(2), 0x24);
        assert_eq!(cell_mask(3), 0x26);
        assert_eq!(cell_mask(4), 0x36);
        assert_eq!(cell_mask(5), 0x37);
        assert_eq!(cell_mask(6), 0x3F);
        assert_eq!(cell_mask(9), 0x3F);
    }

    #[test]
    fn glyph_uses_braille_block() {
        assert_eq!(glyph(0x00), '\u{2800}');
        assert_eq!(glyph(0x04), '\u{2804}');
        assert_eq!(glyph(0x3F), '\u{283F}');
    }

    #[test]
    fn empty_bar_renders_blank_cells() {
        let b = bar(0.0, 4);
        assert_eq!(b.len(), 4);
        assert!(b.iter().all(|c| !c.lit && c.ch == '\u{2800}'));
    }

    #[test]
    fn full_bar_lits_all_dots() {
        let b = bar(1.0, 3);
        assert!(b.iter().all(|c| c.lit && c.ch == '\u{283F}'));
    }

    #[test]
    fn partial_bar_fills_left_cells_first() {
        let b = bar(0.5, 3);
        assert_eq!(b[0].ch, '\u{283F}');
        assert_eq!(b[1].ch, '\u{2826}');
        assert_eq!(b[2].ch, '\u{2800}');
        assert!(!b[2].lit);
    }

    #[test]
    fn cell_positions_t_span_bar() {
        let b = bar(1.0, 3);
        assert_eq!(b[0].t, 0.0);
        assert_eq!(b[2].t, 1.0);
    }

    #[test]
    fn single_cell_bar_uses_t_zero() {
        let b = bar(0.5, 1);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].t, 0.0);
    }
}
