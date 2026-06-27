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
    /// If this line is part of a table region, the table definition.
    pub table: Option<TableDef>,
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

/// Compute how many terminal lines a table occupies.
fn compute_table_height(td: &TableDef) -> usize {
    3 + td.rows.len()
}

impl ChatRenderOutput {
    pub fn total_lines(&self) -> usize {
        self.rendered.len()
    }

    /// Compute the scroll position for the last `visible_height` physical rows,
    /// accounting for word wrapping and multi-row tables.
    ///
    /// Returns the logical line index such that rendering from there fills
    /// at most `visible_height` terminal rows.  This is the correct `max_scroll`
    /// for auto-scroll and scroll clamping — unlike `total - visible_height`
    /// which mixes logical and physical units.
    pub(crate) fn max_scroll_for_height(&self, visible_height: usize, avail: usize) -> usize {
        if self.rendered.is_empty() || visible_height == 0 {
            return 0;
        }
        let mut phys = 0usize;
        for i in (0..self.rendered.len()).rev() {
            let h = if let Some(ref td) = self.rendered[i].table {
                compute_table_height(td)
            } else {
                let w = self.rendered[i].line.width();
                if w == 0 { 1 } else { w.div_ceil(avail) }
            };
            phys += h;
            if phys > visible_height {
                return i;
            }
        }
        0
    }

    /// Build a ratatui Table widget for a given table definition.
    /// Cell content is pre-wrapped to column widths so that long text flows
    /// onto multiple lines inside the table.
    pub(crate) fn build_table_widget(&self, table: &TableDef) -> Table<'static> {
        let col_constraints: Vec<Constraint> = table
            .col_widths
            .iter()
            .map(|&w| Constraint::Length(w as u16 + 2))
            .collect();

        // ── Pre-wrap cell content and compute row heights ──
        fn wrapped_lines(text: &str, width: usize) -> Vec<String> {
            use unicode_width::UnicodeWidthStr;
            let inner = width.max(1);
            let mut out = Vec::new();
            for line in text.lines() {
                let w = UnicodeWidthStr::width(line);
                if w <= inner {
                    out.push(line.to_string());
                } else {
                    let mut start = 0;
                    let chars: Vec<char> = line.chars().collect();
                    while start < chars.len() {
                        let mut end = start;
                        let mut dw = 0;
                        while end < chars.len()
                            && dw + UnicodeWidthChar::width(chars[end]).unwrap_or(0) <= inner
                        {
                            dw += UnicodeWidthChar::width(chars[end]).unwrap_or(0);
                            end += 1;
                        }
                        if end == start {
                            end = start + 1;
                        }
                        out.push(chars[start..end].iter().collect());
                        start = end;
                    }
                }
            }
            out
        }

        // Header row (no wrapping needed, but use a single Line)
        let header_cells: Vec<Cell> = table
            .header
            .iter()
            .map(|h| {
                Cell::from(Text::from(Line::from(Span::styled(
                    format!(" {} ", h),
                    Style::default()
                        .fg(style::BLUE)
                        .add_modifier(Modifier::BOLD),
                ))))
            })
            .collect();
        let header_row = Row::new(header_cells).style(Style::default().bg(style::BG_SECONDARY));

        let data_rows: Vec<Row> = table
            .rows
            .iter()
            .map(|row| {
                let mut max_h = 1usize;
                let cells: Vec<Cell> = row
                    .iter()
                    .zip(table.col_widths.iter())
                    .map(|(c, &col_w)| {
                        let lines = wrapped_lines(c, col_w);
                        max_h = max_h.max(lines.len());
                        let text_lines: Vec<Line<'static>> = lines
                            .into_iter()
                            .map(|l| {
                                Line::from(Span::styled(
                                    format!(" {} ", l),
                                    Style::default().fg(style::TEXT_PRIMARY),
                                ))
                            })
                            .collect();
                        Cell::from(Text::from(text_lines))
                    })
                    .collect();
                Row::new(cells).height(max_h as u16)
            })
            .collect();

        Table::new(data_rows, col_constraints)
            .header(header_row)
            .column_spacing(0)
            .style(Style::default())
            .row_highlight_style(Style::default().bg(style::BG_SECONDARY))
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Main entry point
// ═══════════════════════════════════════════════════════════════════════════

pub(crate) fn build_chat_content(
    state: &CoreState,
    width: usize,
    think_frame: u8,
    think_level: u8,
) -> ChatRenderOutput {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut rendered: Vec<RenderedLine> = Vec::new();

    for message in &state.messages {
        if matches!(message.role, MessageRole::System) {
            // Render system messages as plain text (not markdown) so that
            // all original line breaks, including blank lines, are preserved.
            // The markdown parser would collapse blank lines between
            // paragraphs, losing intentional formatting.
            // str::lines() correctly handles trailing newlines without
            // producing an extra empty line.
            for line in message.content.lines() {
                let mut styled = vec![Span::styled("· ", Style::default().fg(style::TEXT_MUTED))];
                if !line.is_empty() {
                    styled.push(Span::styled(
                        line.to_string(),
                        Style::default().fg(style::TEXT_MUTED),
                    ));
                }
                rendered.push(RenderedLine {
                    line: Line::from(styled),
                    table: None,
                });
            }
            continue;
        }

        if matches!(message.role, MessageRole::Decision) {
            tool_call_lines(&mut rendered, message, content_width);
            continue;
        }

        // Reasoning/chain-of-thought rendering (respect think_level)
        if think_level > 0 && !message.reasoning.is_empty() {
            let text = if think_level == 1 && message.reasoning.chars().count() > 200 {
                format!(
                    "{}...\n\n_Reasoning truncated (set `/think 2` for full)_",
                    message.reasoning.chars().take(200).collect::<String>()
                )
            } else {
                message.reasoning.clone()
            };
            let result = render_md(&text, body_width);
            for md_line in result.lines {
                let mut styled = vec![Span::styled("┊", Style::default().fg(style::TEXT_MUTED))];
                styled.extend(md_line.spans.into_iter().map(|mut s| {
                    s.style = s.style.add_modifier(Modifier::DIM);
                    s
                }));
                rendered.push(RenderedLine {
                    line: Line::from(styled),
                    table: None,
                });
            }
        }

        // User or Agent message — determine bar color based on role and status
        let is_user = matches!(message.role, MessageRole::User);
        let bar_color = if message.status == crate::tui::state::MessageStatus::Error {
            // Red bar for error messages (tool errors, agent failures, LLM errors)
            style::RED
        } else if is_user {
            style::GREEN
        } else {
            style::BLUE
        };
        let bar_char = "┃";

        // Animated thinking indicator for messages in Thinking state with no content yet.
        if message.status == crate::tui::state::MessageStatus::Thinking
            && message.content.is_empty()
        {
            let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let phase = (think_frame as usize / 2) % spinner.len();
            let mut styled = vec![Span::styled(bar_char, Style::default().fg(bar_color))];
            styled.push(Span::styled(
                format!(" {} thinking…", spinner[phase]),
                Style::default()
                    .fg(style::YELLOW)
                    .add_modifier(Modifier::DIM),
            ));
            rendered.push(RenderedLine {
                line: Line::from(styled),
                table: None,
            });
            continue;
        }

        let result = render_md(&message.content, body_width);

        // Render all lines — tables are formatted as text lines with `│`
        // column separators so they wrap naturally via Paragraph::Wrap.
        // Emit table placeholders and regular lines
        let mut line_idx = rendered.len();
        let mut next_table_idx = 0;

        for md_line in result.lines {
            if let Some(ti) = md_line.table_ref {
                if ti == next_table_idx {
                    let td = &result.tables[ti];
                    let table_h = compute_table_height(td);
                    let table_def = TableDef {
                        start_line: line_idx,
                        end_line: line_idx + table_h,
                        header: td.header.clone(),
                        rows: td.rows.clone(),
                        col_widths: td.col_widths.clone(),
                    };
                    for _ in 0..table_h {
                        rendered.push(RenderedLine {
                            line: Line::from(Span::raw("")),
                            table: Some(table_def.clone()),
                        });
                    }
                    // table_def is embedded in RenderedLine.table above
                    line_idx += table_h;
                    next_table_idx += 1;
                }
                continue;
            }
            let mut styled = vec![Span::styled(bar_char, Style::default().fg(bar_color))];
            styled.extend(
                md_line
                    .spans
                    .into_iter()
                    .map(|s| Span::styled(s.content, s.style)),
            );
            rendered.push(RenderedLine {
                line: Line::from(styled),
                table: None,
            });
            line_idx += 1;
        }
    }

    if rendered.is_empty() {
        rendered.push(RenderedLine {
            line: Line::from("No messages yet."),
            table: None,
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
        self.lines.push(MdLine {
            spans,
            table_ref: None,
        });
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
                let line_groups = collect_inline_spans(&mut events);
                for spans in &line_groups {
                    let plain: String = spans.iter().map(|s| s.content.as_ref()).collect();
                    if looks_like_table_row(&plain) {
                        let block = collect_paragraph_table_block(&plain, &mut events);
                        if block.len() >= 2 {
                            extract_table_from_block(&block, &mut result, body_width);
                        } else if !spans.is_empty() {
                            wrap_spans_into(&mut result, spans, body_width, "  ");
                        }
                    } else if !spans.is_empty() {
                        wrap_spans_into(&mut result, spans, body_width, "  ");
                    }
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let line_groups = collect_inline_spans(&mut events);
                for spans in &line_groups {
                    if !spans.is_empty() {
                        render_heading_into(&mut result, spans, level, body_width);
                    }
                }
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
fn tool_call_lines(
    lines: &mut Vec<RenderedLine>,
    message: &crate::tui::state::ChatMessage,
    _box_width: usize,
) {
    let content = &message.content;

    // Format: <name>\nkey=value\nkey=value\n\n<name>\n...
    struct Call {
        name: String,
        args: Vec<String>,
    }
    let mut calls: Vec<Call> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Lines containing '=' are arg key=value pairs; otherwise tool name.
        if !trimmed.contains('=') {
            calls.push(Call {
                name: trimmed.to_string(),
                args: Vec::new(),
            });
        } else if let Some(last) = calls.last_mut() {
            last.args.push(trimmed.to_string());
        }
    }

    // ── Emit one line per call, args on separate indented lines ──
    for call in calls.iter() {
        // Tool name line
        let name_spans: Vec<Span<'static>> = vec![
            Span::styled("> ", Style::default().fg(style::TEXT_MUTED)),
            Span::styled(
                call.name.clone(),
                Style::default()
                    .fg(style::PURPLE)
                    .add_modifier(Modifier::BOLD),
            ),
        ];
        lines.push(RenderedLine {
            line: Line::from(name_spans),
            table: None,
        });

        // Indented args, one per line
        for arg_line in &call.args {
            let arg_spans: Vec<Span<'static>> = vec![
                Span::styled("  ", Style::default()),
                Span::styled(arg_line.clone(), Style::default().fg(style::TEXT_MUTED)),
            ];
            lines.push(RenderedLine {
                line: Line::from(arg_spans),
                table: None,
            });
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
    inner.split('|').all(|cell| {
        cell.trim()
            .chars()
            .all(|c| c == '-' || c == ':' || c == ' ')
    }) && inner.contains('-')
}

fn collect_paragraph_table_block<'a, I>(
    first_line: &str,
    events: &mut std::iter::Peekable<I>,
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
fn extract_table_from_block(block: &[String], result: &mut MdRenderResult, body_width: usize) {
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

    // Compute column widths based on display width
    let max_col = (body_width / col_count).saturating_sub(2).max(4);
    let col_widths: Vec<usize> = (0..col_count)
        .map(|ci| {
            let all_cells: Vec<&str> = header
                .get(ci)
                .into_iter()
                .chain(data_rows.iter().filter_map(|r| r.get(ci)))
                .map(|s| s.as_str())
                .collect();
            let max_width = all_cells
                .iter()
                .map(|c| UnicodeWidthStr::width(*c))
                .max()
                .unwrap_or(4);
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

fn render_table_into<'a, I>(
    events: &mut std::iter::Peekable<I>,
    result: &mut MdRenderResult,
    body_width: usize,
) where
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
                            let line_groups = collect_inline_spans(events);
                            let text: String = line_groups
                                .iter()
                                .flat_map(|g| g.iter())
                                .map(|s| s.content.as_ref())
                                .collect();
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
                            let line_groups = collect_inline_spans(events);
                            let text: String = line_groups
                                .iter()
                                .flat_map(|g| g.iter())
                                .map(|s| s.content.as_ref())
                                .collect();
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
            let all_cells: Vec<&str> = rows
                .iter()
                .filter_map(|r| r.get(ci))
                .map(|s| s.as_str())
                .collect();
            let max_w = all_cells
                .iter()
                .map(|c| UnicodeWidthStr::width(*c))
                .max()
                .unwrap_or(4);
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

/// Collect inline spans, returning one group per line.
/// A SoftBreak/HardBreak in markdown creates a new line group,
/// preserving original newlines as separate text lines.
fn collect_inline_spans<'a, I>(events: &mut std::iter::Peekable<I>) -> Vec<Vec<Span<'static>>>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut istyle = InlineStyle::new();

    macro_rules! flush_line {
        () => {
            flush_buf(&mut spans, &mut buf, &istyle);
            if !spans.is_empty() || !lines.is_empty() {
                lines.push(std::mem::take(&mut spans));
            }
        };
    }

    loop {
        match events.next() {
            Some(Event::Text(t)) => {
                buf.push_str(&t);
            }
            Some(Event::Code(t)) => {
                flush_buf(&mut spans, &mut buf, &istyle);
                spans.push(Span::styled(
                    format!("`{}`", t),
                    Style::default()
                        .fg(style::GREEN)
                        .add_modifier(Modifier::ITALIC),
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
                        lines.push(std::mem::take(&mut spans));
                        return lines;
                    }
                    TagEnd::Item => {
                        flush_buf(&mut spans, &mut buf, &istyle);
                        lines.push(std::mem::take(&mut spans));
                        return lines;
                    }
                    _ => {}
                }
            }
            Some(Event::SoftBreak | Event::HardBreak) => {
                flush_line!();
            }
            None => break,
            _ => {}
        }
    }

    flush_buf(&mut spans, &mut buf, &istyle);
    if !spans.is_empty() || !lines.is_empty() {
        lines.push(spans);
    }
    lines
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
            Some(Event::Text(t)) => {
                // Split text by newlines so embedded \n (common in code block
                // content from pulldown_cmark) produce separate lines.
                let text = t.to_string();
                let mut parts = text.split('\n').peekable();
                while let Some(part) = parts.next() {
                    if parts.peek().is_some() {
                        // This part is followed by a newline — complete line.
                        line.push_str(part);
                        lines.push(std::mem::take(&mut line));
                    } else {
                        // Last part — may be continued by the next Text event.
                        line.push_str(part);
                    }
                }
            }
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
            Span::styled(
                format!("# {} ", lang),
                Style::default().fg(style::TEXT_MUTED),
            ),
        ]);
    }
    for code_line in code_lines {
        result.push_line(vec![
            Span::styled("  ", Style::default().fg(style::TEXT_MUTED)),
            Span::styled(code_line.clone(), Style::default().fg(style::TEXT_PRIMARY)),
        ]);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Blockquote
// ═══════════════════════════════════════════════════════════════════════════

fn collect_blockquote_into<'a, I>(
    events: &mut std::iter::Peekable<I>,
    body_width: usize,
) -> Vec<MdLine>
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
                let line_groups = collect_inline_spans(events);
                for spans in &line_groups {
                    let mut quote_spans =
                        vec![Span::styled("│ ", Style::default().fg(style::YELLOW))];
                    for s in spans {
                        let mut merged = s.style;
                        if merged.fg.is_none() {
                            merged = merged.fg(style::YELLOW);
                        }
                        quote_spans.push(Span::styled(s.content.clone(), merged));
                    }
                    wrap_spans_no_indent(&mut out, &quote_spans, body_width, "  ");
                }
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

fn render_list_into<'a, I>(
    events: &mut std::iter::Peekable<I>,
    result: &mut MdRenderResult,
    body_width: usize,
) where
    I: Iterator<Item = Event<'a>>,
{
    let mut list_depth = 1u32;

    loop {
        match events.next() {
            Some(Event::Start(Tag::Item)) => {
                let line_groups = collect_inline_spans(events);
                let bullet = match list_depth {
                    1 => "•",
                    2 => "◦",
                    3 => "▪",
                    _ => "▸",
                };
                let indent = "  ".repeat((list_depth - 1) as usize);
                let prefix = format!("{}{} ", indent, bullet);
                for (idx, spans) in line_groups.iter().enumerate() {
                    let mut prefix_spans = if idx == 0 {
                        vec![Span::styled(
                            prefix.clone(),
                            Style::default().fg(style::BLUE),
                        )]
                    } else {
                        vec![Span::raw(format!("{}  ", indent))]
                    };
                    prefix_spans.extend(spans.iter().cloned());
                    wrap_spans_into(result, &prefix_spans, body_width, "");
                }
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

fn render_heading_into(
    result: &mut MdRenderResult,
    spans: &[Span<'static>],
    level: HeadingLevel,
    body_width: usize,
) {
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
fn wrap_spans_into(
    result: &mut MdRenderResult,
    spans: &[Span<'static>],
    max_width: usize,
    indent: &str,
) {
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
    let char_to_display: Vec<usize> = chars
        .iter()
        .map(|c| UnicodeWidthChar::width(*c).unwrap_or(0))
        .collect();

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
        let break_point = if let Some(space_pos) = chars[start..end].iter().rposition(|&c| c == ' ')
        {
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
fn wrap_spans_no_indent(
    out: &mut Vec<MdLine>,
    spans: &[Span<'static>],
    max_width: usize,
    indent: &str,
) {
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
    let char_to_display: Vec<usize> = chars
        .iter()
        .map(|c| UnicodeWidthChar::width(*c).unwrap_or(0))
        .collect();

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

        let break_point = if let Some(space_pos) = chars[start..end].iter().rposition(|&c| c == ' ')
        {
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
        let is_code =
            s.style.fg == Some(style::GREEN) && s.style.add_modifier.contains(Modifier::ITALIC);
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
        let char_end = if end < span_end {
            end - span_start
        } else {
            span_len
        };
        if char_start < char_end {
            let sub: String = s
                .content
                .chars()
                .skip(char_start)
                .take(char_end - char_start)
                .collect();
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
        extract_table_from_block(&block, &mut result, 80);

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
        extract_table_from_block(&block, &mut result, 80);

        assert_eq!(result.tables.len(), 1);
        let td = &result.tables[0];
        assert_eq!(td.header, vec!["Name", "Value"]);
        assert_eq!(td.rows.len(), 2);
    }

    #[test]
    fn test_extract_table_empty() {
        let mut result = MdRenderResult::new();
        extract_table_from_block(&[], &mut result, 80);
        assert!(result.tables.is_empty());
    }

    #[test]
    fn test_extract_table_single_row() {
        let block = vec!["| Name |".to_string(), "| --- |".to_string()];
        let mut result = MdRenderResult::new();
        extract_table_from_block(&block, &mut result, 80);
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
        let widget = output.build_table_widget(&td);
        let _ = widget;
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
        assert_eq!(result.lines.len(), 2); // header + code (no footer bars)
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
        let output = build_chat_content(&state, 80, 0, 2);
        assert_eq!(output.total_lines(), 1);
        assert!(output.rendered[0].line.to_string().contains("No messages"));
    }

    #[test]
    fn test_build_chat_content_with_table() {
        use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
        let state = CoreState {
            messages: vec![ChatMessage {
                role: MessageRole::User,
                content: "Show me data\n\n| Name | Value |\n| --- | --- |\n| A | 1 |\n| B | 2 |\n"
                    .to_string(),
                reasoning: String::new(),
                timestamp: "00:00".to_string(),
                status: MessageStatus::Completed,
            }],
            ..CoreState::default()
        };
        let output = build_chat_content(&state, 80, 0, 2);
        let table_line_count = output.rendered.iter().filter(|r| r.table.is_some()).count();
        assert!(table_line_count > 0, "should have table placeholder lines");
    }
    #[test]
    fn test_codeblock() {
        let mut events = "123\n123\n123\n".chars().map(|c| Event::Text(c.into()));
        let lines = collect_code_lines(&mut events);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "123");
        assert_eq!(lines[1], "123");
        assert_eq!(lines[2], "123");
    }
}
