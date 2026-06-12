//! Unified design system for the TUI.
//!
//! Clean, minimal design inspired by Claude Code's aesthetic.
//! Uses standard terminal colors for maximum compatibility.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Wrap},
};

// ── Color Palette (standard terminal colors) ──

/// Border color for panels and dialogs.
pub const BORDER: Color = Color::DarkGray;
/// Title text color.
pub const TITLE: Color = Color::Cyan;
/// Foreground for highlighted list items.
pub const HIGHLIGHT_FG: Color = Color::Cyan;
/// Background for highlighted list items.
pub const HIGHLIGHT_BG: Color = Color::DarkGray;
/// Active input / search border color.
pub const ACTIVE: Color = Color::Cyan;
/// Inactive / subtle border color.
pub const INACTIVE: Color = Color::DarkGray;
/// Metadata label color.
pub const LABEL: Color = Color::DarkGray;
/// Primary value / content color.
pub const VALUE: Color = Color::White;
/// Success / confirm color.
pub const SUCCESS: Color = Color::Green;
/// Warning / in-progress color.
pub const WARNING: Color = Color::Yellow;
/// Error / failure color.
pub const ERROR: Color = Color::Red;
/// Hint / instruction text color.
pub const HINT: Color = Color::DarkGray;
/// Purple accent for tool calls.
pub const PURPLE: Color = Color::Magenta;
/// Light purple for tool call content.
pub const PURPLE_LIGHT: Color = Color::LightMagenta;

// ── Style Helpers ──

/// Style for panel / dialog titles.
pub fn title_style() -> Style {
    Style::default().fg(TITLE).add_modifier(Modifier::BOLD)
}

/// Style for a highlighted (selected) list item foreground.
pub fn highlight_fg() -> Style {
    Style::default().fg(HIGHLIGHT_FG).add_modifier(Modifier::BOLD)
}

/// Style for a highlighted (selected) list item background.
pub fn highlight_bg() -> Style {
    Style::default().bg(HIGHLIGHT_BG)
}

/// Style for metadata labels.
pub fn label_style() -> Style {
    Style::default().fg(LABEL)
}

/// Style for primary values.
pub fn value_style() -> Style {
    Style::default().fg(VALUE)
}

/// Style for hint / instruction text.
pub fn hint_style() -> Style {
    Style::default().fg(HINT)
}

// ── Widget Builders ──

/// A bordered panel with a title, using unified colors.
pub fn panel<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

/// A search / input box with an optional label.
pub fn input_box<'a>(active: bool) -> Block<'a> {
    let border_color = if active { ACTIVE } else { INACTIVE };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
}

/// Input bar with an `Input` label on the left.
pub fn input_bar<'a>(active: bool) -> Block<'a> {
    let border_color = if active { ACTIVE } else { INACTIVE };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            " Input ",
            Style::default().fg(border_color).add_modifier(Modifier::BOLD),
        ))
}

/// Chat panel border — plain corners.
pub fn panel_chat<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

/// Proposal/context panel border — rounded corners.
pub fn panel_proposal<'a>(title: &str) -> Block<'a> {
    use ratatui::widgets::BorderType;
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

/// Width of the proposal / context panel on the right.
pub const PROPOSAL_WIDTH: u16 = 36;

/// Style for diff additions.
pub fn diff_add_style() -> Style {
    Style::default().fg(SUCCESS)
}

/// Style for diff deletions.
pub fn diff_del_style() -> Style {
    Style::default().fg(ERROR)
}

/// Render a hint/instruction line at the bottom of a dialog or panel.
pub fn render_hint(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(Paragraph::new(Span::styled(text, hint_style())), area);
}

/// Render a thin horizontal separator line.
pub fn render_separator(f: &mut Frame, area: Rect) {
    let width = area.width as usize;
    if width > 0 {
        f.render_widget(
            Paragraph::new(Span::styled("─".repeat(width), Style::default().fg(INACTIVE))).wrap(Wrap { trim: false }),
            area,
        );
    }
}
