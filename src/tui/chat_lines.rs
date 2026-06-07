//! Chat message rendering — builds styled [`Line`] vectors from messages.
//!
//! Handles code-block fences, inline backtick highlighting, line
//! wrapping, and an animated thinking indicator.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::state::{AppState, MessageRole, MessageStatus};

/// Thinking animation frames (cycling dots).
const THINK_FRAMES: &[&str] = &[" ●   ", " ●●  ", " ●●● ", "  ●●●", "   ●●", "    ●"];

/// Build styled chat lines from the message list.
pub(crate) fn build_chat_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let content_width = width.max(20);
    let body_width = content_width.saturating_sub(4).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();

    for message in &state.messages {
        let (label, color) = match message.role {
            MessageRole::System => ("system", Color::DarkGray),
            MessageRole::User => ("user", Color::Cyan),
            MessageRole::Agent => ("agent", Color::Blue),
            MessageRole::Decision => ("decision", Color::Green),
        };

        // ── Header line ──
        let state_indicator = match message.status {
            MessageStatus::Thinking => Span::styled(
                THINK_FRAMES[state.think_frame as usize % THINK_FRAMES.len()],
                Style::default().fg(Color::Yellow),
            ),
            MessageStatus::Streaming => Span::styled(
                " ◉ ",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::SLOW_BLINK),
            ),
            MessageStatus::Completed => Span::styled(" ✓ ", Style::default().fg(Color::Green)),
            MessageStatus::Error => Span::styled(" ✗ ", Style::default().fg(Color::Red)),
        };

        lines.push(Line::from(vec![
            state_indicator,
            Span::styled(
                format!("[{}] ", message.timestamp),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(label, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        ]));

        // If thinking with no content yet, show a placeholder.
        if message.content.is_empty() && matches!(message.status, MessageStatus::Thinking) {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    "thinking…",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                ),
            ]));
            lines.push(Line::from(String::new()));
            continue;
        }

        // ── Content rendering with code blocks ──
        let mut in_code_block = false;
        let mut code_lang = String::new();
        let mut code_lines: Vec<String> = Vec::new();

        for raw_line in message.content.lines() {
            if raw_line.trim_start().starts_with("```") {
                if in_code_block {
                    // Close code block
                    flush_code_block(&mut lines, &code_lang, &code_lines);
                    code_lines.clear();
                    code_lang.clear();
                    in_code_block = false;
                } else {
                    // Open code block
                    in_code_block = true;
                    code_lang = raw_line.trim_start().trim_start_matches("```").trim().to_string();
                }
            } else if in_code_block {
                code_lines.push(raw_line.to_string());
            } else {
                // Wrap long text lines to body_width.
                let wrapped = wrap_line(raw_line, body_width);
                for w in wrapped {
                    let spans = render_text_line(&w);
                    lines.push(Line::from(
                        vec![Span::raw("  ")].into_iter().chain(spans).collect::<Vec<_>>(),
                    ));
                }
            }
        }

        // Flush remaining code block.
        if in_code_block {
            flush_code_block(&mut lines, &code_lang, &code_lines);
        }

        // Separator between messages.
        lines.push(Line::from(String::new()));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages yet."));
    }

    lines
}

/// Render a code block with a bordered style.
fn flush_code_block(lines: &mut Vec<Line<'static>>, lang: &str, code_lines: &[String]) {
    if !lang.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("┌─ {} ", lang), Style::default().fg(Color::DarkGray)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("┌───", Style::default().fg(Color::DarkGray)),
        ]));
    }
    for code_line in code_lines {
        lines.push(Line::from(vec![
            Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
        ]));
    }
    lines.push(Line::from(Span::styled("  └───", Style::default().fg(Color::DarkGray))));
}

/// Render a text line with inline backtick highlighting.
fn render_text_line(line: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = line.to_string();

    while !remaining.is_empty() {
        if let Some(start) = remaining.find('`') {
            if start > 0 {
                spans.push(Span::styled(remaining[..start].to_string(), Style::default()));
            }
            if let Some(end) = remaining[start + 1..].find('`') {
                let code = &remaining[start + 1..start + 1 + end];
                spans.push(Span::styled(
                    format!("`{}`", code),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
                ));
                remaining = remaining[start + 2 + end..].to_string();
            } else {
                spans.push(Span::styled(
                    remaining[start..].to_string(),
                    Style::default().fg(Color::Cyan),
                ));
                remaining.clear();
            }
        } else {
            spans.push(Span::styled(remaining.clone(), Style::default()));
            remaining.clear();
        }
    }
    spans
}

/// Wrap a line to at most `max_width` characters, splitting at word
/// boundaries when possible.
fn wrap_line(line: &str, max_width: usize) -> Vec<String> {
    if line.len() <= max_width {
        return vec![line.to_string()];
    }

    let mut result = Vec::new();
    let mut remaining = line;

    while !remaining.is_empty() {
        if remaining.len() <= max_width {
            result.push(remaining.to_string());
            break;
        }

        // Try to break at a word boundary.
        let break_at = if let Some(space) = remaining[..=max_width].rfind(' ') {
            space
        } else {
            max_width
        };

        result.push(remaining[..break_at].to_string());
        remaining = remaining[break_at..].trim_start();
    }

    result
}

/// Compute the display width of `s` up to the given character index
/// (for cursor positioning).
/// Convert a character index to a byte index in a UTF-8 string.
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
    fn test_think_frames_not_empty() {
        assert!(!THINK_FRAMES.is_empty());
        assert!(THINK_FRAMES.iter().all(|f| !f.is_empty()));
    }

    #[test]
    fn test_render_text_line_empty() {
        let spans = render_text_line("");
        assert!(spans.is_empty());
    }

    #[test]
    fn test_render_text_line_no_code() {
        let spans = render_text_line("hello world");
        assert_eq!(spans.len(), 1);
    }

    #[test]
    fn test_render_text_line_with_code() {
        let spans = render_text_line("use `foo` here");
        assert!(spans.len() >= 3);
    }
}
