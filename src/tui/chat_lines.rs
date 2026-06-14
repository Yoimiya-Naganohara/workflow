use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use super::state::{CoreState, MessageRole};
use super::style;

pub(crate) fn build_chat_lines(state: &CoreState, width: usize, _think_frame: u8) -> Vec<Line<'static>> {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for message in &state.messages {
        if matches!(message.role, MessageRole::System) {
            // Render system messages as muted inline text with · prefix
            let content_lines = render_markdown(&message.content, body_width);
            for cl in content_lines {
                let mut styled = vec![Span::styled("· ", Style::default().fg(style::TEXT_MUTED))];
                styled.extend(
                    cl.spans
                        .into_iter()
                        .map(|s| Span::styled(s.content, s.style.fg(style::TEXT_MUTED))),
                );
                lines.push(Line::from(styled));
            }
            lines.push(Line::from(String::new()));
            continue;
        }

        let is_tool_call = matches!(message.role, MessageRole::Decision);

        if is_tool_call {
            // Tool call as separate message (fallback)
            tool_call_lines(&mut lines, message);
            continue;
        }

        // Render message content (always, even if thinking/streaming)
        let is_user = matches!(message.role, MessageRole::User);
        let bar_color = if is_user { style::GREEN } else { style::BLUE };
        let bar_char = "┃";

        let content_lines = render_markdown(&message.content, body_width);
        for cl in content_lines {
            let mut styled = vec![Span::styled(bar_char, Style::default().fg(bar_color))];
            styled.extend(cl.spans.into_iter().map(|s| Span::styled(s.content, s.style)));
            lines.push(Line::from(styled));
        }

        lines.push(Line::from(String::new()));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages yet."));
    }

    lines
}

fn tool_call_lines(lines: &mut Vec<Line<'static>>, message: &crate::tui::state::ChatMessage) {
    let bar = Span::styled("┃", Style::default().fg(style::TEXT_MUTED));
    let content = &message.content;
    let (name, args) = if let Some(pos) = content.find(" — ") {
        (&content[..pos], &content[pos + 5..])
    } else {
        (content.as_str(), "")
    };

    let mut parts = vec![
        bar,
        Span::styled(
            name.trim().to_string(),
            Style::default().fg(style::PURPLE).add_modifier(Modifier::BOLD),
        ),
    ];

    if !args.is_empty() {
        parts.push(Span::styled(
            format!(" {}", args.trim()),
            Style::default().fg(style::TEXT_MUTED),
        ));
    }

    lines.push(Line::from(parts));
    lines.push(Line::from(String::new()));
}

fn render_markdown(text: &str, body_width: usize) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = pulldown_cmark::Parser::new_ext(text, opts);
    let mut events = parser.peekable();
    let mut out = Vec::new();

    while let Some(ev) = events.next() {
        match ev {
            Event::Start(Tag::Paragraph) => {
                let spans = collect_inline_spans(&mut events);
                let plain: String = spans.iter().map(|s| s.content.as_ref()).collect();
                if looks_like_table_row(&plain) {
                    let block = collect_paragraph_table_block(&plain, &mut events, body_width);
                    if block.len() >= 2 {
                        render_plain_table(&block, &mut out, body_width);
                    } else {
                        wrap_spans(&mut out, &spans, body_width, "  ");
                    }
                } else {
                    wrap_spans(&mut out, &spans, body_width, "  ");
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                let spans = collect_inline_spans(&mut events);
                render_heading(&mut out, &spans, level, body_width);
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                let code_lines = collect_code_lines(&mut events);
                flush_code_block(&mut out, &lang, &code_lines);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let inner = collect_blockquote(&mut events, body_width);
                out.extend(inner);
            }
            Event::Start(Tag::List(_)) => {
                render_list(&mut events, &mut out, body_width);
            }
            Event::Rule => {
                render_hr(&mut out, body_width);
            }
            Event::Start(Tag::Table(_alignments)) => {
                render_table(&mut events, &mut out, body_width);
            }
            _ => {}
        }
    }

    out
}

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with("```") {
        return false;
    }
    let pipe_count = trimmed.matches('|').count();
    pipe_count >= 2
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

fn is_separator_row(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 || !trimmed.contains('-') {
        return false;
    }
    let inner = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() > 2 {
        trimmed[1..trimmed.len() - 1].trim()
    } else if trimmed.starts_with('|') {
        trimmed[1..].trim()
    } else if trimmed.ends_with('|') {
        &trimmed[..trimmed.len() - 1]
    } else {
        trimmed
    };
    inner
        .split('|')
        .all(|cell| cell.trim().chars().all(|c| c == '-' || c == ':' || c == ' '))
        && inner.contains('-')
}

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

fn render_plain_table(block: &[String], out: &mut Vec<Line<'static>>, body_width: usize) {
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

    let table_w = body_width.min(80);
    let max_col = (table_w / col_count).saturating_sub(2).max(8).min(50);

    // Use display width for column sizing.
    let col_widths: Vec<usize> = (0..col_count)
        .map(|ci| {
            rows.iter()
                .filter_map(|r| r.get(ci))
                .filter(|c| !is_separator_row(&format!("| {} |", c)))
                .map(|c| UnicodeWidthStr::width(c.as_str()).min(max_col))
                .max()
                .unwrap_or(8)
        })
        .collect();

    let h_line = format!(
        "  {}",
        col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┬")
    );
    out.push(Line::from(Span::styled(h_line, Style::default().fg(style::TEXT_MUTED))));

    for row in rows.iter() {
        let is_sep = row
            .iter()
            .all(|c| c.chars().all(|ch| ch == '-' || ch == ':' || ch == ' ') && c.contains('-'));
        if is_sep {
            let sep = format!(
                "  {}",
                col_widths
                    .iter()
                    .map(|w| "─".repeat(w + 2))
                    .collect::<Vec<_>>()
                    .join("┼")
            );
            out.push(Line::from(Span::styled(sep, Style::default().fg(style::TEXT_MUTED))));
            continue;
        }

        let mut line = String::from("  │");
        for (ci, cell) in row.iter().enumerate() {
            let w = col_widths.get(ci).copied().unwrap_or(8);
            let cell_width = UnicodeWidthStr::width(cell.as_str());
            if cell_width <= w {
                let pad = w - cell_width;
                line.push_str(&format!(" {}{}│", cell, " ".repeat(pad + 1)));
            } else {
                let truncated = truncate_display(cell, w.saturating_sub(1));
                line.push_str(&format!(" {}…{}│", truncated, " ".repeat(1)));
            }
        }
        out.push(Line::from(Span::styled(line, Style::default().fg(style::TEXT_PRIMARY))));
    }

    let b_line = format!(
        "  {}",
        col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┴")
    );
    out.push(Line::from(Span::styled(b_line, Style::default().fg(style::TEXT_MUTED))));
    out.push(Line::from(String::new()));
}

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
                    _ => {
                        if let TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell = tag_end {
                            flush_buf(&mut spans, &mut buf, &istyle);
                            break;
                        }
                        if let TagEnd::Item = tag_end {
                            flush_buf(&mut spans, &mut buf, &istyle);
                            break;
                        }
                    }
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

fn collect_blockquote<'a, I>(events: &mut std::iter::Peekable<I>, body_width: usize) -> Vec<Line<'static>>
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
                    // Preserve inner formatting (bold, italic, code color) but tint base text yellow
                    let mut merged = s.style;
                    if merged.fg.is_none() {
                        merged = merged.fg(style::YELLOW);
                    }
                    // Only apply yellow if the span has no explicit color set
                    // (code=GREEN, link=BLUE should be preserved)
                    quote_spans.push(Span::styled(s.content.clone(), merged));
                }
                wrap_spans(&mut out, &quote_spans, body_width, "  ");
            }
            _ => {
                let _ = events.next();
            }
        }
    }
    out
}

fn render_list<'a, I>(events: &mut std::iter::Peekable<I>, out: &mut Vec<Line<'static>>, body_width: usize)
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
                // Use wrap_spans to preserve inline styling on continuation lines
                let mut prefix_spans = vec![Span::styled(prefix, Style::default().fg(style::BLUE))];
                prefix_spans.extend(item_spans.iter().map(|s| s.clone()));
                wrap_spans(out, &prefix_spans, body_width, "");
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

fn render_heading(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], level: HeadingLevel, body_width: usize) {
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
    // Apply heading color and bold while preserving inner formatting
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
    wrap_spans(out, &prefix_spans, body_width, "");
}

fn render_hr(out: &mut Vec<Line<'static>>, body_width: usize) {
    let hr_width = body_width.min(40);
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("─".repeat(hr_width), Style::default().fg(style::TEXT_MUTED)),
    ]));
}

/// Render a markdown table as a bordered grid.
fn render_table<'a, I>(events: &mut std::iter::Peekable<I>, out: &mut Vec<Line<'static>>, body_width: usize)
where
    I: Iterator<Item = Event<'a>>,
{
    // Collect all rows as styled spans. First row is header, rest are data.
    let mut rows: Vec<Vec<Vec<Span<'static>>>> = Vec::new();
    loop {
        match events.next() {
            Some(Event::Start(Tag::Table(_))) => {}
            Some(Event::Start(Tag::TableHead)) => {
                // TableHead contains cells directly (no TableRow wrapper in pulldown-cmark).
                let mut cells: Vec<Vec<Span<'static>>> = Vec::new();
                loop {
                    match events.next() {
                        Some(Event::Start(Tag::TableCell)) => {
                            let spans = collect_inline_spans(events);
                            cells.push(spans);
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
                let mut cells: Vec<Vec<Span<'static>>> = Vec::new();
                loop {
                    match events.next() {
                        Some(Event::Start(Tag::TableCell)) => {
                            let spans = collect_inline_spans(events);
                            cells.push(spans);
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

    let table_w = body_width.min(80);
    let max_col = (table_w / col_count).saturating_sub(2).max(8).min(50);

    // Find max display width per column.
    let col_widths: Vec<usize> = (0..col_count)
        .map(|ci| {
            rows.iter()
                .filter_map(|r| r.get(ci))
                .map(|spans| {
                    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
                    UnicodeWidthStr::width(text.as_str()).min(max_col)
                })
                .max()
                .unwrap_or(8)
        })
        .collect();

    // Top border.
    let h_line = format!(
        "  {}",
        col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┬")
    );
    out.push(Line::from(Span::styled(h_line, Style::default().fg(style::TEXT_MUTED))));

    for (ri, row) in rows.iter().enumerate() {
        let mut spans_out: Vec<Span<'static>> = vec![Span::styled("  │", Style::default().fg(style::TEXT_PRIMARY))];
        for (ci, cell_spans) in row.iter().enumerate() {
            let w = col_widths.get(ci).copied().unwrap_or(8);
            let cell_text: String = cell_spans.iter().map(|s| s.content.as_ref()).collect();
            let cell_width = UnicodeWidthStr::width(cell_text.as_str());

            if cell_width <= w {
                // Fits — render with full inline styling
                spans_out.push(Span::raw(" ".to_string()));
                for s in cell_spans {
                    spans_out.push(Span::styled(s.content.clone(), s.style));
                }
                // Pad to column width
                let pad = w - cell_width;
                if pad > 0 {
                    spans_out.push(Span::styled(" ".repeat(pad), Style::default().fg(style::TEXT_PRIMARY)));
                }
                spans_out.push(Span::styled(" │", Style::default().fg(style::TEXT_PRIMARY)));
            } else {
                // Truncate: build styled segments up to display width w-1, then append ellipsis
                spans_out.push(Span::raw(" ".to_string()));
                let mut remaining = w.saturating_sub(1);
                for s in cell_spans {
                    if remaining == 0 {
                        break;
                    }
                    let seg_width = UnicodeWidthStr::width(s.content.as_ref());
                    if seg_width <= remaining {
                        spans_out.push(Span::styled(s.content.clone(), s.style));
                        remaining -= seg_width;
                    } else {
                        // Partial: take chars until we fill remaining display width
                        let partial: String = truncate_display(&s.content, remaining);
                        if !partial.is_empty() {
                            spans_out.push(Span::styled(partial, s.style));
                        }
                        remaining = 0;
                    }
                }
                spans_out.push(Span::styled("…", Style::default().fg(style::TEXT_PRIMARY)));
                spans_out.push(Span::styled(" │", Style::default().fg(style::TEXT_PRIMARY)));
            }
        }
        out.push(Line::from(spans_out));

        // Separator after header.
        if ri == 0 && rows.len() > 1 {
            let sep = format!(
                "  {}",
                col_widths
                    .iter()
                    .map(|w| "─".repeat(w + 2))
                    .collect::<Vec<_>>()
                    .join("┼")
            );
            out.push(Line::from(Span::styled(sep, Style::default().fg(style::TEXT_MUTED))));
        }
    }

    // Bottom border.
    let b_line = format!(
        "  {}",
        col_widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┴")
    );
    out.push(Line::from(Span::styled(b_line, Style::default().fg(style::TEXT_MUTED))));
    out.push(Line::from(String::new()));
}

fn flush_code_block(out: &mut Vec<Line<'static>>, lang: &str, code_lines: &[String]) {
    if !lang.is_empty() {
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("┌─ {} ", lang), Style::default().fg(style::TEXT_MUTED)),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("┌───", Style::default().fg(style::TEXT_MUTED)),
        ]));
    }
    for code_line in code_lines {
        out.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(style::TEXT_MUTED)),
            Span::styled(code_line.clone(), Style::default().fg(style::TEXT_PRIMARY)),
        ]));
    }
    out.push(Line::from(Span::styled(
        "  └───",
        Style::default().fg(style::TEXT_MUTED),
    )));
}

fn wrap_spans(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], max_width: usize, indent: &str) {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = full_text.chars().collect();
    let indent_len = indent.chars().count();
    if chars.is_empty() {
        return;
    }
    let avail = max_width.saturating_sub(indent_len);
    if chars.len() <= avail {
        let mut row = vec![Span::raw(indent.to_string())];
        for s in spans {
            row.push(Span::styled(s.content.clone(), s.style));
        }
        out.push(Line::from(row));
        return;
    }

    // Find backtick span boundaries to prevent breaking inside inline code
    let code_ranges = find_code_span_ranges(spans);

    let mut start = 0usize;
    let len = chars.len();
    while start < len {
        let remaining = len - start;
        if remaining <= avail {
            let mut row = vec![Span::raw(indent.to_string())];
            row.extend(make_spans_for_range(spans, start, len));
            out.push(Line::from(row));
            break;
        }
        // Find best break point: prefer space, but avoid breaking inside code spans
        let mut end = start + avail;
        if let Some(space) = chars[start..start + avail].iter().rposition(|&c| c == ' ') {
            let space_end = start + space;
            // Check if breaking at this space would split a code span
            if !is_inside_code_span(space_end, &code_ranges) {
                end = space_end;
            } else {
                // Try earlier spaces or fall back to non-code-span boundary
                let mut best = None;
                for i in (0..space).rev() {
                    if chars[start + i] == ' ' && !is_inside_code_span(start + i, &code_ranges) {
                        best = Some(start + i);
                        break;
                    }
                }
                if let Some(b) = best {
                    end = b;
                } else {
                    // No good space found; break at the edge of a code span if possible
                    let mut best_boundary = None;
                    for &(rs, _re) in &code_ranges {
                        if rs > start && rs <= start + avail {
                            best_boundary = Some(rs);
                        }
                    }
                    if let Some(b) = best_boundary {
                        end = b;
                    }
                    // else: just break at avail (last resort, inside code)
                }
            }
        } else {
            // No space: try to break at a code span boundary
            let mut best_boundary = None;
            for &(rs, _re) in &code_ranges {
                if rs > start && rs <= start + avail {
                    best_boundary = Some(rs);
                }
            }
            if let Some(b) = best_boundary {
                end = b;
            }
        }
        let mut row = vec![Span::raw(indent.to_string())];
        row.extend(make_spans_for_range(spans, start, end));
        out.push(Line::from(row));
        start = end;
        while start < len && chars[start] == ' ' {
            start += 1;
        }
    }
}

/// Find character-position ranges of inline code spans (backtick-delimited).
/// Returns Vec<(start_char, end_char)> where end_char is exclusive.
fn find_code_span_ranges(spans: &[Span<'static>]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut pos = 0;
    for s in spans {
        let len = s.content.chars().count();
        // Detect code spans: style has GREEN fg and ITALIC modifier (our convention)
        let is_code = s.style.fg == Some(style::GREEN) && s.style.add_modifier.contains(Modifier::ITALIC);
        if is_code {
            // The span content includes backticks, e.g. `code`
            ranges.push((pos, pos + len));
        }
        pos += len;
    }
    ranges
}

/// Check if a character position falls inside any code span range.
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

#[allow(dead_code)]
fn char_boundary_clamp(s: &str, byte_pos: usize) -> usize {
    if byte_pos >= s.len() {
        return s.len();
    }
    if s.is_char_boundary(byte_pos) {
        return byte_pos;
    }
    s.char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i < byte_pos)
        .last()
        .unwrap_or(0)
}

#[allow(dead_code)]
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_width {
        return vec![line.to_string()];
    }
    let mut result = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= max_width {
            result.push(chars[start..].iter().collect());
            break;
        }
        let end = if let Some(space) = chars[start..(start + max_width).min(chars.len())]
            .iter()
            .rposition(|&c| c == ' ')
        {
            start + space
        } else {
            (start + max_width).min(chars.len())
        };
        result.push(chars[start..end].iter().collect());
        start = end;
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_line_short() {
        assert_eq!(wrap_line("hello", 80), vec!["hello"]);
    }

    #[test]
    fn test_wrap_line_long() {
        let result = wrap_line("abc def ghi jkl mno pqr stu vwx yz", 12);
        assert!(result.len() > 1);
    }

    #[test]
    fn test_render_markdown_empty() {
        assert!(render_markdown("", 80).is_empty());
    }

    #[test]
    fn test_render_markdown_bold() {
        let lines = render_markdown("**bold**", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_code_block() {
        let lines = render_markdown("```rust\nfn main() {}\n```", 80);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_render_markdown_table() {
        let md = "| Name | Value |\n| --- | --- |\n| A | 1 |\n| B | 2 |";
        let lines = render_markdown(md, 80);
        let text: String = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains('│'),
            "Table should contain │ border chars, got:\n{}",
            text
        );
        assert!(!text.contains("| Name"), "Raw pipes should not appear, got:\n{}", text);
    }

    #[test]
    fn test_render_markdown_table_no_leading_pipe() {
        let md = "Name | Value\n--- | ---\nA | 1\nB | 2";
        let lines = render_markdown(md, 80);
        let text: String = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains('│'),
            "Table should contain │ border chars, got:\n{}",
            text
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
    fn test_table_inline_formatting_preserved() {
        let md = "| **Name** | `Value` |
| --- | --- |
| A | 1 |";
        let lines = render_markdown(md, 80);
        // Find spans that should be bold or code-styled
        let all_spans: Vec<&Span> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        let has_bold = all_spans
            .iter()
            .any(|s| s.content.contains("Name") && s.style.add_modifier.contains(Modifier::BOLD));
        let has_code = all_spans
            .iter()
            .any(|s| s.content.contains("`Value`") && s.style.fg == Some(style::GREEN));
        assert!(has_bold, "Table header should preserve **bold** formatting");
        assert!(has_code, "Table header should preserve `code` formatting");
    }

    #[test]
    fn test_table_cjk_truncation() {
        // CJK chars are 3 bytes but 1 display column
        let md = "| 名前 | 値 |\n| --- | --- |\n| 日本語テスト | 長いテストデータ |";
        let lines = render_markdown(md, 30); // narrow width to force truncation
        let text: String = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('│'), "CJK table should have borders, got:\n{}", text);
        assert!(
            text.contains('…'),
            "CJK table should truncate wide cells, got:\n{}",
            text
        );
        // Should NOT contain garbled bytes from byte-slice truncation
        assert!(
            !text.contains('\u{FFFD}'),
            "CJK table should not contain replacement chars"
        );
    }

    #[test]
    fn test_table_column_width_display() {
        // Column widths should be based on display width, not byte length
        let md = "| AB | CDEF |\n| --- | --- |\n| X | Y |";
        let lines = render_markdown(md, 80);
        let text: String = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        // CDEF column should be wider than AB column
        assert!(text.contains("│"), "Should have table borders");
    }

    #[test]
    fn test_truncate_display_basic() {
        assert_eq!(truncate_display("hello", 3), "hel");
        assert_eq!(truncate_display("hi", 10), "hi");
        assert_eq!(truncate_display("", 5), "");
    }

    #[test]
    fn test_truncate_display_cjk() {
        // CJK chars are full-width (2 display columns each).
        assert_eq!(truncate_display("你好世界", 4), "你好");
        assert_eq!(truncate_display("你好世界", 2), "你");
        assert_eq!(truncate_display("你好世界", 1), "");
    }
}
