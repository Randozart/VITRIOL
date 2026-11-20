//! Base-background widgets: the kitty background_opacity countermeasure.
//!
//! kitty's `background_opacity` (the owner runs 0.5 + blur) alpha-blends ONLY
//! cells whose background is the terminal default; any explicit bg renders
//! opaque. Every cell this TUI emits therefore has to carry `BASE_BG()` —
//! derived one bit off `bg` (plan 2026-11-20-tui-opaque-base-bg) — instead of
//! the default, or the frosted desktop bleeds through the gaps between
//! widgets. Two widgets cover the two ways a cell can end up default:
//!
//! - [`BaseFill`] paints `BASE_BG()` onto every cell of an area **without**
//!   touching symbols or fg — rendered first in `render()`, it underlays the
//!   whole frame so unpainted regions are covered too.
//! - [`ClearBase`] replaces ratatui's `Clear` at popup sites: `Clear` calls
//!   `cell.reset()` which blanks a cell back to `Color::Reset` (the default
//!   bg) — exactly the transparency hole. `ClearBase` performs the same blank
//!   but lands on `BASE_BG()`, preserving the blank-then-fill contract every
//!   call site documents.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

use crate::theme;

/// Paint `BASE_BG()` under an entire area, preserving content.
///
/// Only the `bg` field of each cell is patched; symbol, fg, modifier and
/// underline stay untouched, so this is safe to run over a buffer that
/// already holds content (and over the fresh frame, where every cell starts
/// as `Reset`).
#[derive(Debug, Clone, Copy, Default)]
pub struct BaseFill;

impl Widget for BaseFill {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let base = theme::BASE_BG();
        let area = area.intersection(*buf.area());
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_bg(base);
                }
            }
        }
    }
}

/// The `Clear` drop-in that blanks onto `BASE_BG()` instead of the default.
///
/// `reset()` returns the cell to a pristine state (space symbol, no styles,
/// `Color::Reset` everywhere); we then re-apply base bg. fg/modifier are left
/// at their pristine defaults, identical to what `Clear` produced — only the
/// landing background differs.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClearBase;

impl Widget for ClearBase {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let base = theme::BASE_BG();
        let area = area.intersection(*buf.area());
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.reset();
                    cell.set_bg(base);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BaseFill, ClearBase};
    use crate::theme;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};
    use ratatui::widgets::Widget;

    const AREA: Rect = Rect::new(0, 0, 4, 3);

    fn buffer_with_content() -> Buffer {
        let mut buf = Buffer::empty(AREA);
        buf.set_style(AREA, Style::new().fg(Color::Red).bg(Color::Blue));
        for y in 0..AREA.height {
            for x in 0..AREA.width {
                buf.cell_mut((x, y)).unwrap().set_symbol("x");
            }
        }
        buf
    }

    /// INV-5: BaseFill patches bg only — symbols and fg must survive.
    #[test]
    fn base_fill_preserves_symbols() {
        let mut buf = buffer_with_content();
        BaseFill.render(AREA, &mut buf);
        for y in 0..AREA.height {
            for x in 0..AREA.width {
                let cell = &buf[(x, y)];
                assert_eq!(cell.symbol(), "x", "symbol clobbered at {x},{y}");
                assert_eq!(cell.fg, Color::Red, "fg clobbered at {x},{y}");
                assert_eq!(cell.bg, theme::BASE_BG(), "bg not patched at {x},{y}");
            }
        }
    }
    /// INV-4: ClearBase blanks the cell exactly like Clear, but lands on the
    /// base background instead of the terminal default — no Reset bg survives.
    #[test]
    fn clear_base_blanks_cell_onto_base_bg() {
        let mut buf = buffer_with_content();
        ClearBase.render(AREA, &mut buf);
        for y in 0..AREA.height {
            for x in 0..AREA.width {
                let cell = &buf[(x, y)];
                assert_eq!(cell.symbol(), " ", "cell not blanked at {x},{y}");
                assert_eq!(cell.fg, Color::Reset, "fg not reset at {x},{y}");
                assert_eq!(
                    cell.modifier,
                    Modifier::empty(),
                    "modifier not reset at {x},{y}"
                );
                assert_eq!(cell.bg, theme::BASE_BG(), "bg not on base at {x},{y}");
                assert_ne!(cell.bg, Color::Reset, "transparency hole at {x},{y}");
            }
        }
    }
}
