//! CommonMark → styled ratatui lines.
//!
//! Full port from vitriol-tui/src/markdown.rs (this repo, Apache-2.0;
//! renderer is original work). Renders markdown via `pulldown-cmark` into a
//! `Vec<Line<'static>>`: headings, bold/emphasis, inline + fenced code,
//! lists, blockquotes, rules, links, and pipe tables. Paragraphs are
//! hard-wrapped to `width` so scroll offsets stay in logical-line units.
//! Styles come from the Vitriolum theme; the module owns no layout state.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme;

/// The inline-code surface color (cyan on panel).
fn code_style() -> Style {
    Style::new().fg(theme::CYAN).bg(theme::PANEL)
}

/// The code-block body color (foreground text on panel).
fn code_block_style() -> Style {
    Style::new().fg(theme::TEXT).bg(theme::PANEL)
}

/// The heading style: H1 = gold banner, deeper levels = green title.
fn heading_style(h1: bool) -> Style {
    if h1 {
        theme::banner()
    } else {
        theme::title()
    }
}

/// Render `text` into wrapped, styled lines of at most `width` columns.
pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    use pulldown_cmark::{Options, Parser};
    let mut r = Renderer::new(width);
    for event in Parser::new_ext(text, Options::all()) {
        r.event(&event);
    }
    r.finish()
}

/// Pushdown state for the markdown event walk.
struct Renderer {
    /// Final wrapped lines.
    out: Vec<Line<'static>>,
    /// The line currently being accumulated.
    cur: Vec<Span<'static>>,
    /// Wrap width in columns.
    width: usize,
    /// List stack: None = bullet, Some(start) = ordered with its next index.
    lists: Vec<(Option<u64>, u64)>,
    /// Accumulated prefix width of all ancestor items (for nested indent).
    indent: usize,
    /// Current item's continuation indent column (indent + own prefix width).
    hang: usize,
    /// Stack of own prefix widths, parallel to `lists`, for safe pop on End(Item).
    prefix_widths: Vec<usize>,
    /// Blockquote nesting depth.
    quote: usize,
    /// Inside a fenced/indented code block.
    in_code: bool,
    /// Active heading style, when inside a heading.
    heading: Option<Style>,
    /// Inline modifier stack (strong / emphasis / link).
    inline: Vec<Modifier>,
    /// Table cells accumulated for the current row.
    row_cells: Vec<String>,
    /// Whether the current table row is the header.
    row_head: bool,
}

impl Renderer {
    fn new(width: usize) -> Self {
        Self {
            out: Vec::new(),
            cur: Vec::new(),
            width: width.max(1),
            lists: Vec::new(),
            indent: 0,
            hang: 0,
            prefix_widths: Vec::new(),
            quote: 0,
            in_code: false,
            heading: None,
            inline: Vec::new(),
            row_cells: Vec::new(),
            row_head: false,
        }
    }

    fn event(&mut self, e: &pulldown_cmark::Event) {
        use pulldown_cmark::{Event as E, Tag as T, TagEnd as TE};
        if self.in_code {
            self.code_event(e);
            return;
        }
        match e {
            E::Start(T::Heading { level, .. }) => {
                self.gap();
                self.flush();
                let h1 = *level == pulldown_cmark::HeadingLevel::H1;
                self.heading = Some(heading_style(h1));
            }
            E::End(TE::Heading(_)) => {
                self.heading = None;
                self.flush();
            }
            E::Start(T::Paragraph) => {}
            E::End(TE::Paragraph) => {
                self.flush();
                self.gap();
            }
            E::Text(s) => self.text_span(s),
            E::Code(s) => self.push(s, code_style()),
            E::SoftBreak => self.push(" ", self.base()),
            E::HardBreak => self.flush(),
            E::Rule => {
                self.flush();
                self.gap();
                self.cur
                    .push(Span::styled("─".repeat(self.width), theme::muted()));
                self.flush();
                self.gap();
            }
            E::Start(T::Strong) => self.inline.push(Modifier::BOLD),
            E::End(TE::Strong) => {
                self.inline.pop();
            }
            E::Start(T::Emphasis) => self.inline.push(Modifier::ITALIC),
            E::End(TE::Emphasis) => {
                self.inline.pop();
            }
            E::Start(T::Link { .. }) => self.inline.push(Modifier::UNDERLINED),
            E::End(TE::Link) => {
                self.inline.pop();
            }
            E::Start(T::List(_start)) => {
                if self.lists.is_empty() {
                    self.gap();
                }
                self.lists.push((*_start, _start.unwrap_or(0)));
            }
            E::End(TE::List(_)) => {
                self.lists.pop();
                // No gap here — the next block's Start(List)/End(Paragraph)
                // etc. will emit a gap if needed. Gap at End(List) would
                // produce a trailing blank line when a list is the last block.
            }
            E::Start(T::Item) => {
                self.flush();
                let (bullet, pw) = self.item_prefix();
                self.cur.push(Span::styled(bullet, theme::muted()));
                self.cur
                    .push(Span::styled(" ".repeat(pw), theme::text()));
                self.indent += pw;
                self.hang = self.indent;
                self.prefix_widths.push(pw);
            }
            E::End(TE::Item) => {
                self.flush();
                let pw = self.prefix_widths.pop().unwrap_or(0);
                self.indent = self.indent.saturating_sub(pw);
                self.hang = 0;
            }
            E::Start(T::BlockQuote(_)) => {
                self.flush();
                if self.quote == 0 {
                    self.gap();
                }
                self.quote += 1;
            }
            E::End(TE::BlockQuote(_)) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
                if self.quote == 0 {
                    self.gap();
                }
            }
            E::Start(T::CodeBlock(_)) => {
                self.flush();
                self.gap();
                self.in_code = true;
            }
            E::End(TE::CodeBlock) => {
                self.in_code = false;
                self.flush();
                self.gap();
            }
            E::Start(T::TableHead) => {
                self.flush();
                self.row_head = true;
            }
            E::End(TE::TableHead) => {
                self.flush_table_row();
                self.row_head = false;
            }
            E::Start(T::TableRow) => {
                self.flush();
                self.row_cells.clear();
            }
            E::End(TE::TableRow) => self.flush_table_row(),
            E::Start(T::TableCell) => {
                self.flush();
            }
            E::End(TE::TableCell) => {
                let cell = self.take_line_text();
                self.row_cells.push(cell);
            }
            E::Start(T::Table(_)) | E::End(TE::Table) => {}
            _ => {}
        }
    }

    /// Handle events while inside a code block (raw text lines).
    fn code_event(&mut self, e: &pulldown_cmark::Event) {
        match e {
            pulldown_cmark::Event::Text(s) => {
                for line in s.lines() {
                    self.out.push(Line::from(Span::styled(
                        line.to_string(),
                        code_block_style(),
                    )));
                }
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                self.in_code = false;
            }
            _ => {}
        }
    }

    /// The bullet or ordered prefix for the current item.
    fn item_prefix(&mut self) -> (String, usize) {
        if let Some((Some(start), idx)) = self.lists.last_mut() {
            let label = format!("{idx}.");
            *idx += 1;
            let _ = start;
            (label.clone(), label.len() + 1)
        } else {
            ("•".to_string(), 2)
        }
    }

    /// The base style for inline text (heading style overrides body).
    fn base(&self) -> Style {
        self.heading.unwrap_or_else(theme::text)
    }

    /// Push a text span with the active inline modifiers and quote prefix.
    fn text_span(&mut self, s: &str) {
        let mut style = self.base();
        for m in &self.inline {
            style = style.add_modifier(*m);
        }
        self.push(s, style);
    }

    /// Push a raw span; prepend the blockquote prefix when at line start.
    fn push(&mut self, s: &str, style: Style) {
        if self.cur.is_empty() && self.quote > 0 {
            self.cur
                .push(Span::styled("│ ".repeat(self.quote), theme::muted()));
        }
        self.cur.push(Span::styled(s.to_string(), style));
    }

    /// Join the current table row cells into one line and flush it.
    fn flush_table_row(&mut self) {
        if self.row_cells.is_empty() {
            return;
        }
        let sep = if self.row_head { "┃" } else { "│" };
        self.cur.push(Span::styled(
            self.row_cells.join(&format!(" {sep} ")),
            theme::text(),
        ));
        self.flush();
    }

    /// The current line as plain text (for table cells), then clear it.
    fn take_line_text(&mut self) -> String {
        let mut out = String::new();
        for s in self.cur.drain(..) {
            out.push_str(&s.content);
        }
        out
    }

    /// Wrap the current line to `width` and push it onto `out`.
    /// Hanging indent: when `hang > 0`, the first line wraps to `width`
    /// (prefix already in cur), continuation lines wrap to `width - hang`
    /// and are prefixed with `hang` blank spaces (owner request 2026-09-03).
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let line = Line::from(std::mem::take(&mut self.cur));
        let hang = self.hang;
        let mut wrapped = wrap_line(&line, self.width, hang);
        if self.quote > 0 {
            let prefix = "│ ".repeat(self.quote);
            for w in &mut wrapped {
                let mut spans = vec![Span::styled(prefix.clone(), theme::muted())];
                spans.extend(w.spans.clone());
                *w = Line::from(spans);
            }
        }
        self.out.extend(wrapped);
    }

    /// Push a blank separator line between blocks (paragraphs, headings,
    /// code blocks, lists, blockquotes, rules) — but never at document
    /// start and never if one already exists (owner request 2026-09-03).
    fn gap(&mut self) {
        if !self.out.is_empty() && self.cur.is_empty() {
            if self.out.last().map_or(false, |l: &Line| l.to_string().is_empty()) {
                return;
            }
            self.out.push(Line::from(""));
        }
    }

    /// Flush any remaining content at end of input.
    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        self.out
    }
}

/// Word-wrap a single `Line` into `width`-limited lines, preserving span styles
/// by coalescing adjacent styled characters with equal styles.
///
/// `hang`: when > 0, the first line wraps to `width` and continuation lines
/// wrap to `width - hang`, prefixed with `hang` blank spaces (hanging indent
/// for list items — owner request 2026-09-03).
fn wrap_line(line: &Line, width: usize, hang: usize) -> Vec<Line<'static>> {
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(|c| (c, s.style)))
        .collect();
    if chars.is_empty() {
        return vec![Line::from(vec![])];
    }

    // First line: full width. Continuation lines: width - hang.
    let first_w = width;
    let cont_w = width.saturating_sub(hang);

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_w = 0usize;
    let mut done = 0usize; // lines already flushed
    let mut line_budget = first_w;

    for (c, style) in &chars {
        let c = *c;
        let style = *style;
        if c == ' ' && cur_w >= line_budget {
            push_chars_hang(&mut out, &mut cur, hang, done);
            done += 1;
            line_budget = cont_w;
            cur_w = 0;
            continue;
        }
        if cur_w > 0 && cur_w + 1 > line_budget {
            push_chars_hang(&mut out, &mut cur, hang, done);
            done += 1;
            line_budget = cont_w;
            cur_w = 0;
        }
        cur.push((c, style));
        cur_w += 1;
    }
    push_chars_hang(&mut out, &mut cur, hang, done);
    out
}

/// Coalesce a char+style run into spans, prepending `hang` blank spaces
/// on continuation lines (line_idx > 0), and push as one wrapped line.
fn push_chars_hang(out: &mut Vec<Line<'static>>, cur: &mut Vec<(char, Style)>, hang: usize, line_idx: usize) {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if hang > 0 && line_idx > 0 {
        spans.push(Span::styled(" ".repeat(hang), Style::default()));
    }
    for (c, style) in cur.drain(..) {
        if spans
            .last()
            .map(|s: &Span| s.style == style)
            .unwrap_or(false)
        {
            let last = spans.last_mut().unwrap();
            let mut content = last.content.clone().into_owned();
            content.push(c);
            last.content = content.into();
        } else {
            spans.push(Span::styled(c.to_string(), style));
        }
    }
    if !spans.is_empty() {
        out.push(Line::from(spans));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_lines(lines: &[Line]) -> Vec<String> {
        lines.iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn renders_heading_and_paragraph() {
        let lines = render("# Title\n\nbody text\n", 40);
        let text = text_lines(&lines).join("\n");
        assert!(text.contains("Title"));
        assert!(text.contains("body text"));
    }

    #[test]
    fn wraps_long_paragraphs_to_width() {
        let text = "word ".repeat(60);
        let lines = render(&text, 20);
        assert!(lines.len() > 3);
        assert!(lines.iter().all(|l| l.width() <= 20));
    }

    #[test]
    fn renders_fenced_code_block() {
        let lines = render("```rust\nfn main() {}\n```\n", 40);
        let text = text_lines(&lines).join("\n");
        assert!(text.contains("fn main() {}"));
    }

    #[test]
    fn renders_bullet_and_ordered_lists() {
        let lines = render("- a\n- b\n\n1. one\n2. two\n", 40);
        let joined = text_lines(&lines).join("\n");
        assert!(joined.contains('•'));
        assert!(joined.contains("1."));
        assert!(joined.contains("2."));
    }

    #[test]
    fn renders_inline_code() {
        let lines = render("use `braille::bar`", 40);
        let joined = text_lines(&lines).join("\n");
        assert!(joined.contains("braille::bar"));
    }

    #[test]
    fn renders_table_rows() {
        let lines = render("| a | b |\n|---|---|\n| 1 | 2 |\n", 40);
        let joined = text_lines(&lines).join("\n");
        assert!(joined.contains('│'));
        assert!(joined.contains("a"));
        assert!(joined.contains("2"));
    }

    #[test]
    fn renders_blockquote_prefix() {
        let lines = render("> quoted\n", 40);
        let joined = text_lines(&lines).join("\n");
        assert!(joined.contains('│'));
    }

    #[test]
    fn empty_input_yields_no_lines() {
        assert!(render("", 40).is_empty());
    }

    // ── 2026-09-03: hanging indent + paragraph gaps ──────────────────────

    #[test]
    fn list_item_hanging_indent() {
        // A long item whose text wraps: continuation lines must align
        // under the item's text start (column 2), not under the bullet.
        let text = "- this is a very long list item that should definitely wrap at the width boundary\n";
        let lines = render(text, 30);
        let strs = text_lines(&lines);
        // First line has the bullet; continuation lines start with 2 spaces.
        assert!(strs[0].contains("•"));
        for s in &strs[1..] {
            assert!(
                s.starts_with("  "),
                "continuation should be indented 2 cols: {:?}",
                s
            );
        }
    }

    #[test]
    fn ordered_list_hanging_indent() {
        let text = "1. first item\n2. a very long second item that wraps across multiple lines at the boundary here yes\n";
        let lines = render(text, 35);
        let strs = text_lines(&lines);
        // Second item starts with "2." then wraps.
        let second_start = strs.iter().position(|s| s.starts_with("2.")).unwrap();
        // The next line after "2. ..." must start with 3 spaces (indent = len("2.") + 1 = 3).
        if second_start + 1 < strs.len() {
            assert!(
                strs[second_start + 1].starts_with("   "),
                "ordered continuation indent 3: {:?}",
                strs[second_start + 1]
            );
        }
    }

    #[test]
    fn paragraph_gap_between_blocks() {
        let lines = render("first\n\nsecond\n", 40);
        let strs = text_lines(&lines);
        let gap_pos = strs.iter().position(|s| s.is_empty());
        assert!(gap_pos.is_some(), "blank line between paragraphs: {:?}", strs);
    }

    #[test]
    fn no_gap_at_document_start() {
        let lines = render("hello\n", 40);
        let strs = text_lines(&lines);
        assert!(!strs[0].is_empty(), "no leading blank line");
    }

    #[test]
    fn no_gap_between_list_items() {
        let lines = render("- a\n- b\n- c\n", 40);
        let strs = text_lines(&lines);
        let blanks: Vec<_> = strs.iter().filter(|s| s.is_empty()).collect();
        assert!(blanks.is_empty(), "no gaps between list items: {:?}", strs);
    }

    #[test]
    fn gap_after_list_before_paragraph() {
        let lines = render("- item\n\nnext paragraph\n", 40);
        let strs = text_lines(&lines);
        let gap_pos = strs.iter().position(|s| s.is_empty());
        assert!(gap_pos.is_some(), "blank line between list and paragraph: {:?}", strs);
    }

    #[test]
    fn code_block_gap() {
        let lines = render("text\n\n```\ncode\n```\n\nmore\n", 40);
        let strs = text_lines(&lines);
        // Should have blank lines before and after the code block.
        assert!(strs.iter().any(|s| s.is_empty()), "has gap: {:?}", strs);
        assert!(strs.iter().any(|s| s.contains("code")), "has code");
        assert!(strs.iter().any(|s| s.contains("text")), "has text");
    }
}
