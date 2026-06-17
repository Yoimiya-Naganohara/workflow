//! Unified design system for the TUI.
//!
//! Modern color palette with semantic colors and consistent styling.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Wrap},
};

// ── Modern Color Palette ──

// Backgrounds
pub const BG_PRIMARY: Color = Color::Rgb(30, 30, 46); // Main background
pub const BG_SECONDARY: Color = Color::Rgb(36, 36, 54); // Panel backgrounds
pub const BG_TERTIARY: Color = Color::Rgb(24, 24, 38); // Input, status bar

// Text
pub const TEXT_PRIMARY: Color = Color::Rgb(205, 214, 244); // Main text
pub const TEXT_SECONDARY: Color = Color::Rgb(166, 173, 200); // Secondary text
pub const TEXT_MUTED: Color = Color::Rgb(108, 112, 134); // Muted text

// Borders
pub const BORDER_DEFAULT: Color = Color::Rgb(69, 71, 90);
pub const BORDER_FOCUSED: Color = Color::Rgb(137, 180, 250);

// Semantic Colors
pub const BLUE: Color = Color::Rgb(137, 180, 250); // Links, interactive
pub const GREEN: Color = Color::Rgb(166, 227, 161); // Success, added
pub const RED: Color = Color::Rgb(243, 139, 168); // Error, removed
pub const YELLOW: Color = Color::Rgb(249, 226, 175); // Warning
pub const PURPLE: Color = Color::Rgb(203, 166, 247); // Special, tool calls
pub const CYAN: Color = Color::Rgb(137, 220, 235); // Info, highlights

// ── Backward-compatible aliases ──

pub const BG: Color = BG_PRIMARY;
pub const BG2: Color = BG_SECONDARY;
pub const BG3: Color = BG_TERTIARY;
pub const TEXT: Color = TEXT_PRIMARY;
pub const TEXT2: Color = TEXT_SECONDARY;
pub const TEXT3: Color = TEXT_MUTED;
pub const BORDER: Color = BORDER_DEFAULT;
pub const BORDER_D: Color = BORDER_FOCUSED;
pub const MAUVE: Color = PURPLE;
pub const OVERLAY0: Color = TEXT_MUTED;

// Legacy aliases
pub const TITLE: Color = BLUE;
pub const HIGHLIGHT_FG: Color = BLUE;
pub const HIGHLIGHT_BG: Color = BG_SECONDARY;
pub const ACTIVE: Color = BLUE;
pub const INACTIVE: Color = TEXT_MUTED;
pub const LABEL: Color = TEXT_MUTED;
pub const VALUE: Color = TEXT_PRIMARY;
pub const SUCCESS: Color = GREEN;
pub const WARNING: Color = YELLOW;
pub const ERROR: Color = RED;
pub const HINT: Color = TEXT_MUTED;
pub const PROPOSAL_WIDTH: u16 = 36;

// ── Style Helpers ──

pub fn title_style() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn highlight_fg() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn highlight_bg() -> Style {
    Style::default().bg(BG_SECONDARY)
}

pub fn label_style() -> Style {
    Style::default().fg(TEXT_MUTED)
}

pub fn value_style() -> Style {
    Style::default().fg(TEXT_PRIMARY)
}

pub fn hint_style() -> Style {
    Style::default().fg(TEXT_MUTED)
}

pub fn success_style() -> Style {
    Style::default().fg(GREEN)
}

pub fn error_style() -> Style {
    Style::default().fg(RED)
}

pub fn warning_style() -> Style {
    Style::default().fg(YELLOW)
}

// ── Widget Builders ──

pub fn panel<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_DEFAULT))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

pub fn panel_focused<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER_FOCUSED))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

pub fn input_box<'a>(active: bool) -> Block<'a> {
    let border_color = if active { BLUE } else { BORDER_DEFAULT };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
}

pub fn input_bar<'a>(active: bool) -> Block<'a> {
    let border_color = if active { BLUE } else { BORDER_DEFAULT };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            " Input ",
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))
}

pub fn panel_chat<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::NONE)
        .border_style(Style::default().fg(BORDER_DEFAULT))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

pub fn diff_add_style() -> Style {
    Style::default().fg(GREEN)
}

pub fn diff_del_style() -> Style {
    Style::default().fg(RED)
}

pub fn render_hint(f: &mut Frame, area: Rect, text: &str) {
    f.render_widget(Paragraph::new(Span::styled(text, hint_style())), area);
}

pub fn render_separator(f: &mut Frame, area: Rect) {
    let width = area.width as usize;
    if width > 0 {
        f.render_widget(
            Paragraph::new(Span::styled(
                "─".repeat(width),
                Style::default().fg(TEXT_MUTED),
            ))
            .wrap(Wrap { trim: false }),
            area,
        );
    }
}
