//! CommonMark → styled ratatui lines.
//!
//! Full port from vitriol-tui/src/markdown.rs (this repo, Apache-2.0;
//! renderer is original work). Renders markdown via `pulldown-cmark` into a
//! `Vec<Line<'static>>`: headings, bold/emphasis, inline + fenced code,
//! lists, blockquotes, rules, links, and pipe tables. Paragraphs are
//! hard-wrapped to `width` so scroll offsets stay in logical-line units.
//! Styles come from the Vitriolum theme; the module owns no layout state.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::theme;

/// The inline-code surface color (cyan on panel).
fn code_style() -> Style {
    Style::new().fg(theme::CYAN).bg(theme::PANEL)
}

/// The code-block body color (foreground text on panel).
fn code_block_style() -> Style {
    Style::new().fg(theme::TEXT).bg(theme::PANEL)
}

/// Lazily-loaded syntect syntaxes/themes (one process-wide copy).
fn syntaxes() -> &'static SyntaxSet {
    static SS: OnceLock<SyntaxSet> = OnceLock::new();
    SS.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn highlight_theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let mut set = ThemeSet::load_defaults();
        set.themes
            .remove("base16-ocean.dark")
            .or_else(|| set.themes.values().next().cloned())
            .expect("syntect ships at least one theme")
    })
}

/// Map a syntect style onto the Vitriol panel surface.
fn syn_style(src: syntect::highlighting::Style) -> Style {
    let fg = Color::Rgb(src.foreground.r, src.foreground.g, src.foreground.b);
    let mut style = Style::new().fg(fg).bg(theme::PANEL);
    if src.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if src.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if src.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
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
    /// Active syntect highlighter for the current fenced block.
    code_hl: Option<HighlightLines<'static>>,
    /// Active heading style, when inside a heading.
    heading: Option<Style>,
    /// Inline modifier stack (strong / emphasis / link).
    inline: Vec<Modifier>,
    /// Table cells accumulated for the current row.
    row_cells: Vec<String>,
    /// Fully collected table rows (rendered when End(Table) arrives).
    table_rows: Vec<Vec<String>>,
    /// Number of leading header rows (normally 1).
    table_header_rows: usize,
    /// Inside a table block.
    in_table: bool,
    /// Per-column alignment from the table delimiter row.
    table_align: Vec<pulldown_cmark::Alignment>,
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
            code_hl: None,
            heading: None,
            inline: Vec::new(),
            row_cells: Vec::new(),
            table_rows: Vec::new(),
            table_header_rows: 0,
            in_table: false,
            table_align: Vec::new(),
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
                self.cur.push(Span::styled(" ".repeat(pw), theme::text()));
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
            E::Start(T::CodeBlock(kind)) => {
                self.flush();
                self.gap();
                let lang = match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => info
                        .split(|c: char| c == ',' || c.is_whitespace())
                        .next()
                        .unwrap_or(""),
                    pulldown_cmark::CodeBlockKind::Indented => "",
                };
                let ss = syntaxes();
                let syntax = if lang.is_empty() {
                    ss.find_syntax_plain_text()
                } else {
                    ss.find_syntax_by_token(lang)
                        .or_else(|| ss.find_syntax_by_extension(lang))
                        .unwrap_or_else(|| ss.find_syntax_plain_text())
                };
                self.code_hl = Some(HighlightLines::new(syntax, highlight_theme()));
                self.in_code = true;
            }
            E::End(TE::CodeBlock) => {
                self.in_code = false;
                self.code_hl = None;
                self.flush();
                self.gap();
            }
            E::Start(T::Table(aligns)) => {
                self.flush();
                self.gap();
                self.table_rows.clear();
                self.table_header_rows = 0;
                self.table_align = aligns.clone();
                self.in_table = true;
            }
            E::End(TE::Table) => {
                self.in_table = false;
                self.render_table();
                self.gap();
            }
            E::Start(T::TableHead) => {
                self.flush();
            }
            E::End(TE::TableHead) => {
                self.collect_table_row();
                self.table_header_rows = self.table_rows.len();
            }
            E::Start(T::TableRow) => {
                self.flush();
                self.row_cells.clear();
            }
            E::End(TE::TableRow) => self.collect_table_row(),
            E::Start(T::TableCell) => {
                self.flush();
            }
            E::End(TE::TableCell) => {
                let cell = self.take_line_text();
                self.row_cells.push(cell);
            }
            _ => {}
        }
    }

    /// Handle events while inside a code block (raw text lines), with
    /// syntect syntax highlighting. Code is not wrapped: one visual row per
    /// source line, truncated with `…` at the right edge.
    fn code_event(&mut self, e: &pulldown_cmark::Event) {
        match e {
            pulldown_cmark::Event::Text(s) => {
                let ss = syntaxes();
                let width = self.width;
                for line in s.lines() {
                    let n = line.chars().count();
                    let truncated = n > width;
                    let shown: String = if truncated {
                        line.chars().take(width.saturating_sub(1)).collect()
                    } else {
                        line.to_string()
                    };
                    let mut spans: Vec<Span<'static>> = Vec::new();
                    if let Some(hl) = self.code_hl.as_mut() {
                        match hl.highlight_line(&shown, ss) {
                            Ok(ranges) => {
                                for (style, text) in ranges {
                                    if !text.is_empty() {
                                        spans
                                            .push(Span::styled(text.to_string(), syn_style(style)));
                                    }
                                }
                            }
                            Err(_) => spans.push(Span::styled(shown.clone(), code_block_style())),
                        }
                    } else {
                        spans.push(Span::styled(shown.clone(), code_block_style()));
                    }
                    if truncated {
                        spans.push(Span::styled("…", theme::muted()));
                    }
                    if spans.is_empty() {
                        spans.push(Span::styled(String::new(), code_block_style()));
                    }
                    self.out.push(Line::from(spans));
                }
            }
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::CodeBlock) => {
                self.in_code = false;
                self.code_hl = None;
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

    /// Stash the current row for aligned rendering at End(Table).
    fn collect_table_row(&mut self) {
        if self.row_cells.is_empty() {
            return;
        }
        self.table_rows.push(std::mem::take(&mut self.row_cells));
    }

    /// Render the collected table with aligned columns. Natural widths win
    /// when they fit; otherwise columns shrink to a readable floor and cell
    /// CONTENT WRAPS across lines (automatic overflow). If even that cannot
    /// fit (too many columns), rows fall back to a stacked record layout so
    /// every field stays readable on narrow/phone terminals.
    fn render_table(&mut self) {
        let rows = std::mem::take(&mut self.table_rows);
        let aligns = std::mem::take(&mut self.table_align);
        let header = self
            .table_header_rows
            .max(if rows.is_empty() { 0 } else { 1 })
            .min(rows.len());
        self.table_header_rows = 0;
        if rows.is_empty() {
            return;
        }
        let ncols = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if ncols == 0 || self.width == 0 {
            return;
        }

        const GAP: usize = 3; // " │ "
        const MIN_COL: usize = 5;

        let mut widths = vec![0usize; ncols];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let natural_total: usize = widths.iter().sum::<usize>() + GAP * (ncols - 1);
        let min_total = ncols * MIN_COL + GAP * (ncols - 1);

        if natural_total > self.width {
            if self.width < min_total {
                self.render_stacked_rows(&rows, header, ncols);
                return;
            }
            // Shrink the widest columns to fit, but never below MIN_COL.
            let mut total = natural_total;
            while total > self.width {
                let Some(i) = (0..ncols)
                    .filter(|&i| widths[i] > MIN_COL)
                    .max_by_key(|&i| widths[i])
                else {
                    break;
                };
                widths[i] -= 1;
                total -= 1;
            }
        }

        let head_style = Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD);
        let body_style = theme::text();
        let sep_style = theme::muted();

        for (ri, row) in rows.iter().enumerate() {
            let style = if ri < header { head_style } else { body_style };
            let cells: Vec<Vec<String>> = (0..ncols)
                .map(|ci| {
                    let raw = row.get(ci).map(String::as_str).unwrap_or("");
                    wrap_cell(raw, widths[ci])
                })
                .collect();
            let height = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
            for li in 0..height {
                let mut spans: Vec<Span<'static>> = Vec::new();
                for ci in 0..ncols {
                    if ci > 0 {
                        spans.push(Span::styled(" │ ", sep_style));
                    }
                    let chunk = cells[ci].get(li).map(String::as_str).unwrap_or("");
                    let pad = widths[ci].saturating_sub(chunk.chars().count());
                    let align = aligns
                        .get(ci)
                        .copied()
                        .unwrap_or(pulldown_cmark::Alignment::None);
                    let (left_pad, right_pad) = match align {
                        pulldown_cmark::Alignment::Right => (pad, 0),
                        pulldown_cmark::Alignment::Center => (pad / 2, pad - pad / 2),
                        _ => (0, pad),
                    };
                    if left_pad > 0 {
                        spans.push(Span::styled(" ".repeat(left_pad), style));
                    }
                    spans.push(Span::styled(chunk.to_string(), style));
                    if right_pad > 0 {
                        spans.push(Span::styled(" ".repeat(right_pad), style));
                    }
                }
                self.out.push(Line::from(spans));
            }
            if ri + 1 == header {
                let mut sep: Vec<Span<'static>> = Vec::new();
                for ci in 0..ncols {
                    if ci > 0 {
                        sep.push(Span::styled("─┼─", sep_style));
                    }
                    sep.push(Span::styled("─".repeat(widths[ci]), sep_style));
                }
                self.out.push(Line::from(sep));
            }
        }
    }

    /// Vertical/record fallback for narrow terminals and high column counts.
    /// Every value is preserved; each record reads as `Header: value` rows.
    fn render_stacked_rows(&mut self, rows: &[Vec<String>], header: usize, ncols: usize) {
        let labels: Vec<String> = (0..ncols)
            .map(|ci| {
                let from_head = rows
                    .get(0)
                    .and_then(|r| r.get(ci))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                if header > 0 && !from_head.is_empty() {
                    from_head
                } else {
                    format!("field {}", ci + 1)
                }
            })
            .collect();
        let label_w = labels
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(4)
            .min(self.width.saturating_sub(10).max(4))
            .max(4);

        let body_start = header.min(rows.len());
        let body: Vec<&Vec<String>> = rows.iter().skip(body_start).collect();
        let head_style = Style::new().fg(theme::GOLD).add_modifier(Modifier::BOLD);
        let body_style = theme::text();
        let muted = theme::muted();

        if body.is_empty() {
            // Header-only table: render one record so nothing vanishes.
            for (ci, label) in labels.iter().enumerate() {
                let value = rows
                    .get(0)
                    .and_then(|r| r.get(ci))
                    .cloned()
                    .unwrap_or_default();
                if value.trim().is_empty() || value.trim() == label.trim() {
                    continue;
                }
                self.render_stacked_field(label, &value, label_w, head_style, body_style);
            }
            return;
        }

        for (ri, row) in body.iter().enumerate() {
            if ri > 0 {
                self.out.push(Line::from(""));
            }
            // Lead with the first meaningful column as a record marker.
            let mut first_written = false;
            for (ci, label) in labels.iter().enumerate() {
                let value = row.get(ci).map(String::as_str).unwrap_or("").trim();
                if value.is_empty() {
                    continue;
                }
                let style = if first_written {
                    body_style
                } else {
                    body_style
                };
                let prefix = if first_written { "  " } else { "▸ " };
                self.render_stacked_field_with_prefix(
                    prefix, label, value, label_w, head_style, style, muted,
                );
                first_written = true;
            }
        }
    }

    fn render_stacked_field(
        &mut self,
        label: &str,
        value: &str,
        label_w: usize,
        label_style: Style,
        value_style: Style,
    ) {
        self.render_stacked_field_with_prefix(
            "▸ ",
            label,
            value,
            label_w,
            label_style,
            value_style,
            theme::muted(),
        );
    }

    fn render_stacked_field_with_prefix(
        &mut self,
        prefix: &str,
        label: &str,
        value: &str,
        label_w: usize,
        label_style: Style,
        value_style: Style,
        muted: Style,
    ) {
        let prefix_w = prefix.chars().count();
        let head_w = prefix_w + label_w + 2; // ": "
        let value_w = self.width.saturating_sub(head_w).max(1);
        let chunks = wrap_cell(value, value_w);
        let label_cell = format!("{:<width$}", label, width = label_w);
        for (i, chunk) in chunks.iter().enumerate() {
            if i == 0 {
                self.out.push(Line::from(vec![
                    Span::styled(prefix.to_string(), muted),
                    Span::styled(label_cell.clone(), label_style),
                    Span::styled(": ", muted),
                    Span::styled(chunk.clone(), value_style),
                ]));
            } else {
                let indent = " ".repeat(prefix_w + label_w + 2);
                self.out.push(Line::from(vec![
                    Span::styled(indent, muted),
                    Span::styled(chunk.clone(), value_style),
                ]));
            }
        }
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
        // Blockquote prefix is added AFTER wrapping — reserve its 2 columns
        // so prefixed lines never exceed width (owner bug 2026-09-03).
        let budget = if self.quote > 0 {
            self.width.saturating_sub(2)
        } else {
            self.width
        };
        let mut wrapped = wrap_line(&line, budget, hang);
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
            if self
                .out
                .last()
                .map_or(false, |l: &Line| l.to_string().is_empty())
            {
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

/// Wrap a table cell to `w` columns. Words wrap; overlong unbreakable tokens
/// are hard-broken. Empty lines are preserved so multi-line cells stay stable.
fn wrap_cell(text: &str, w: usize) -> Vec<String> {
    let width = w.max(1);
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let mut line = String::new();
        for word in raw.split_whitespace() {
            let mut rest: Vec<char> = word.chars().collect();
            while !rest.is_empty() {
                let room = if line.is_empty() {
                    width
                } else {
                    width.saturating_sub(line.chars().count() + 1)
                };
                if room == 0 {
                    out.push(std::mem::take(&mut line));
                    continue;
                }
                if rest.len() <= room {
                    if !line.is_empty() {
                        line.push(' ');
                    }
                    line.extend(rest.drain(..));
                } else if line.is_empty() {
                    let chunk: String = rest.drain(..room).collect();
                    out.push(chunk);
                } else {
                    out.push(std::mem::take(&mut line));
                }
            }
        }
        if !line.is_empty() {
            out.push(line);
        } else if raw.trim().is_empty() {
            out.push(String::new());
        }
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
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
fn push_chars_hang(
    out: &mut Vec<Line<'static>>,
    cur: &mut Vec<(char, Style)>,
    hang: usize,
    line_idx: usize,
) {
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
        let text =
            "- this is a very long list item that should definitely wrap at the width boundary\n";
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
        assert!(
            gap_pos.is_some(),
            "blank line between paragraphs: {:?}",
            strs
        );
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
    fn pipe_table_columns_align() {
        let md = "| Name | Qty |\n| --- | --- |\n| alpha | 1 |\n| beta | 22 |\n";
        let lines = render(md, 40);
        let strs = text_lines(&lines);
        let rows: Vec<&String> = strs.iter().filter(|s| s.contains('│')).collect();
        assert_eq!(rows.len(), 3, "header + 2 body rows: {:?}", strs);
        assert!(
            strs.iter().any(|s| s.contains('┼')),
            "header rule: {:?}",
            strs
        );
        let pos: Vec<usize> = rows
            .iter()
            .map(|s| s.chars().position(|c| c == '│').unwrap_or(usize::MAX))
            .collect();
        assert!(
            pos.windows(2).all(|w| w[0] == w[1]),
            "columns aligned: {:?}",
            rows
        );
        assert!(rows[0].contains("Name") && rows[0].contains("Qty"));
        assert!(rows[1].contains("alpha") && rows[2].contains("beta"));
    }

    #[test]
    fn pipe_table_fits_narrow_width() {
        let md = format!(
            "| {} | {} |\n| --- | --- |\n| {} | {} |\n",
            "a".repeat(20),
            "b".repeat(20),
            "c",
            "d"
        );
        for line in render(&md, 30) {
            assert!(line.width() <= 30, "table line width {}", line.width());
        }
    }

    #[test]
    fn gap_after_list_before_paragraph() {
        let lines = render("- item\n\nnext paragraph\n", 40);
        let strs = text_lines(&lines);
        let gap_pos = strs.iter().position(|s| s.is_empty());
        assert!(
            gap_pos.is_some(),
            "blank line between list and paragraph: {:?}",
            strs
        );
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

#[cfg(test)]
mod overflow_tests {
    use super::*;

    /// Owner bug 2026-09-03: raw code lines re-wrapped in the paragraph and
    /// pushed transcript rows off the bottom. Every rendered line must fit.
    #[test]
    fn long_code_lines_truncate_not_wrap() {
        let code = "x".repeat(300);
        let md = format!("```\n{code}\n```\n");
        let lines = render(&md, 40);
        assert_eq!(lines.len(), 1, "one source line = one visual line");
        assert!(lines[0].width() <= 40, "line width {}", lines[0].width());
        assert!(lines[0].to_string().ends_with('…'), "loss marked");
    }

    #[test]
    fn fenced_code_is_syntax_highlighted() {
        let md = "```rust\nfn main() { let x = \"hi\"; }\n```\n";
        let lines = render(md, 80);
        let line = lines
            .iter()
            .find(|l| l.to_string().contains("fn main"))
            .expect("code line rendered");
        assert!(
            line.spans.len() > 1,
            "multiple styled tokens: {:?}",
            line.spans
        );
        let colors: std::collections::HashSet<_> = line.spans.iter().map(|s| s.style.fg).collect();
        assert!(colors.len() > 1, "keywords/strings differ: {:?}", colors);
        assert_eq!(line.to_string().trim(), "fn main() { let x = \"hi\"; }");
    }

    #[test]
    fn unknown_code_language_still_renders() {
        let md = "```not-a-language\nplain text line\n```\n";
        let lines = render(md, 40);
        assert!(lines
            .iter()
            .any(|l| l.to_string().contains("plain text line")));
    }

    #[test]
    fn table_wraps_cells_instead_of_truncating() {
        let md = "| Name | Description |\n| --- | --- |\n| alpha | This is a fairly long description that should wrap neatly into several lines |\n";
        let lines = render(md, 40);
        let strs: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        assert!(lines.iter().all(|l| l.width() <= 40));
        let joined = strs.join(" ");
        assert!(
            joined.contains("description"),
            "content preserved: {joined}"
        );
        assert!(joined.contains("neatly"), "tail preserved: {joined}");
        assert!(!joined.contains('…'), "no ellipsis truncation: {joined}");
        let rows: Vec<&String> = strs.iter().filter(|s| s.contains('│')).collect();
        assert!(rows.len() > 2, "wrapped row: {strs:?}");
        let pos: Vec<usize> = rows
            .iter()
            .map(|s| s.chars().position(|c| c == '│').unwrap_or(usize::MAX))
            .collect();
        assert!(
            pos.windows(2).all(|w| w[0] == w[1]),
            "columns stay aligned: {rows:?}"
        );
    }

    #[test]
    fn table_stacks_into_records_when_too_many_columns() {
        let md = "| H1 | H2 | H3 | H4 | H5 | H6 | H7 | H8 |\n|---|---|---|---|---|---|---|---|\n| v1 | v2 | v3 | v4 | v5 | v6 | v7 | v8 |\n";
        let lines = render(md, 30);
        assert!(lines.iter().all(|l| l.width() <= 30), "fits narrow width");
        let strs: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let joined = strs.join("\n");
        assert!(joined.contains('▸'), "record marker: {joined}");
        assert!(joined.contains("H8"), "last header kept: {joined}");
        assert!(joined.contains("v8"), "last value kept: {joined}");
    }

    #[test]
    fn wide_table_rows_fit_width() {
        let cells = vec!["a".repeat(15), "b".repeat(15), "c".repeat(15)];
        let md = format!(
            "| {} | {} | {} |\n| --- | --- | --- |\n| x | y | z |\n",
            cells[0], cells[1], cells[2]
        );
        let lines = render(&md, 30);
        assert!(
            lines.iter().any(|l| l.to_string().contains('│')),
            "parsed as a table"
        );
        for l in &lines {
            assert!(l.width() <= 30, "table line width {}", l.width());
        }
    }

    #[test]
    fn blockquote_prefix_never_exceeds_width() {
        let text = format!("> {}\n", "word ".repeat(30));
        let lines = render(&text, 30);
        assert!(!lines.is_empty());
        for l in &lines {
            assert!(l.width() <= 30, "quote line width {}", l.width());
        }
    }
}
