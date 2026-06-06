//! Chat message rendering — builds styled [`Line`] vectors from messages.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::keymap::Action;
use super::state::{AppState, MessageRole, MessageStatus};

/// Build styled chat lines from the message list.
pub(crate) fn build_chat_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let content_width = width.max(1);
    let _body_width = content_width.saturating_sub(2).max(1);
    let mut lines = Vec::new();

    for message in &state.messages {
        let (label, color) = match message.role {
            MessageRole::System => ("system", Color::DarkGray),
            MessageRole::User => ("user", Color::Cyan),
            MessageRole::Agent => ("agent", Color::Blue),
            MessageRole::Decision => ("decision", Color::Green),
        };

        let state_indicator = match message.status {
            MessageStatus::Thinking => Span::styled(
                " ◌ ",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK),
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

        // Render message content with code block highlighting
        let mut in_code_block = false;
        let mut code_lang = String::new();
        let mut code_lines: Vec<String> = Vec::new();

        for line in message.content.lines() {
            if line.trim_start().starts_with("```") {
                if in_code_block {
                    if !code_lang.is_empty() {
                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(
                                format!("{} ", code_lang),
                                Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                            ),
                        ]));
                    }
                    for code_line in code_lines.iter() {
                        lines.push(Line::from(vec![
                            Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
                        ]));
                    }
                    lines.push(Line::from(Span::styled("  └───", Style::default().fg(Color::DarkGray))));
                    code_lines.clear();
                    code_lang.clear();
                    in_code_block = false;
                } else {
                    in_code_block = true;
                    code_lang = line.trim_start().trim_start_matches("```").trim().to_string();
                }
            } else if in_code_block {
                code_lines.push(line.to_string());
            } else {
                let spans = render_text_line(line);
                lines.push(Line::from(
                    vec![Span::raw("  ")].into_iter().chain(spans).collect::<Vec<_>>(),
                ));
            }
        }
        // Flush remaining code block
        if in_code_block {
            if !code_lang.is_empty() {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        format!("{} ", code_lang),
                        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
            for code_line in code_lines.iter() {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(code_line.clone(), Style::default().fg(Color::Cyan)),
                ]));
            }
            lines.push(Line::from(Span::styled("  └───", Style::default().fg(Color::DarkGray))));
        }

        lines.push(Line::from(String::new()));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages yet."));
    }

    lines
}

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

pub(crate) fn display_width_up_to(s: &str, char_idx: usize) -> usize {
    s.chars()
        .take(char_idx)
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
        .sum()
}

pub(crate) fn format_action(action: &Action) -> String {
    match action {
        Action::Quit => "Quit the application",
        Action::CancelResponse => "Cancel current response",
        Action::ToggleStatusPanel => "Show/hide status panel",
        Action::MoveUp => "Move up / Previous item",
        Action::MoveDown => "Move down / Next item",
        Action::MoveLeft => "Move cursor left",
        Action::MoveRight => "Move cursor right",
        Action::ScrollUp => "Scroll chat up",
        Action::ScrollDown => "Scroll chat down",
        Action::ScrollToTop => "Scroll to top",
        Action::ScrollToBottom => "Scroll to bottom",
        Action::Confirm => "Confirm selection",
        Action::Cancel => "Cancel / Close dialog",
        Action::OpenModelPicker => "Open model picker",
        Action::OpenProviderDialog => "Open provider dialog",
        Action::SwitchPanel => "Switch panel",
        Action::SendMessage => "Send message",
        Action::TypeChar(_) => "Type character",
        Action::DeleteChar => "Delete character",
        Action::HistoryPrev => "Previous input history",
        Action::HistoryNext => "Next input history",
        Action::CommandPrev => "Previous command",
        Action::CommandNext => "Next command",
        Action::None => "",
    }
    .to_string()
}
