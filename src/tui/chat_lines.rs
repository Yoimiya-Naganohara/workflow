use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Tag, TagEnd};
use ratatui::{
    layout::Constraint,
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Cell, Row, Table},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::state::{CoreState, MessageRole};
use super::style;

// ═══════════════════════════════════════════════════════════════════════════
//  Public types
// ═══════════════════════════════════════════════════════════════════════════

/// A single rendered line in the chat output.
#[derive(Debug, Clone)]
pub(crate) struct RenderedLine {
    pub line: Line<'static>,
}

/// Metadata for rendering a markdown table as a ratatui Table widget.
#[derive(Debug, Clone)]
pub(crate) struct TableDef {
    pub start_line: usize,
    pub end_line: usize,
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub col_widths: Vec<usize>,
}

/// Output of chat content building.
#[derive(Debug, Clone)]
pub(crate) struct ChatRenderOutput {
    pub rendered: Vec<RenderedLine>,
}

/// Format a table as text lines with `│` column separators so they
/// wrap naturally via Paragraph::Wrap.
/// Compute how many terminal lines a table occupies.
fn compute_table_height(td: &TableDef) -> usize {
    3 + td.rows.len()
}

fn table_as_text_lines(td: &TableDef, bar_char: &str, bar_color: ratatui::style::Color) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let sep = Span::styled(" │ ", Style::default().fg(style::TEXT_MUTED));
    let bar = Span::styled(bar_char.to_string(), Style::default().fg(bar_color));

    // Separator row
    fn make_sep(col_widths: &[usize]) -> String {
        let parts: Vec<String> = col_widths.iter().map(|&w| "─".repeat(w)).collect();
        format!("├─{}─┤", parts.join("─┼─"))
    }

    // Header
    {
        let mut cells: Vec<Span> = Vec::new();
        cells.push(bar.clone());
        cells.push(Span::styled(" ", Style::default()));
        for (i, h) in td.header.iter().enumerate() {
            if i > 0 {
                cells.push(sep.clone());
            }
            cells.push(Span::styled(
                format!("{:w$}", h, w = td.col_widths[i]),
                Style::default().add_modifier(ratatui::style::Modifier::BOLD),
            ));
        }
        cells.push(Span::styled(
            format!("{:>w$}", "│", w = 1),
            Style::default().fg(style::TEXT_MUTED),
        ));
        out.push(Line::from(cells));
    }

    // Separator
    out.push(Line::from(vec![
        bar.clone(),
        Span::styled(
            format!(" {}", make_sep(&td.col_widths)),
            Style::default().fg(style::TEXT_MUTED),
        ),
    ]));

    // Data rows
    for row in &td.rows {
        let mut cells: Vec<Span> = Vec::new();
        cells.push(bar.clone());
        cells.push(Span::styled(" ", Style::default()));
        for (i, c) in row.iter().enumerate() {
            if i > 0 {
                cells.push(sep.clone());
            }
            cells.push(Span::styled(
                format!("{:w$}", c, w = td.col_widths[i]),
                Style::default(),
            ));
        }
        cells.push(Span::styled(
            format!("{:>w$}", "│", w = 1),
            Style::default().fg(style::TEXT_MUTED),
        ));
        out.push(Line::from(cells));
    }

    // Bottom border
    out.push(Line::from(vec![
        bar.clone(),
        Span::styled(
            format!(" {}", make_sep(&td.col_widths)),
            Style::default().fg(style::TEXT_MUTED),
        ),
    ]));

    out
}

impl ChatRenderOutput {
    pub fn total_lines(&self) -> usize {
        self.rendered.len()
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main entry point
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn build_chat_content(state: &CoreState, width: usize, _think_frame: u8) -> ChatRenderOutput {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut rendered: Vec<RenderedLine> = Vec::new();

    for message in &state.messages {
        if matches!(message.role, MessageRole::System) {
            let result = render_md(&message.content, body_width);
            for md_line in result.lines {
                let mut styled = vec![Span::styled("· ", Style::default().fg(style::TEXT_MUTED))];
                styled.extend(
                    md_line
                        .spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.fg(style::TEXT_MUTED))),
                );
                rendered.push(RenderedLine {
                    line: Line::from(styled),
                });
            }
            continue;
        }

        if matches!(message.role, MessageRole::Decision) {
            tool_call_lines(&mut rendered, message, content_width);
            continue;
        }

        // User or Agent message
        let is_user = matches!(message.role, MessageRole::User);
        let bar_color = if is_user { style::GREEN } else { style::BLUE };
        let bar_char = "┃";

        let result = render_md(&message.content, body_width);

        // Render all lines — tables are formatted as text lines with `│`
        // column separators so they wrap naturally via Paragraph::Wrap.
        let mut next_table_idx = 0;
        for md_line in result.lines {
            if let Some(ti) = md_line.table_ref {
                // First occurrence of this table — emit formatted text lines
                if ti == next_table_idx {
                    let td = &result.tables[ti];
                    for row in table_as_text_lines(td, &bar_char, bar_color) {
                        rendered.push(RenderedLine { line: row });
                    }
                    next_table_idx += 1;
                }
                continue;
            }
            let mut styled = vec![Span::styled(bar_char, Style::default().fg(bar_color))];
            styled.extend(md_line.spans.into_iter().map(|s| Span::styled(s.content, s.style)));
            rendered.push(RenderedLine {
                line: Line::from(styled),
            });
        }
    }

    if rendered.is_empty() {
        rendered.push(RenderedLine {
            line: Line::from("No messages yet."),
        });
    }

    ChatRenderOutput { rendered }
}

// ═══════════════════════════════════════════════════════════════════════════
//  MdLine & MdRenderResult — intermediate representation
// ═══════════════════════════════════════════════════════════════════════════

/// A rendered markdown line with optional table reference.
struct MdLine {
    spans: Vec<Span<'static>>,
    /// If Some, this line is part of the old-style table box-drawing.
    /// We track this so we can replace it with a TableDef.
    table_ref: Option<usize>,
}

/// Holds both text lines and extracted tables from markdown rendering.
struct MdRenderResult {
    lines: Vec<MdLine>,
    tables: Vec<TableDef>,
}

impl MdRenderResult {
    fn new() -> Self {
        MdRenderResult {
            lines: Vec::new(),
            tables: Vec::new(),
        }
    }

    fn push_line(&mut self, spans: Vec<Span<'static>>) {
        self.lines.push(MdLine { spans, table_ref: None });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Markdown renderer (table-aware)
// ═══════════════════════════════════════════════════════════════════════════

fn render_md(text: &str, body_width: usize) -> MdRenderResult {
    let mut result = MdRenderResult::new();

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = pulldown_cmark::Parser::new_ext(text, opts);
    let mut events = parser.peekable();

    while let Some(ev) = events.next() {
        match ev {
            Event::Start(Tag::Paragraph) => {
                let spans = collect_inline_spans(&mut events);
                let plain: String = spans.iter().map(|s| s.content.as_ref()).collect();
                if looks_like_table_row(&plain) {
                    let block = collect_paragraph_table_block(&plain, &mut events, body_width);
                    if block.len() >= 2 {
                        extract_table_from_block(&block, &mut result);
                    } else {
                        wrap_spans_into(&mut result, &spans, body_width, "  ");
                    }
                } else {
                    wrap_spans_into(&mut result, &spans, body_width, "  ");
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let spans = collect_inline_spans(&mut events);
                render_heading_into(&mut result, &spans, level, body_width);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let code_lines = collect_code_lines(&mut events);
                flush_code_block_into(&mut result, &lang, &code_lines);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let inner = collect_blockquote_into(&mut events, body_width);
                result.lines.extend(inner);
            }
            Event::Start(Tag::List(_)) => {
                render_list_into(&mut events, &mut result, body_width);
            }
            Event::Rule => {
                render_hr_into(&mut result, body_width);
            }
            Event::Start(Tag::Table(_alignments)) => {
                render_table_into(&mut events, &mut result, body_width);
            }
            _ => {}
        }
    }

    result
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tool call rendering
// ═══════════════════════════════════════════════════════════════════════════
/// Render a tool-call message.  The content uses the format:
///   `tool_name` or `tool_name — args`
/// Newlines separate multiple calls in the same message (streaming).
///
/// Render a tool-call message.  The content uses `name — args` format.
/// Newlines serve TWO purposes:
/// 1. Multiple tool calls in one message (streaming): `call1\ncall2`
/// 2. Multi-line args in one call: `name — line1\nargs line2`
///
/// Lines with ` — ` start a new tool call; subsequent lines without ` — `
/// are continuation of the previous call's args.
///
/// Each call renders as a simple indented block (no box-drawing characters)
/// so that every line wraps naturally via Paragraph::Wrap:
///
///   read_file           ← purple bold
///     path=/foo.txt     ← gray, 2-space indent
///     count=42
///
fn tool_call_lines(lines: &mut Vec<RenderedLine>, message: &crate::tui::state::ChatMessage, _box_width: usize) {
    let content = &message.content;

    // ── Parse content into tool-call groups ──
    struct Call {
        name: String,
        args: Vec<String>,
    }
    let mut calls: Vec<Call> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if let Some(last) = calls.last_mut() {
                last.args.push(String::new());
            }
            continue;
        }

        if let Some(pos) = trimmed.find(" — ") {
            calls.push(Call {
                name: trimmed[..pos].trim().to_string(),
                args: vec![trimmed[pos + 5..].to_string()],
            });
        } else if let Some(last) = calls.last_mut() {
            last.args.push(trimmed.to_string());
        }
    }

    // ── Emit lines for each call ──
    for call in &calls {
        // Tool name (purple bold)
        lines.push(RenderedLine {
            line: Line::from(Span::styled(
                call.name.clone(),
                Style::default()
                    .fg(style::PURPLE)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            )),
        });

        // Args (gray, indented)
        for arg_line in &call.args {
            if arg_line.is_empty() {
                lines.push(RenderedLine {
                    line: Line::from(Span::raw("")),
                });
            } else {
                lines.push(RenderedLine {
                    line: Line::from(Span::styled(
                        format!("  {}", arg_line),
                        Style::default().fg(style::TEXT_MUTED),
                    )),
                });
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Paragraph table detection & extraction
// ═══════════════════════════════════════════════════════════════════════════

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("```") {
        return false;
    }
    let pipe_count = trimmed.matches('|').count();
    pipe_count >= 2
}

fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 || !trimmed.contains('-') {
        return false;
    }
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2 {
        trimmed[1..trimmed.len() - 1].trim()
    } else if let Some(after_start) = trimmed.strip_prefix('|') {
        after_start.trim()
    } else if let Some(after_end) = trimmed.strip_suffix('|') {
        after_end.trim()
    } else {
        trimmed
    };
    inner
        .split('|')
        .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':' || c == ' '))
        && inner.contains('-')
}

fn collect_paragraph_table_block<'a, I>(
    first_line: &str,
    events: &mut std::iter::Peekable<I>,
    _body_width: usize,
) -> Vec<String>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut block = vec![first_line.to_string()];
    while let Some(ev) = events.peek() {
        match ev {
            Event::SoftBreak | Event::HardBreak => {
                events.next();
            }
            Event::Text(t) => {
                let next_line = t.to_string();
                if looks_like_table_row(&next_line) || is_separator_row(&next_line) {
                    block.push(next_line);
                    events.next();
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    block
}

/// Extract a table from a paragraph-style table block and add to result.
fn extract_table_from_block(block: &[String], result: &mut MdRenderResult) {
    let rows: Vec<Vec<String>> = block
        .iter()
        .map(|line| {
            line.trim()
                .trim_start_matches('|')
                .trim_end_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();

    if rows.is_empty() {
        return;
    }

    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return;
    }

    // Find separator row index
    let sep_idx = rows.iter().position(|r| {
        r.iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ') && c.contains('-'))
    });

    let (header, data_rows) = if let Some(si) = sep_idx {
        let hdr = rows.first().cloned().unwrap_or_default();
        let data: Vec<Vec<String>> = rows.iter().skip(si + 1).cloned().collect();
        (hdr, data)
    } else {
        // No separator: treat first row as header, rest as data
        let hdr = rows.first().cloned().unwrap_or_default();
        let data: Vec<Vec<String>> = rows.iter().skip(1).cloned().collect();
        (hdr, data)
    };

    // Compute column widths based on display width (no hardcoded limits)
    let body_avail = 80; // reasonable max table width
    let max_col = (body_avail / col_count).saturating_sub(2).max(4);
    let col_widths: Vec<usize> = (0..col_count)
        .map(|ci| {
            let all_cells: Vec<&str> = header
                .get(ci)
                .into_iter()
                .chain(data_rows.iter().filter_map(|r| r.get(ci)))
                .map(|s| s.as_str())
                .collect();
            let max_width = all_cells.iter().map(|c| UnicodeWidthStr::width(*c)).max().unwrap_or(4);
            max_width.min(max_col).max(4)
        })
        .collect();

    let td = TableDef {
        start_line: 0, // will be set later
        end_line: 0,
        header,
        rows: data_rows,
        col_widths,
    };

    let table_idx = result.tables.len();
    result.tables.push(td);

    // Push old-style box-drawing lines as placeholders (they'll be replaced)
    let total_h = compute_table_height(&result.tables[table_idx]);
    for _ in 0..total_h {
        result.lines.push(MdLine {
            spans: vec![Span::raw("")],
            table_ref: Some(table_idx),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Table rendering (pulldown-cmark parsed tables → TableDef)
// ═══════════════════════════════════════════════════════════════════════════

fn render_table_into<'a, I>(events: &mut std::iter::Peekable<I>, result: &mut MdRenderResult, body_width: usize)
where
    I: Iterator<Item = Event<'a>>,
{
    // Collect rows as plain strings (inline formatting stripped for Table widget)
    let mut rows: Vec<Vec<String>> = Vec::new();

    loop {
        match events.next() {
            Some(Event::Start(Tag::Table(_))) => {}
            Some(Event::Start(Tag::TableHead)) => {
                let mut cells: Vec<String> = Vec::new();
                loop {
                    match events.next() {
                        Some(Event::Start(Tag::TableCell)) => {
                            let spans = collect_inline_spans(events);
                            let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                            cells.push(text);
                        }
                        Some(Event::End(TagEnd::TableHead)) => break,
                        None => break,
                        _ => {}
                    }
                }
                if !cells.is_empty() {
                    rows.push(cells);
                }
            }
            Some(Event::Start(Tag::TableRow)) => {
                let mut cells: Vec<String> = Vec::new();
                loop {
                    match events.next() {
                        Some(Event::Start(Tag::TableCell)) => {
                            let spans = collect_inline_spans(events);
                            let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                            cells.push(text);
                        }
                        Some(Event::End(TagEnd::TableRow)) => break,
                        Some(Event::End(TagEnd::Table)) => break,
                        None => break,
                        _ => {}
                    }
                }
                rows.push(cells);
            }
            Some(Event::End(TagEnd::Table)) => break,
            None => break,
            _ => {}
        }
    }

    if rows.is_empty() {
        return;
    }

    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return;
    }

    // Compute column widths using display width (no hardcoded 80/50 limits)
    let max_col = (body_width / col_count).saturating_sub(2).clamp(4, 60);
    let col_widths: Vec<usize> = (0..col_count)
        .map(|ci| {
            let all_cells: Vec<&str> = rows.iter().filter_map(|r| r.get(ci)).map(|s| s.as_str()).collect();
            let max_w = all_cells.iter().map(|c| UnicodeWidthStr::width(*c)).max().unwrap_or(4);
            max_w.min(max_col).max(4)
        })
        .collect();

    // First row is header (from TableHead), rest are data
    let header = rows.first().cloned().unwrap_or_default();
    let data_rows: Vec<Vec<String>> = rows.iter().skip(1).cloned().collect();

    let td = TableDef {
        start_line: 0,
        end_line: 0,
        header,
        rows: data_rows,
        col_widths,
    };

    let table_idx = result.tables.len();
    result.tables.push(td);

    // Emit placeholder lines
    let total_h = compute_table_height(&result.tables[table_idx]);
    for _ in 0..total_h {
        result.lines.push(MdLine {
            spans: vec![Span::raw("")],
            table_ref: Some(table_idx),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Inline span collection & styling
// ═══════════════════════════════════════════════════════════════════════════

struct InlineStyle {
    bold: u32,
    italic: u32,
    strike: u32,
    fg: Vec<ratatui::style::Color>,
}

impl InlineStyle {
    fn new() -> Self {
        Self {
            bold: 0,
            italic: 0,
            strike: 0,
            fg: Vec::new(),
        }
    }

    fn current_style(&self) -> Style {
        let mut s = Style::default();
        if self.bold > 0 {
            s = s.add_modifier(Modifier::BOLD);
        }
        if self.italic > 0 {
            s = s.add_modifier(Modifier::ITALIC);
        }
        if self.strike > 0 {
            s = s.add_modifier(Modifier::CROSSED_OUT);
        }
        if let Some(c) = self.fg.last() {
            s = s.fg(*c);
        }
        s
    }
}

fn collect_inline_spans<'a, I>(events: &mut std::iter::Peekable<I>) -> Vec<Span<'static>>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut istyle = InlineStyle::new();

    loop {
        match events.next() {
            Some(Event::Text(t)) => {
                buf.push_str(&t);
            }
            Some(Event::Code(t)) => {
                flush_buf(&mut spans, &mut buf, &istyle);
                spans.push(Span::styled(
                    format!("`{}`", t),
                    Style::default().fg(style::GREEN).add_modifier(Modifier::ITALIC),
                ));
            }
            Some(Event::Start(tag)) => {
                flush_buf(&mut spans, &mut buf, &istyle);
                match tag {
                    Tag::Emphasis => istyle.italic += 1,
                    Tag::Strong => istyle.bold += 1,
                    Tag::Strikethrough => istyle.strike += 1,
                    Tag::Link { .. } => {
                        istyle.fg.push(style::BLUE);
                    }
                    _ => {}
                }
            }
            Some(Event::End(tag_end)) => {
                flush_buf(&mut spans, &mut buf, &istyle);
                match tag_end {
                    TagEnd::Emphasis => istyle.italic = istyle.italic.saturating_sub(1),
                    TagEnd::Strong => istyle.bold = istyle.bold.saturating_sub(1),
                    TagEnd::Strikethrough => istyle.strike = istyle.strike.saturating_sub(1),
                    TagEnd::Link => {
                        istyle.fg.pop();
                    }
                    TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => {
                        flush_buf(&mut spans, &mut buf, &istyle);
                        break;
                    }
                    TagEnd::Item => {
                        flush_buf(&mut spans, &mut buf, &istyle);
                        break;
                    }
                    _ => {}
                }
            }
            Some(Event::SoftBreak | Event::HardBreak) => {
                buf.push(' ');
            }
            None => break,
            _ => {}
        }
    }

    flush_buf(&mut spans, &mut buf, &istyle);
    spans
}

fn flush_buf(spans: &mut Vec<Span<'static>>, buf: &mut String, istyle: &InlineStyle) {
    if !buf.is_empty() {
        let s = istyle.current_style();
        spans.push(Span::styled(std::mem::take(buf), s));
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Code block
// ═══════════════════════════════════════════════════════════════════════════

fn collect_code_lines<'a, I>(events: &mut I) -> Vec<String>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut lines = Vec::new();
    let mut line = String::new();
    loop {
        match events.next() {
            Some(Event::Text(t)) => line.push_str(&t),
            Some(Event::End(TagEnd::CodeBlock)) => {
                if !line.is_empty() {
                    lines.push(line);
                }
                break;
            }
            Some(Event::SoftBreak | Event::HardBreak) => {
                lines.push(std::mem::take(&mut line));
            }
            None => break,
            _ => {}
        }
    }
    lines
}

fn flush_code_block_into(result: &mut MdRenderResult, lang: &str, code_lines: &[String]) {
    if !lang.is_empty() {
        result.push_line(vec![
            Span::raw("  "),
            Span::styled(format!("▐ {} ", lang), Style::default().fg(style::TEXT_MUTED)),
        ]);
    } else {
        result.push_line(vec![
            Span::raw("  "),
            Span::styled("▐", Style::default().fg(style::TEXT_MUTED)),
        ]);
    }
    for code_line in code_lines {
        result.push_line(vec![
            Span::styled("▐ ", Style::default().fg(style::TEXT_MUTED)),
            Span::styled(code_line.clone(), Style::default().fg(style::TEXT_PRIMARY)),
        ]);
    }
    result.push_line(vec![
        Span::raw("  "),
        Span::styled("▐", Style::default().fg(style::TEXT_MUTED)),
    ]);
}

// ═══════════════════════════════════════════════════════════════════════════
//  Blockquote
// ═══════════════════════════════════════════════════════════════════════════

fn collect_blockquote_into<'a, I>(events: &mut std::iter::Peekable<I>, body_width: usize) -> Vec<MdLine>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut out = Vec::new();
    let mut depth = 1u32;
    loop {
        match events.peek() {
            Some(Event::Start(Tag::BlockQuote(_))) => {
                depth += 1;
                let _ = events.next();
            }
            Some(Event::End(TagEnd::BlockQuote(_))) => {
                depth -= 1;
                let _ = events.next();
                if depth == 0 {
                    break;
                }
            }
            Some(Event::Start(Tag::Paragraph)) => {
                let _ = events.next();
                let spans = collect_inline_spans(events);
                let mut quote_spans = vec![Span::styled("│ ", Style::default().fg(style::YELLOW))];
                for s in &spans {
                    let mut merged = s.style;
                    if merged.fg.is_none() {
                        merged = merged.fg(style::YELLOW);
                    }
                    quote_spans.push(Span::styled(s.content.clone(), merged));
                }
                wrap_spans_no_indent(&mut out, &quote_spans, body_width, "  ");
            }
            _ => {
                let _ = events.next();
            }
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
//  List
// ═══════════════════════════════════════════════════════════════════════════

fn render_list_into<'a, I>(events: &mut std::iter::Peekable<I>, result: &mut MdRenderResult, body_width: usize)
where
    I: Iterator<Item = Event<'a>>,
{
    let mut list_depth = 1u32;

    loop {
        match events.next() {
            Some(Event::Start(Tag::Item)) => {
                let item_spans = collect_inline_spans(events);
                let bullet = match list_depth {
                    1 => "•",
                    2 => "◦",
                    3 => "▪",
                    _ => "▸",
                };
                let indent = "  ".repeat((list_depth - 1) as usize);
                let prefix = format!("{}{} ", indent, bullet);
                let mut prefix_spans = vec![Span::styled(prefix, Style::default().fg(style::BLUE))];
                prefix_spans.extend(item_spans.iter().cloned());
                wrap_spans_into(result, &prefix_spans, body_width, "");
            }
            Some(Event::End(TagEnd::List(_))) => {
                list_depth -= 1;
                if list_depth == 0 {
                    break;
                }
            }
            Some(Event::Start(Tag::List(_))) => {
                list_depth += 1;
            }
            None => break,
            _ => {}
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Heading
// ═══════════════════════════════════════════════════════════════════════════

fn render_heading_into(result: &mut MdRenderResult, spans: &[Span<'static>], level: HeadingLevel, body_width: usize) {
    let level_num: u8 = match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        _ => 3,
    };
    let header_color = match level_num {
        1 => style::YELLOW,
        2 => style::BLUE,
        _ => style::TEXT_MUTED,
    };
    let prefix = format!("  {} ", "#".repeat(level_num as usize));
    let styled_spans: Vec<Span<'static>> = spans
        .iter()
        .map(|s| {
            let mut merged = s.style;
            if merged.fg.is_none() {
                merged = merged.fg(header_color);
            }
            merged = merged.add_modifier(Modifier::BOLD);
            Span::styled(s.content.clone(), merged)
        })
        .collect();
    let mut prefix_spans = vec![Span::styled(prefix, Style::default().fg(style::TEXT_MUTED))];
    prefix_spans.extend(styled_spans);
    wrap_spans_into(result, &prefix_spans, body_width, "");
}

// ═══════════════════════════════════════════════════════════════════════════
//  Horizontal rule
// ═══════════════════════════════════════════════════════════════════════════

fn render_hr_into(result: &mut MdRenderResult, body_width: usize) {
    let hr_width = body_width.min(40);
    result.push_line(vec![
        Span::raw("  "),
        Span::styled("─".repeat(hr_width), Style::default().fg(style::TEXT_MUTED)),
    ]);
}

// ═══════════════════════════════════════════════════════════════════════════
//  Word-wrap with display‑width awareness (fixes CJK wrapping)
// ═══════════════════════════════════════════════════════════════════════════

/// Wraps styled spans, pushing lines to `MdRenderResult`.
fn wrap_spans_into(result: &mut MdRenderResult, spans: &[Span<'static>], max_width: usize, indent: &str) {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = full_text.chars().collect();
    let indent_w = UnicodeWidthStr::width(indent);

    if chars.is_empty() {
        return;
    }

    let avail = max_width.saturating_sub(indent_w);

    // Use display width (not char count) for wrap detection
    let total_display_width = UnicodeWidthStr::width(full_text.as_str());
    if total_display_width <= avail {
        let mut row = vec![Span::raw(indent.to_string())];
        for s in spans {
            row.push(Span::styled(s.content.clone(), s.style));
        }
        result.push_line(row);
        return;
    }

    let code_ranges = find_code_span_ranges(spans);

    let mut start = 0usize;
    let len = chars.len();
    let char_to_display: Vec<usize> = chars.iter().map(|c| UnicodeWidthChar::width(*c).unwrap_or(0)).collect();

    while start < len {
        // Measure display width from start to end of available space
        let mut display_so_far = 0usize;
        let mut end = start;
        while end < len {
            let cw = char_to_display[end];
            if display_so_far + cw > avail {
                break;
            }
            display_so_far += cw;
            end += 1;
        }

        if end >= len {
            // Remaining fits
            let mut row = vec![Span::raw(indent.to_string())];
            row.extend(make_spans_for_range(spans, start, len));
            result.push_line(row);
            break;
        }

        // Try to break at a space (avoid breaking inside code spans)
        let break_point = if let Some(space_pos) = chars[start..end].iter().rposition(|&c| c == ' ') {
            let real_pos = start + space_pos;
            if !is_inside_code_span(real_pos, &code_ranges) {
                Some(real_pos)
            } else {
                // Find earlier space not inside code
                let mut found = None;
                for i in (0..space_pos).rev() {
                    if chars[start + i] == ' ' && !is_inside_code_span(start + i, &code_ranges) {
                        found = Some(start + i);
                        break;
                    }
                }
                found
            }
        } else {
            None
        };

        let actual_end = break_point.unwrap_or(end);

        let mut row = vec![Span::raw(indent.to_string())];
        row.extend(make_spans_for_range(spans, start, actual_end));
        result.push_line(row);

        start = actual_end;
        // Skip trailing spaces
        while start < len && chars[start] == ' ' {
            start += 1;
        }
    }
}

/// Like wrap_spans_into but returns Vec<MdLine> (used by blockquote).
fn wrap_spans_no_indent(out: &mut Vec<MdLine>, spans: &[Span<'static>], max_width: usize, indent: &str) {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = full_text.chars().collect();
    let indent_w = UnicodeWidthStr::width(indent);

    if chars.is_empty() {
        return;
    }

    let avail = max_width.saturating_sub(indent_w);

    let total_dw = UnicodeWidthStr::width(full_text.as_str());
    if total_dw <= avail {
        let mut row = vec![Span::raw(indent.to_string())];
        for s in spans {
            row.push(Span::styled(s.content.clone(), s.style));
        }
        out.push(MdLine {
            spans: row,
            table_ref: None,
        });
        return;
    }

    let code_ranges = find_code_span_ranges(spans);
    let char_to_display: Vec<usize> = chars.iter().map(|c| UnicodeWidthChar::width(*c).unwrap_or(0)).collect();

    let mut start = 0usize;
    let len = chars.len();

    while start < len {
        let mut display_so_far = 0usize;
        let mut end = start;
        while end < len {
            let cw = char_to_display[end];
            if display_so_far + cw > avail {
                break;
            }
            display_so_far += cw;
            end += 1;
        }

        if end >= len {
            let mut row = vec![Span::raw(indent.to_string())];
            row.extend(make_spans_for_range(spans, start, len));
            out.push(MdLine {
                spans: row,
                table_ref: None,
            });
            break;
        }

        let break_point = if let Some(space_pos) = chars[start..end].iter().rposition(|&c| c == ' ') {
            let real_pos = start + space_pos;
            if !is_inside_code_span(real_pos, &code_ranges) {
                Some(real_pos)
            } else {
                let mut found = None;
                for i in (0..space_pos).rev() {
                    if chars[start + i] == ' ' && !is_inside_code_span(start + i, &code_ranges) {
                        found = Some(start + i);
                        break;
                    }
                }
                found
            }
        } else {
            None
        };

        let actual_end = break_point.unwrap_or(end);

        let mut row = vec![Span::raw(indent.to_string())];
        row.extend(make_spans_for_range(spans, start, actual_end));
        out.push(MdLine {
            spans: row,
            table_ref: None,
        });

        start = actual_end;
        while start < len && chars[start] == ' ' {
            start += 1;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Code span helpers (for wrap logic)
// ═══════════════════════════════════════════════════════════════════════════

fn find_code_span_ranges(spans: &[Span<'static>]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut pos = 0;
    for s in spans {
        let len = s.content.chars().count();
        let is_code = s.style.fg == Some(style::GREEN) && s.style.add_modifier.contains(Modifier::ITALIC);
        if is_code {
            ranges.push((pos, pos + len));
        }
        pos += len;
    }
    ranges
}

fn is_inside_code_span(pos: usize, ranges: &[(usize, usize)]) -> bool {
    ranges.iter().any(|&(start, end)| pos > start && pos < end)
}

fn make_spans_for_range(all: &[Span<'static>], start: usize, end: usize) -> Vec<Span<'static>> {
    if start >= end {
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut pos = 0usize;
    for s in all {
        let span_len = s.content.chars().count();
        let span_start = pos;
        let span_end = pos + span_len;
        if span_end <= start {
            pos = span_end;
            continue;
        }
        if span_start >= end {
            break;
        }
        let char_start = start.saturating_sub(span_start);
        let char_end = if end < span_end { end - span_start } else { span_len };
        if char_start < char_end {
            let sub: String = s.content.chars().skip(char_start).take(char_end - char_start).collect();
            if !sub.is_empty() {
                result.push(Span::styled(sub, s.style));
            }
        }
        pos = span_end;
    }
    result
}

// ═══════════════════════════════════════════════════════════════════════════
//  Legacy helpers (test-only)
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
fn truncate_display(s: &str, max_display_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > max_display_width {
            break;
        }
        width += cw;
        result.push(ch);
    }
    result
}

pub(crate) fn char_idx_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte_idx, _)| byte_idx)
        .unwrap_or(s.len())
}

pub(crate) fn display_width_up_to(s: &str, char_idx: usize) -> usize {
    s.chars()
        .take(char_idx)
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── Wrap / display width tests ──

    #[test]
    fn test_wrap_spans_short() {
        let mut result = MdRenderResult::new();
        let spans = vec![Span::raw("hello world")];
        wrap_spans_into(&mut result, &spans, 80, "");
        assert_eq!(result.lines.len(), 1);
    }

    #[test]
    fn test_wrap_spans_cjk_fits() {
        // 8 CJK chars × 2 = 16 display width, avail = 20 → fits
        let mut result = MdRenderResult::new();
        let spans = vec![Span::raw("你好世界测试")];
        wrap_spans_into(&mut result, &spans, 20, "  ");
        assert_eq!(result.lines.len(), 1, "CJK should fit in 20 cols");
    }

    #[test]
    fn test_wrap_spans_cjk_wraps() {
        // 12 CJK chars × 2 = 24 display width, avail = 10 → wraps
        let mut result = MdRenderResult::new();
        let spans = vec![Span::raw("你好世界测试中文")];
        wrap_spans_into(&mut result, &spans, 12, "  ");
        assert!(
            result.lines.len() >= 2,
            "CJK should wrap: got {} lines",
            result.lines.len()
        );
        // Each line should be at most 12 display cols
        for (i, line) in result.lines.iter().enumerate() {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let dw = UnicodeWidthStr::width(text.as_str());
            assert!(dw <= 12, "Line {}: {} display width exceeds 12", i, dw);
        }
    }

    #[test]
    fn test_wrap_spans_mixed_cjk() {
        // Mix of ASCII and CJK
        let mut result = MdRenderResult::new();
        let spans = vec![Span::raw("hello 你好 world 世界 test")];
        wrap_spans_into(&mut result, &spans, 16, "");
        assert!(result.lines.len() >= 2, "Mixed should wrap");
    }

    #[test]
    fn test_wrap_spans_empty() {
        let mut result = MdRenderResult::new();
        wrap_spans_into(&mut result, &[], 80, "");
        assert!(result.lines.is_empty());
    }

    // ── Table extraction tests ──

    #[test]
    fn test_extract_table_simple() {
        let block = vec![
            "| Name | Value |".to_string(),
            "| --- | --- |".to_string(),
            "| A | 1 |".to_string(),
            "| B | 2 |".to_string(),
        ];
        let mut result = MdRenderResult::new();
        extract_table_from_block(&block, &mut result);

        assert_eq!(result.tables.len(), 1);
        let td = &result.tables[0];
        assert_eq!(td.header, vec!["Name", "Value"]);
        assert_eq!(td.rows.len(), 2);
        assert_eq!(td.rows[0], vec!["A", "1"]);
    }

    #[test]
    fn test_extract_table_no_separator() {
        let block = vec![
            "| Name | Value |".to_string(),
            "| A | 1 |".to_string(),
            "| B | 2 |".to_string(),
        ];
        let mut result = MdRenderResult::new();
        extract_table_from_block(&block, &mut result);

        assert_eq!(result.tables.len(), 1);
        let td = &result.tables[0];
        assert_eq!(td.header, vec!["Name", "Value"]);
        assert_eq!(td.rows.len(), 2);
    }

    #[test]
    fn test_extract_table_empty() {
        let mut result = MdRenderResult::new();
        extract_table_from_block(&[], &mut result);
        assert!(result.tables.is_empty());
    }

    #[test]
    fn test_extract_table_single_row() {
        let block = vec!["| Name |".to_string(), "| --- |".to_string()];
        let mut result = MdRenderResult::new();
        extract_table_from_block(&block, &mut result);
        assert_eq!(result.tables.len(), 1);
        assert!(result.tables[0].rows.is_empty());
    }

    // ── table height ──

    #[test]
    fn test_compute_table_height() {
        let td = TableDef {
            start_line: 0,
            end_line: 0,
            header: vec!["A".to_string()],
            rows: vec![vec!["1".to_string()], vec!["2".to_string()]],
            col_widths: vec![4],
        };
        assert_eq!(compute_table_height(&td), 5); // top + header + sep + 2 rows + bottom = 5
    }

    // ── TableDef → table_as_text_lines builds without panic ──

    #[test]
    fn test_table_as_text_lines() {
        let td = TableDef {
            start_line: 0,
            end_line: 5,
            header: vec!["Name".to_string(), "Value".to_string()],
            rows: vec![
                vec!["Alice".to_string(), "100".to_string()],
                vec!["Bob".to_string(), "200".to_string()],
            ],
            col_widths: vec![6, 6],
        };
        let output = ChatRenderOutput { rendered: vec![] };
        // Tables are now rendered as text lines — verify table_as_text_lines
        let lines = table_as_text_lines(&td, "┃", crate::tui::style::TEXT_PRIMARY);
        assert!(!lines.is_empty(), "table_as_text_lines should produce output");
        let _ = output;
    }

    // ── render_md basic tests ──

    #[test]
    fn test_render_md_empty() {
        let result = render_md("", 80);
        assert!(result.lines.is_empty());
    }

    #[test]
    fn test_render_md_bold() {
        let result = render_md("**bold**", 80);
        assert!(!result.lines.is_empty());
    }

    #[test]
    fn test_render_md_code_block() {
        let result = render_md("```rust\nfn main() {}\n```", 80);
        assert_eq!(result.lines.len(), 3); // header + code + footer
    }

    #[test]
    fn test_render_md_table() {
        let md = "| Name | Value |\n| --- | --- |\n| A | 1 |\n| B | 2 |";
        let result = render_md(md, 80);
        assert!(!result.tables.is_empty(), "should extract a TableDef");
        let td = &result.tables[0];
        assert_eq!(td.header, vec!["Name", "Value"]);
        assert_eq!(td.rows.len(), 2);
    }

    #[test]
    fn test_render_md_table_no_leading_pipe() {
        let md = "Name | Value\n--- | ---\nA | 1\nB | 2";
        let result = render_md(md, 80);
        assert!(
            !result.tables.is_empty(),
            "should extract a TableDef even without leading pipe"
        );
    }

    #[test]
    fn test_separator_row_detection() {
        assert!(is_separator_row("--- | ---"));
        assert!(is_separator_row("| --- | --- |"));
        assert!(is_separator_row("|:---|:---:|"));
        assert!(!is_separator_row("| Name | Value |"));
        assert!(!is_separator_row("just text"));
    }

    #[test]
    fn test_looks_like_table_row() {
        assert!(looks_like_table_row("| a | b |"));
        assert!(looks_like_table_row("a | b | c"));
        assert!(!looks_like_table_row("a | b"));
        assert!(!looks_like_table_row("just text"));
        assert!(!looks_like_table_row(""));
    }

    #[test]
    fn test_truncate_display_basic() {
        assert_eq!(truncate_display("hello", 3), "hel");
        assert_eq!(truncate_display("hi", 10), "hi");
        assert_eq!(truncate_display("", 5), "");
    }

    #[test]
    fn test_truncate_display_cjk() {
        assert_eq!(truncate_display("你好世界", 4), "你好");
        assert_eq!(truncate_display("你好世界", 2), "你");
        assert_eq!(truncate_display("你好世界", 1), "");
    }

    // ── build_chat_content integration tests ──

    #[test]
    fn test_build_chat_content_no_messages() {
        let mut state = CoreState::default();
        state.messages.clear();
        let output = build_chat_content(&state, 80, 0);
        assert_eq!(output.total_lines(), 1);
        assert!(output.rendered[0].line.to_string().contains("No messages"));
    }

    #[test]
    fn test_build_chat_content_with_table() {
        use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
        let mut state = CoreState::default();
        state.messages = vec![ChatMessage {
            role: MessageRole::User,
            content: "Show me data\n\n| Name | Value |\n| --- | --- |\n| A | 1 |\n| B | 2 |\n".to_string(),
            timestamp: "00:00".to_string(),
            status: MessageStatus::Completed,
        }];
        let output = build_chat_content(&state, 80, 0);
        // Tables are rendered as text lines — verify pipe characters exist
        let all_text: String = output.rendered.iter().map(|r| r.line.to_string()).collect::<Vec<_>>().join("\n");
        assert!(all_text.contains("Name"), "table header should appear in rendered output");
        assert!(all_text.contains("Value"), "table header should appear in rendered output");
    }
}
