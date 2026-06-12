//! Unified design system for the TUI.
//!
//! Catppuccin Mocha palette, opencode-inspired layout.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Paragraph, Wrap},
};

// ── Catppuccin Mocha Palette ──

pub const BG: Color = Color::Rgb(33, 33, 33);
pub const BG2: Color = Color::Rgb(44, 44, 44);
pub const BG3: Color = Color::Rgb(24, 24, 24);
pub const TEXT: Color = Color::Rgb(205, 214, 244);
pub const TEXT2: Color = Color::Rgb(166, 173, 200);
pub const TEXT3: Color = Color::Rgb(127, 132, 156);
pub const BORDER: Color = Color::Rgb(75, 76, 92);
pub const BORDER_D: Color = Color::Rgb(49, 50, 68);
pub const BLUE: Color = Color::Rgb(137, 180, 250);
pub const MAUVE: Color = Color::Rgb(203, 166, 247);
pub const GREEN: Color = Color::Rgb(166, 227, 161);
pub const RED: Color = Color::Rgb(243, 139, 168);
pub const YELLOW: Color = Color::Rgb(249, 226, 175);
pub const OVERLAY0: Color = Color::Rgb(108, 112, 134);

// ── Backward-compatible aliases ──

pub const TITLE: Color = BLUE;
pub const HIGHLIGHT_FG: Color = BLUE;
pub const HIGHLIGHT_BG: Color = BG2;
pub const ACTIVE: Color = BLUE;
pub const INACTIVE: Color = OVERLAY0;
pub const LABEL: Color = TEXT3;
pub const VALUE: Color = TEXT;
pub const SUCCESS: Color = GREEN;
pub const WARNING: Color = YELLOW;
pub const ERROR: Color = RED;
pub const HINT: Color = TEXT3;
pub const PURPLE: Color = MAUVE;
pub const PROPOSAL_WIDTH: u16 = 36;

// ── Style Helpers ──

pub fn title_style() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn highlight_fg() -> Style {
    Style::default().fg(BLUE).add_modifier(Modifier::BOLD)
}

pub fn highlight_bg() -> Style {
    Style::default().bg(BG2)
}

pub fn label_style() -> Style {
    Style::default().fg(TEXT3)
}

pub fn value_style() -> Style {
    Style::default().fg(TEXT)
}

pub fn hint_style() -> Style {
    Style::default().fg(TEXT3)
}

// ── Widget Builders ──

pub fn panel<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(format!(" {} ", title), title_style()))
}

pub fn input_box<'a>(active: bool) -> Block<'a> {
    let border_color = if active { BLUE } else { BORDER };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
}

pub fn input_bar<'a>(active: bool) -> Block<'a> {
    let border_color = if active { BLUE } else { BORDER };
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            " Input ",
            Style::default().fg(border_color).add_modifier(Modifier::BOLD),
        ))
}

pub fn panel_chat<'a>(title: &str) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(BORDER))
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
            Paragraph::new(Span::styled("─".repeat(width), Style::default().fg(OVERLAY0))).wrap(Wrap { trim: false }),
            area,
        );
    }
}
