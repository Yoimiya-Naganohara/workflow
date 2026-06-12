use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use super::state::{CoreState, MessageRole, MessageStatus};
use super::style;

pub(crate) fn build_chat_lines(state: &CoreState, width: usize, think_frame: u8) -> Vec<Line<'static>> {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for message in &state.messages {
        if matches!(message.role, MessageRole::System) {
            continue;
        }

        let is_tool_call = matches!(message.role, MessageRole::Decision);

        if is_tool_call {
            tool_call_lines(&mut lines, message);
            continue;
        }

        if message.content.is_empty() && matches!(message.status, MessageStatus::Thinking) {
            let spinner = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
            let phase = (think_frame as usize / 2) % spinner.len();
            let ch = spinner[phase];
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} Generating...", ch),
                    Style::default().fg(style::BLUE).add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(String::new()));
            continue;
        }

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
    let bar = Span::styled("┃", Style::default().fg(style::OVERLAY0));
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
            Style::default().fg(style::MAUVE).add_modifier(Modifier::BOLD),
        ),
    ];

    if !args.is_empty() {
        parts.push(Span::styled(
            format!(" {}", args.trim()),
            Style::default().fg(style::TEXT3),
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
                wrap_spans(&mut out, &spans, body_width, "  ");
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
            _ => {}
        }
    }

    out
}

struct InlineStyle {
    bold: u32,
    italic: u32,
    strike: u32,
    fg: Vec<ratatui::style::Color>,
}

impl InlineStyle {
    fn new() -> Self {
        Self { bold: 0, italic: 0, strike: 0, fg: Vec::new() }
    }

    fn current_style(&self) -> Style {
        let mut s = Style::default();
        if self.bold > 0 { s = s.add_modifier(Modifier::BOLD); }
        if self.italic > 0 { s = s.add_modifier(Modifier::ITALIC); }
        if self.strike > 0 { s = s.add_modifier(Modifier::CROSSED_OUT); }
        if let Some(c) = self.fg.last() { s = s.fg(*c); }
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
            Some(Event::Text(t)) => { buf.push_str(&t); }
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
                    Tag::Link { .. } => { istyle.fg.push(style::BLUE); }
                    _ => {}
                }
            }
            Some(Event::End(tag_end)) => {
                flush_buf(&mut spans, &mut buf, &istyle);
                match tag_end {
                    TagEnd::Emphasis => istyle.italic = istyle.italic.saturating_sub(1),
                    TagEnd::Strong => istyle.bold = istyle.bold.saturating_sub(1),
                    TagEnd::Strikethrough => istyle.strike = istyle.strike.saturating_sub(1),
                    TagEnd::Link => { istyle.fg.pop(); }
                    _ => {
                        if let TagEnd::Paragraph | TagEnd::Heading(_) = tag_end {
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
            Some(Event::SoftBreak | Event::HardBreak) => { buf.push(' '); }
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
                if !line.is_empty() { lines.push(line); }
                break;
            }
            Some(Event::SoftBreak | Event::HardBreak) => { lines.push(std::mem::take(&mut line)); }
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
            Some(Event::Start(Tag::BlockQuote(_))) => { depth += 1; let _ = events.next(); }
            Some(Event::End(TagEnd::BlockQuote(_))) => { depth -= 1; let _ = events.next(); if depth == 0 { break; } }
            Some(Event::Start(Tag::Paragraph)) => {
                let _ = events.next();
                let spans = collect_inline_spans(events);
                let mut quote_spans = vec![Span::styled("│ ", Style::default().fg(style::YELLOW))];
                for s in &spans {
                    quote_spans.push(Span::styled(s.content.clone(), s.style.fg(style::YELLOW)));
                }
                wrap_spans(&mut out, &quote_spans, body_width, "  ");
            }
            _ => { let _ = events.next(); }
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
                let raw_spans = collect_inline_spans(events);
                let text: String = raw_spans.iter().map(|s| s.content.as_ref()).collect();
                let prefix = "  • ";
                let wrapped = wrap_line(&text, body_width.saturating_sub(4));
                for (idx, w) in wrapped.iter().enumerate() {
                    if idx == 0 {
                        out.push(Line::from(vec![
                            Span::styled(prefix, Style::default().fg(style::BLUE)),
                            Span::raw(w.to_string()),
                        ]));
                    } else {
                        out.push(Line::from(vec![
                            Span::raw("    "),
                            Span::raw(w.to_string()),
                        ]));
                    }
                }
            }
            Some(Event::End(TagEnd::List(_))) => { list_depth -= 1; if list_depth == 0 { break; } }
            Some(Event::Start(Tag::List(_))) => { list_depth += 1; }
            None => break,
            _ => {}
        }
    }
}

fn render_heading(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], level: HeadingLevel, body_width: usize) {
    let level_num: u8 = match level { HeadingLevel::H1 => 1, HeadingLevel::H2 => 2, _ => 3 };
    let header_color = match level_num { 1 => style::YELLOW, 2 => style::BLUE, _ => style::TEXT3 };
    let prefix = format!("  {} ", "#".repeat(level_num as usize));
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let wrapped = wrap_line(&text, body_width);
    for w in wrapped {
        out.push(Line::from(vec![
            Span::styled(prefix.clone(), Style::default().fg(style::TEXT3)),
            Span::styled(w, Style::default().fg(header_color).add_modifier(Modifier::BOLD)),
        ]));
    }
}

fn render_hr(out: &mut Vec<Line<'static>>, body_width: usize) {
    let hr_width = body_width.min(40);
    out.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("─".repeat(hr_width), Style::default().fg(style::OVERLAY0)),
    ]));
}

fn flush_code_block(out: &mut Vec<Line<'static>>, lang: &str, code_lines: &[String]) {
    if !lang.is_empty() {
        out.push(Line::from(vec![Span::raw("  "), Span::styled(format!("┌─ {} ", lang), Style::default().fg(style::TEXT3))]));
    } else {
        out.push(Line::from(vec![Span::raw("  "), Span::styled("┌───", Style::default().fg(style::TEXT3))]));
    }
    for code_line in code_lines {
        out.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(style::TEXT3)),
            Span::styled(code_line.clone(), Style::default().fg(style::TEXT)),
        ]));
    }
    out.push(Line::from(Span::styled("  └───", Style::default().fg(style::TEXT3))));
}

fn wrap_spans(out: &mut Vec<Line<'static>>, spans: &[Span<'static>], max_width: usize, indent: &str) {
    let full_text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    let chars: Vec<char> = full_text.chars().collect();
    let indent_len = indent.chars().count();
    if chars.is_empty() { return; }
    let avail = max_width.saturating_sub(indent_len);
    if chars.len() <= avail {
        let mut row = vec![Span::raw(indent.to_string())];
        for s in spans { row.push(Span::styled(s.content.clone(), s.style)); }
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
        let end = if let Some(space) = chars[start..start + avail].iter().rposition(|&c| c == ' ') { start + space } else { start + avail };
        let mut row = vec![Span::raw(indent.to_string())];
        row.extend(make_spans_for_range(spans, start, end));
        out.push(Line::from(row));
        start = end + 1;
        while start < len && chars[start] == ' ' { start += 1; }
    }
}

fn make_spans_for_range(all: &[Span<'static>], start: usize, end: usize) -> Vec<Span<'static>> {
    if start >= end { return Vec::new(); }
    let mut result = Vec::new();
    let mut pos = 0usize;
    for s in all {
        let span_len = s.content.len();
        let span_start = pos;
        let span_end = pos + span_len;
        if span_end <= start { pos = span_end; continue; }
        if span_start >= end { break; }
        let byte_start = start.saturating_sub(span_start);
        let byte_end = if end < span_end { end - span_start } else { span_len };
        if byte_start < byte_end {
            let safe_start = char_boundary_clamp(&s.content, byte_start);
            let safe_end = if byte_end <= s.content.len() { char_boundary_clamp(&s.content, byte_end) } else { s.content.len() };
            if safe_start < safe_end {
                result.push(Span::styled(s.content[safe_start..safe_end].to_string(), s.style));
            }
        }
        pos = span_end;
    }
    result
}

fn char_boundary_clamp(s: &str, byte_pos: usize) -> usize {
    if byte_pos >= s.len() { return s.len(); }
    if s.is_char_boundary(byte_pos) { return byte_pos; }
    s.char_indices().map(|(i, _)| i).take_while(|&i| i < byte_pos).last().unwrap_or(0)
}

fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.len() <= max_width { return vec![line.to_string()]; }
    let mut result = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= max_width { result.push(chars[start..].iter().collect()); break; }
        let end = if let Some(space) = chars[start..(start + max_width).min(chars.len())].iter().rposition(|&c| c == ' ') { start + space } else { (start + max_width).min(chars.len()) };
        result.push(chars[start..end].iter().collect());
        start = end;
        while start < chars.len() && chars[start] == ' ' { start += 1; }
    }
    result
}

pub(crate) fn char_idx_to_byte_idx(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map(|(byte_idx, _)| byte_idx).unwrap_or(s.len())
}

pub(crate) fn display_width_up_to(s: &str, char_idx: usize) -> usize {
    s.chars().take(char_idx).map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wrap_line_short() { assert_eq!(wrap_line("hello", 80), vec!["hello"]); }

    #[test]
    fn test_wrap_line_long() {
        let result = wrap_line("abc def ghi jkl mno pqr stu vwx yz", 12);
        assert!(result.len() > 1);
    }

    #[test]
    fn test_render_markdown_empty() { assert!(render_markdown("", 80).is_empty()); }

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
}
