use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::state::{AppState, MessageRole, MessageStatus};

pub(crate) fn build_chat_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for message in &state.messages {
        let is_tool_call = matches!(
            message.role,
            MessageRole::Decision if message.content.starts_with('\u{1f527}')
        );

        if is_tool_call {
            let first_newline = message.content.find('\n').unwrap_or(message.content.len());
            let header_clean = message.content[..first_newline]
                .trim_start_matches('\u{1f527}')
                .trim()
                .to_string();
            lines.push(Line::from(vec![Span::styled(
                header_clean,
                Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
            )]));
            if first_newline < message.content.len() {
                for raw_line in message.content[first_newline..].lines() {
                    let trimmed = raw_line.trim();
                    if !trimmed.is_empty() && trimmed != "```json" && trimmed != "```" {
                        lines.push(Line::from(vec![
                            Span::styled("    ", Style::default().fg(Color::DarkGray)),
                            Span::styled(trimmed.to_string(), Style::default().fg(Color::Cyan)),
                        ]));
                    }
                }
            }
            lines.push(Line::from(String::new()));
            continue;
        }

        // ── Header (status + timestamp, no role label) ──
        let state_indicator = match message.status {
            MessageStatus::Thinking => Span::styled(
                "  \u{25cc}",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
            ),
            MessageStatus::Streaming => Span::styled(
                "  \u{25c9}",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK),
            ),
            MessageStatus::Completed => Span::styled("  \u{2713}", Style::default().fg(Color::Green)),
            MessageStatus::Error => Span::styled("  \u{2717}", Style::default().fg(Color::Red)),
        };

        let ts = Span::styled(
            format!("[{}] ", message.timestamp),
            Style::default().fg(Color::DarkGray),
        );

        lines.push(Line::from(vec![
            state_indicator,
            ts,
        ]));

        if message.content.is_empty() && matches!(message.status, MessageStatus::Thinking) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "thinking\u{2026}  ",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(String::new()));
            continue;
        }

        // ── Content ──
        let content_lines = render_markdown(&message.content, body_width);
        for (i, cl) in content_lines.into_iter().enumerate() {
            if i == 0 {
                // First content line: flush left
                lines.push(cl);
            } else {
                lines.push(cl);
            }
        }

        // Streaming cursor: blinking block at end of streaming messages
        if matches!(message.status, MessageStatus::Streaming) && !message.content.is_empty() {
            if let Some(last) = lines.last_mut() {
                last.spans.push(Span::styled(
                    " \u{2588}",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::SLOW_BLINK),
                ));
            }
        }

        // ── Separator between messages ──
        let sep_char = match message.role {
            MessageRole::System => "\u{2500}",
            MessageRole::User => "\u{2504}",
            MessageRole::Agent => "\u{2504}",
            MessageRole::Decision => "\u{2504}",
        };
        let sep_style = Style::default().fg(Color::DarkGray);
        if content_width >= 4 {
            let sep_count = content_width.saturating_sub(2);
            let sep_line = format!("  {}", sep_char.repeat(sep_count));
            lines.push(Line::from(Span::styled(sep_line, sep_style)));
        }

        lines.push(Line::from(String::new()));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages yet."));
    }

    lines
}

fn render_markdown(text: &str, body_width: usize) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);

    let parser = pulldown_cmark::Parser::new_ext(text, opts);
    let mut events = parser.peekable();
    let mut out = Vec::new();

    while let Some(ev) = events.next() {
        match ev {
            Event::Start(Tag::Paragraph) => {
                let spans = collect_inline_spans(&mut events);
                wrap_spans(&mut out, &spans, body_width, "");
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
            Event::Start(Tag::HtmlBlock) => {
                let html_lines = collect_html_block(&mut events);
                if !html_lines.is_empty() {
                    for line_text in &html_lines {
                        wrap_spans(
                            &mut out,
                            &[Span::styled(line_text.clone(), Style::default().fg(Color::DarkGray))],
                            body_width,
                            "",
                        );
                    }
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                let inner = collect_blockquote(&mut events, body_width);
                out.extend(inner);
            }
            Event::Start(Tag::List(_)) => {
                render_list(&mut events, &mut out, body_width);
            }
            Event::Start(Tag::Table(aligns)) => {
                render_table(&mut events, &mut out, &aligns, body_width);
            }
            Event::Rule => {
                render_hr(&mut out, body_width);
            }
            _ => {}
        }
    }

    out
}

struct InlineStyle {
    bold: u32,
    italic: u32,
    strike: u32,
    link: u32,
    fg: Vec<Color>,
}

impl InlineStyle {
    fn new() -> Self {
        Self {
            bold: 0,
            italic: 0,
            strike: 0,
            link: 0,
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
        if self.link > 0 {
            s = s.add_modifier(Modifier::UNDERLINED);
            if let Some(c) = self.fg.last() {
                s = s.fg(*c);
            }
        } else if let Some(c) = self.fg.last() {
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
    let mut style = InlineStyle::new();

    loop {
        match events.next() {
            Some(Event::Text(t)) => {
                buf.push_str(&t);
            }
            Some(Event::Code(t)) => {
                flush_buf(&mut spans, &mut buf, &style);
                spans.push(Span::styled(
                    format!("`{}`", t),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
                ));
            }
            Some(Event::Start(tag)) => {
                flush_buf(&mut spans, &mut buf, &style);
                match tag {
                    Tag::Emphasis => style.italic += 1,
                    Tag::Strong => style.bold += 1,
                    Tag::Strikethrough => style.strike += 1,
                    Tag::Link { .. } => {
                        style.link += 1;
                        style.fg.push(Color::Blue);
                    }
                    Tag::Image { .. } => {
                        style.fg.push(Color::DarkGray);
                    }
                    _ => {}
                }
            }
            Some(Event::End(tag_end)) => {
                flush_buf(&mut spans, &mut buf, &style);
                match tag_end {
                    TagEnd::Emphasis => style.italic = style.italic.saturating_sub(1),
                    TagEnd::Strong => style.bold = style.bold.saturating_sub(1),
                    TagEnd::Strikethrough => style.strike = style.strike.saturating_sub(1),
                    TagEnd::Link => {
                        style.link = style.link.saturating_sub(1);
                        style.fg.pop();
                    }
                    TagEnd::Image => {
                        style.fg.pop();
                    }
                    _ => {
                        if let TagEnd::Paragraph | TagEnd::Heading(_) = tag_end {
                            flush_buf(&mut spans, &mut buf, &style);
                            break;
                        }
                        if let TagEnd::Item | TagEnd::TableCell = tag_end {
                            flush_buf(&mut spans, &mut buf, &style);
                            break;
                        }
                    }
                }
            }
            Some(Event::SoftBreak | Event::HardBreak) => {
                buf.push(' ');
            }
            Some(Event::Html(html)) => {
                flush_buf(&mut spans, &mut buf, &style);
                let clean = strip_html(&html);
                if !clean.is_empty() {
                    spans.push(Span::styled(clean, Style::default().fg(Color::DarkGray)));
                }
            }
            Some(Event::TaskListMarker(checked)) => {
                flush_buf(&mut spans, &mut buf, &style);
                let marker = if checked { "\u{2611}" } else { "\u{2610}" };
                spans.push(Span::styled(marker, Style::default().fg(Color::Green)));
            }
            None => break,
            _ => {}
        }
    }

    flush_buf(&mut spans, &mut buf, &style);
    spans
}

fn flush_buf(spans: &mut Vec<Span<'static>>, buf: &mut String, style: &InlineStyle) {
    if !buf.is_empty() {
        let s = style.current_style();
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

/// Collect all `Event::Html` content lines from an HTML block.
fn collect_html_block<'a, I>(events: &mut I) -> Vec<String>
where
    I: Iterator<Item = Event<'a>>,
{
    let mut lines = Vec::new();
    loop {
        match events.next() {
            Some(Event::Html(html)) => {
                let clean = strip_html(&html);
                if !clean.is_empty() {
                    // Split multi-line HTML content into individual lines.
                    for l in clean.lines() {
                        let trimmed = l.trim();
                        if !trimmed.is_empty() {
                            lines.push(trimmed.to_string());
                        }
                    }
                }
            }
            Some(Event::End(TagEnd::HtmlBlock)) => break,
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
                let mut quote_spans = vec![Span::styled("  \u{258e} ", Style::default().fg(Color::DarkGray))];
                for s in &spans {
                    quote_spans.push(Span::styled(s.content.clone(), s.style.fg(Color::DarkGray)));
                }
                wrap_spans(&mut out, &quote_spans, body_width, "");
            }
            Some(Event::Rule) => {
                let _ = events.next();
                render_hr(&mut out, body_width);
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
    let mut item_counter = 0u64;
    let mut list_depth = 1u32;

    loop {
        match events.next() {
            Some(Event::Start(Tag::Item)) => {
                item_counter += 1;
                let raw_spans = collect_inline_spans(events);
                let indent = "  ";
                let wrap_indent = "    ";

                let text: String = raw_spans.iter().map(|s| s.content.as_ref()).collect();

                let first = raw_spans.first().map(|s| s.content.as_ref()).unwrap_or("");
                if first == "\u{2611}" || first == "\u{2610}" {
                    let marker = raw_spans[0].content.clone();
                    let content_text = if text.len() > marker.len() {
                        &text[marker.len()..]
                    } else {
                        ""
                    };
                    let content_text = content_text.trim();
                    let wrapped = wrap_line(content_text, body_width.saturating_sub(5));
                    for (idx, w) in wrapped.iter().enumerate() {
                        if idx == 0 {
                            out.push(Line::from(vec![
                                Span::raw(format!("{}{} ", indent, marker)),
                                Span::styled(w.to_string(), Style::default()),
                            ]));
                        } else {
                            out.push(Line::from(vec![
                                Span::raw(wrap_indent),
                                Span::styled(w.to_string(), Style::default()),
                            ]));
                        }
                    }
                    continue;
                }

                let bullet = format!("{:>2}.", item_counter);
                let prefix = format!("{}{} ", indent, bullet);
                let wrapped = wrap_line(&text, body_width.saturating_sub(4));
                for (idx, w) in wrapped.iter().enumerate() {
                    if idx == 0 {
                        out.push(Line::from(vec![Span::raw(prefix.clone()), Span::raw(w.to_string())]));
                    } else {
                        out.push(Line::from(vec![
                            Span::raw(wrap_indent),
                            Span::styled(w.to_string(), Style::default()),
                        ]));
                    }
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

fn render_heading(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], level: HeadingLevel, body_width: usize) {
    let level_num: u8 = match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    };
    let header_color = match level_num {
        1 => Color::Yellow,
        2 => Color::Cyan,
        _ => Color::Blue,
    };
    let prefix = format!("  {} ", "#".repeat(level_num as usize));
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let wrapped = wrap_line(&text, body_width);

    for w in wrapped {
        let line_spans = spans_for_line(spans, &w);
        let mut row = vec![Span::styled(prefix.clone(), Style::default().fg(Color::DarkGray))];
        for s in line_spans {
            let mut style = s.style;
            style = style.fg(header_color).add_modifier(Modifier::BOLD);
            row.push(Span::styled(s.content, style));
        }
        out.push(Line::from(row));
    }
}

fn render_hr(out: &mut Vec<Line<'static>>, body_width: usize) {
    let hr_width = body_width.min(40);
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("\u{2500}".repeat(hr_width), Style::default().fg(Color::DarkGray)),
    ]));
}

fn flush_code_block(out: &mut Vec<Line<'static>>, lang: &str, code_lines: &[String]) {
    if !lang.is_empty() {
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(
                format!("\u{250c}\u{2500} {} ", lang),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
    } else {
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("\u{250c}\u{2500}\u{2500}\u{2500}", Style::default().fg(Color::DarkGray)),
        ]));
    }
    for code_line in code_lines {
        out.push(Line::from(vec![
            Span::styled("  \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }
    out.push(Line::from(Span::styled(
        "  \u{2514}\u{2500}\u{2500}\u{2500}",
        Style::default().fg(Color::DarkGray),
    )));
}

fn render_table<'a, I>(
    events: &mut std::iter::Peekable<I>,
    out: &mut Vec<Line<'static>>,
    aligns: &[pulldown_cmark::Alignment],
    body_width: usize,
) where
    I: Iterator<Item = Event<'a>>,
{
    let ncols = aligns.len();
    let mut headers: Vec<Vec<String>> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut current_cell_parts: Vec<String> = Vec::new();
    let mut in_head = false;
    let mut in_row = false;
    let mut table_depth = 1u32;

    loop {
        match events.next() {
            Some(Event::Start(Tag::TableHead)) => in_head = true,
            Some(Event::End(TagEnd::TableHead)) => in_head = false,
            Some(Event::Start(Tag::TableRow)) => {
                in_row = true;
                current_cell_parts.clear();
            }
            Some(Event::End(TagEnd::TableRow)) => {
                if !current_cell_parts.is_empty() {
                    if in_head {
                        headers.push(current_cell_parts.clone());
                    } else {
                        let joined = current_cell_parts.join("");
                        if rows.is_empty() {
                            rows.push(vec![joined]);
                        } else {
                            rows.last_mut().unwrap().push(joined);
                        }
                    }
                }
                current_cell_parts.clear();
                in_row = false;
            }
            Some(Event::Start(Tag::TableCell)) => {
                current_cell_parts.clear();
            }
            Some(Event::End(TagEnd::TableCell)) => {
                let joined = current_cell_parts.join("");
                if in_head {
                    headers.push(vec![joined]);
                } else if in_row {
                    let idx = if rows.is_empty() { 0 } else { rows.len() - 1 };
                    if rows.is_empty() || !rows[idx].is_empty() {
                        let cell_text = current_cell_parts.join("");
                        if rows.is_empty() {
                            rows.push(vec![cell_text]);
                        } else {
                            rows[idx].push(cell_text);
                        }
                    }
                }
                current_cell_parts.clear();
            }
            Some(Event::Text(t)) => {
                current_cell_parts.push(t.to_string());
            }
            Some(Event::Code(t)) => {
                current_cell_parts.push(format!("`{}`", t));
            }
            Some(Event::End(TagEnd::Table)) => {
                table_depth -= 1;
                if table_depth == 0 {
                    break;
                }
            }
            Some(Event::Start(Tag::Table(_))) => table_depth += 1,
            None => break,
            _ => {}
        }
    }

    render_table_output(out, &headers, &rows, ncols, body_width);
}

fn render_table_output(
    out: &mut Vec<Line<'static>>,
    headers: &[Vec<String>],
    rows: &[Vec<String>],
    ncols: usize,
    body_width: usize,
) {
    let effective_cols = if ncols > 0 {
        ncols
    } else {
        headers
            .len()
            .max(rows.iter().map(|r| r.len()).max().unwrap_or(0))
            .max(1)
    };
    if effective_cols == 0 {
        return;
    }

    let max_width = body_width.saturating_sub(2 + (effective_cols.saturating_sub(1) * 3));
    let mut col_widths: Vec<usize> = (0..effective_cols)
        .map(|ci| {
            let hw = headers
                .iter()
                .map(|h| h.get(ci).map(|s| s.len()).unwrap_or(0))
                .max()
                .unwrap_or(0);
            let rw = rows.iter().flat_map(|r| r.get(ci)).map(|c| c.len()).max().unwrap_or(0);
            hw.max(rw).min(max_width)
        })
        .collect();

    let total: usize = col_widths.iter().sum();
    if total < max_width && effective_cols > 0 {
        let extra = (max_width - total) / effective_cols;
        for w in &mut col_widths {
            *w += extra;
        }
    }

    let mk_sep = |left: &str, mid: &str, right: &str| -> String {
        format!(
            "  {}{}{}",
            left,
            col_widths
                .iter()
                .map(|w| "\u{2500}".repeat(*w))
                .collect::<Vec<_>>()
                .join(&format!("\u{2500}{}\u{2500}", mid)),
            right,
        )
    };

    let top = mk_sep("\u{250c}", "\u{252c}", "\u{2510}");
    let sep = mk_sep("\u{251c}", "\u{253c}", "\u{2524}");
    let bot = mk_sep("\u{2514}", "\u{2534}", "\u{2518}");

    out.push(Line::from(Span::styled(top, Style::default().fg(Color::DarkGray))));

    if !headers.is_empty() && headers.iter().any(|h| !h.is_empty()) {
        let mut cells = vec![Span::styled("  \u{2502} ", Style::default().fg(Color::DarkGray))];
        for ci in 0..effective_cols {
            let cell_text = headers.first().and_then(|h| h.get(ci)).map(|s| s.as_str()).unwrap_or("");
            let w = col_widths.get(ci).copied().unwrap_or(0);
            let padded = if ci + 1 < effective_cols {
                format!("{:<width$} \u{2502} ", cell_text, width = w)
            } else {
                format!("{:<width$} \u{2502}", cell_text, width = w)
            };
            cells.push(Span::styled(padded, Style::default().add_modifier(Modifier::BOLD)));
        }
        out.push(Line::from(cells));
        out.push(Line::from(Span::styled(
            sep.clone(),
            Style::default().fg(Color::DarkGray),
        )));
    }

    for row in rows {
        let mut cells = vec![Span::styled("  \u{2502} ", Style::default().fg(Color::DarkGray))];
        for ci in 0..effective_cols {
            let cell_text = row.get(ci).map(|s| s.as_str()).unwrap_or("");
            let w = col_widths.get(ci).copied().unwrap_or(0);
            let padded = if ci + 1 < effective_cols {
                format!("{:<width$} \u{2502} ", cell_text, width = w)
            } else {
                format!("{:<width$} \u{2502}", cell_text, width = w)
            };
            cells.push(Span::raw(padded));
        }
        out.push(Line::from(cells));
    }

    out.push(Line::from(Span::styled(bot, Style::default().fg(Color::DarkGray))));
}

fn wrap_spans(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], max_width: usize, indent: &str) {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = full_text.chars().collect();
    let indent_chars: Vec<char> = indent.chars().collect();
    let indent_len = indent_chars.len();

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

        let end = if let Some(space) = chars[start..start + avail].iter().rposition(|&c| c == ' ') {
            start + space
        } else {
            start + avail
        };

        let mut row = vec![Span::raw(indent.to_string())];
        row.extend(make_spans_for_range(spans, start, end));
        out.push(Line::from(row));

        start = end + 1;
        while start < len && chars[start] == ' ' {
            start += 1;
        }
    }
}

fn make_spans_for_range(all: &[Span<'static>], start: usize, end: usize) -> Vec<Span<'static>> {
    if start >= end {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut pos = 0usize;

    for s in all {
        let span_len = s.content.len();
        let span_start = pos;
        let span_end = pos + span_len;

        if span_end <= start {
            pos = span_end;
            continue;
        }
        if span_start >= end {
            break;
        }

        let seg_start = start.saturating_sub(span_start);
        let seg_end = if end < span_end { end - span_start } else { span_len };

        if seg_start < seg_end {
            let seg_text: String = s.content.chars().skip(seg_start).take(seg_end - seg_start).collect();
            result.push(Span::styled(seg_text, s.style));
        }

        pos = span_end;
    }

    result
}

fn spans_for_line(spans: &[Span<'static>], line_text: &str) -> Vec<Span<'static>> {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    match full_text.find(line_text) {
        Some(byte_start) => {
            let byte_end = byte_start + line_text.len();
            make_spans_for_range(spans, byte_start, byte_end)
        }
        None => vec![Span::styled(line_text.to_string(), Style::default())],
    }
}

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

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' if !in_tag => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
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
        let result = wrap_line("hello", 80);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn test_wrap_line_long() {
        let line = "abc def ghi jkl mno pqr stu vwx yz";
        let result = wrap_line(line, 12);
        assert!(result.len() > 1);
        for segment in &result {
            assert!(segment.len() <= 12);
        }
    }

    #[test]
    fn test_wrap_line_no_spaces() {
        let line = "abcdefghijklmnopqrstuvwxyz";
        let result = wrap_line(line, 10);
        assert_eq!(result, vec!["abcdefghij", "klmnopqrst", "uvwxyz"]);
    }

    #[test]
    fn test_render_markdown_empty() {
        let lines = render_markdown("", 80);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_render_markdown_plain_text() {
        let lines = render_markdown("hello world", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_bold() {
        let lines = render_markdown("**bold text**", 80);
        assert!(!lines.is_empty());
        let line = &lines[0];
        let spans = &line.spans;
        let has_bold = spans.iter().any(|s| s.style.add_modifier.contains(Modifier::BOLD));
        assert!(has_bold);
    }

    #[test]
    fn test_render_markdown_italic() {
        let lines = render_markdown("*italic text*", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_bold_italic_nested() {
        let lines = render_markdown("***bold italic***", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_nested_bold_italic() {
        let lines = render_markdown("**bold *and italic* text**", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_strikethrough() {
        let lines = render_markdown("~~deleted~~", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_inline_code() {
        let lines = render_markdown("use `code` here", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_link() {
        let lines = render_markdown("[text](https://example.com)", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_image() {
        let lines = render_markdown("![alt](https://example.com/img.png)", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_headers() {
        let h1 = render_markdown("# Header 1", 80);
        assert!(!h1.is_empty());

        let h2 = render_markdown("## Header 2", 80);
        assert!(!h2.is_empty());
    }

    #[test]
    fn test_render_markdown_code_block() {
        let lines = render_markdown("```rust\nfn main() {}\n```", 80);
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_render_markdown_unordered_list() {
        let lines = render_markdown("- item 1\n- item 2", 80);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_render_markdown_ordered_list() {
        let lines = render_markdown("1. first\n2. second", 80);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_render_markdown_task_list() {
        let lines = render_markdown("- [x] done\n- [ ] todo", 80);
        assert!(lines.len() >= 2);
    }

    #[test]
    fn test_render_markdown_blockquote() {
        let lines = render_markdown("> quoted text", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_horizontal_rule() {
        let lines = render_markdown("---", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_render_markdown_table() {
        let lines = render_markdown("| A | B |\n|---|---|\n| 1 | 2 |", 80);
        assert!(lines.len() >= 4);
    }

    #[test]
    fn test_html_stripped() {
        let lines = render_markdown("Hello <br> world", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_setext_heading() {
        let lines = render_markdown("Heading\n=====", 80);
        assert!(!lines.is_empty());
    }

    #[test]
    fn test_make_spans_for_range() {
        let spans = vec![
            Span::styled("hello ", Style::default()),
            Span::styled("world", Style::default().add_modifier(Modifier::BOLD)),
        ];
        let result = make_spans_for_range(&spans, 0, 11);
        assert_eq!(result.len(), 2);
        let result2 = make_spans_for_range(&spans, 0, 6);
        assert_eq!(result2.len(), 1);
        let result3 = make_spans_for_range(&spans, 6, 11);
        assert_eq!(result3.len(), 1);
        assert!(result3[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_strip_html() {
        assert_eq!(strip_html("hello<br>world"), "helloworld");
        assert_eq!(strip_html("<kbd>Ctrl</kbd>"), "Ctrl");
        assert_eq!(strip_html("no tags"), "no tags");
    }
}
