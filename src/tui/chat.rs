//! Chat area rendering — message list + input box.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Paragraph, Wrap},
};

use super::state::{AppState, Focus};
use super::style;
use crate::tui::chat_lines::{char_idx_to_byte_idx, display_width_up_to};

/// Render the chat messages pane and the input box within `area`.
///
/// `visible_lines` are pre-computed (word-wrapped, scrolled) chat lines.
pub(crate) fn render_chat(f: &mut Frame, area: Rect, state: &AppState, visible_lines: &[Line<'static>]) {
    let input_lines = state.ui.input.lines().count().clamp(1, 5) as u16;
    let input_height = input_lines + 2; // borders

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(input_height)])
        .split(area);

    // ── Chat messages ──
    let chat_block = Block::default();
    let inner = chat_block.inner(chunks[0]);
    f.render_widget(chat_block, chunks[0]);
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(visible_lines.to_vec())).wrap(Wrap { trim: false }),
        inner,
    );

    // ── Input box ──
    let is_focused = state.ui.focus == Focus::Input;
    let input_block = style::input_box(is_focused);
    let input_style = if is_focused {
        style::value_style()
    } else {
        style::hint_style()
    };
    let placeholder = "Type a message or /command… (Alt+Enter newline)";
    let input_display = if state.ui.input.is_empty() {
        Paragraph::new(placeholder).style(input_style)
    } else {
        Paragraph::new(state.ui.input.as_str()).style(style::value_style())
    };

    f.render_widget(input_display.block(input_block).wrap(Wrap { trim: false }), chunks[1]);

    // ── Cursor ──
    if is_focused && !state.ui.input.is_empty() {
        let prefix_width = display_width_up_to(&state.ui.input, state.ui.input_cursor);
        let cursor_x = chunks[1].x + prefix_width as u16 + 1;
        let line_no = state.ui.input[..char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor)]
            .lines()
            .count()
            .saturating_sub(1);
        let cursor_y = chunks[1].y + 1 + line_no as u16;
        f.set_cursor_position((
            cursor_x.min(chunks[1].right().saturating_sub(1)),
            cursor_y.min(chunks[1].bottom().saturating_sub(1)),
        ));
    } else if is_focused {
        f.set_cursor_position((chunks[1].x + 1, chunks[1].y + 1));
    }
}
