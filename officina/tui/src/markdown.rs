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
                self.flush();
                let h1 = *level == pulldown_cmark::HeadingLevel::H1;
                self.heading = Some(heading_style(h1));
            }
            E::End(TE::Heading(_)) => {
                self.heading = None;
                self.flush();
            }
            E::Start(T::Paragraph) | E::End(TE::Paragraph) => {
                if let E::End(_) = e {
                    self.flush();
                }
            }
            E::Text(s) => self.text_span(s),
            E::Code(s) => self.push(s, code_style()),
            E::SoftBreak => self.push(" ", self.base()),
            E::HardBreak => self.flush(),
            E::Rule => {
                self.flush();
                self.cur
                    .push(Span::styled("─".repeat(self.width), theme::muted()));
                self.flush();
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
            E::Start(T::List(start)) => self.lists.push((*start, start.unwrap_or(0))),
            E::End(TE::List(_)) => {
                self.lists.pop();
            }
            E::Start(T::Item) => {
                self.flush();
                let (bullet, width) = self.item_prefix();
                self.cur.push(Span::styled(bullet, theme::muted()));
                self.cur
                    .push(Span::styled(" ".repeat(width), theme::text()));
            }
            E::End(TE::Item) => self.flush(),
            E::Start(T::BlockQuote(_)) => {
                self.flush();
                self.quote += 1;
            }
            E::End(TE::BlockQuote(_)) => {
                self.flush();
                self.quote = self.quote.saturating_sub(1);
            }
            E::Start(T::CodeBlock(_)) => {
                self.flush();
                self.in_code = true;
            }
            E::End(TE::CodeBlock) => {
                self.in_code = false;
                self.flush();
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
    fn flush(&mut self) {
        if self.cur.is_empty() {
            return;
        }
        let line = Line::from(std::mem::take(&mut self.cur));
        let mut wrapped = wrap_line(&line, self.width);
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

    /// Flush any remaining content at end of input.
    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush();
        self.out
    }
}

/// Word-wrap a single `Line` into `width`-limited lines, preserving span styles
/// by coalescing adjacent styled characters with equal styles.
fn wrap_line(line: &Line, width: usize) -> Vec<Line<'static>> {
    let chars: Vec<(char, Style)> = line
        .spans
        .iter()
        .flat_map(|s| s.content.chars().map(|c| (c, s.style)))
        .collect();
    if chars.is_empty() {
        return vec![Line::from(vec![])];
    }

    let mut out: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<(char, Style)> = Vec::new();
    let mut cur_w = 0usize;
    for (c, style) in chars {
        if c == ' ' && cur_w >= width {
            push_chars(&mut out, &mut cur);
            cur_w = 0;
            continue;
        }
        if cur_w > 0 && cur_w + 1 > width {
            push_chars(&mut out, &mut cur);
            cur_w = 0;
        }
        cur.push((c, style));
        cur_w += 1;
    }
    push_chars(&mut out, &mut cur);
    out
}

/// Coalesce a char+style run into spans and push as one wrapped line.
fn push_chars(out: &mut Vec<Line<'static>>, cur: &mut Vec<(char, Style)>) {
    let mut spans: Vec<Span<'static>> = Vec::new();
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
}
