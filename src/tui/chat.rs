//! Chat area rendering — message list + popup + input box.
//!
//! Layout:
//! ```text
//! ┌─ chat messages ─────────────────────┐
//! │ ...                                  │
//! │                                      │
//! ├─ popup (conditional) ───────────────┤
//! │  Commands / Provider picker / Key    │
//! ├─ input box ─────────────────────────┤
//! │ [Input ...                           │
//! └──────────────────────────────────────┘
//! ```
//! The popup section is only present when the command autocomplete
//! is visible or a popup-style dialog (Provider, Key) is active.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
    widgets::{Block, Paragraph, Wrap},
};

use super::state::AppState;
use super::style;
use crate::tui::chat_lines::{char_idx_to_byte_idx, display_width_up_to};

/// Render the chat messages pane, optional popup, and input box within `area`.
///
/// `visible_lines` are pre-computed (word-wrapped, scrolled) chat lines.
pub(crate) fn render_chat(f: &mut Frame, area: Rect, state: &AppState, visible_lines: &[Line<'static>]) {
    let input_lines = state.ui.input.lines().count().clamp(1, 5) as u16;
    let input_height = input_lines + 2; // borders
    let pop_h = crate::tui::popup::popup_height(state);

    let mut constraints = vec![Constraint::Min(0)];
    if pop_h > 0 {
        constraints.push(Constraint::Length(pop_h));
    }
    constraints.push(Constraint::Length(input_height));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    // ── Chat messages ──
    let msg_area = chunks[0];
    let chat_block = Block::default();
    let inner = chat_block.inner(msg_area);
    f.render_widget(chat_block, msg_area);
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(visible_lines.to_vec())).wrap(Wrap { trim: false }),
        inner,
    );

    // ── Popup (commands, providers, or key input) ──
    if pop_h > 0 {
        let popup_idx = 1;
        crate::tui::popup::render_popup(f, chunks[popup_idx], state);
    }

    // ── Input box ──
    let input_idx = if pop_h > 0 { 2 } else { 1 };
    let input_area = chunks[input_idx];
    let is_focused = state.ui.focus == crate::tui::state::Focus::Input;
    let input_block = style::input_bar(is_focused);
    let input_style = if is_focused {
        style::value_style()
    } else {
        style::hint_style()
    };
    let placeholder = "Type a message or /command… (Alt+Enter newline)";
    let input_display = if state.ui.input.is_empty() && is_focused {
        Paragraph::new(placeholder).style(input_style)
    } else if state.ui.input.is_empty() {
        Paragraph::new(" ").style(style::hint_style())
    } else {
        Paragraph::new(state.ui.input.as_str()).style(input_style)
    };
    f.render_widget(input_display.block(input_block).wrap(Wrap { trim: false }), input_area);

    // ── Cursor (only when no popup dialog is active — popup.rs handles its own cursor) ──
    if is_focused && !state.ui.input.is_empty() && state.active_dialog.is_none() {
        let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
        let line_start_byte = state.ui.input[..byte_idx].rfind('\n').map(|pos| pos + 1).unwrap_or(0);
        let chars_on_this_line = state.ui.input[line_start_byte..byte_idx].chars().count();
        let line_width = display_width_up_to(&state.ui.input[line_start_byte..], chars_on_this_line);
        let cursor_x = input_area.x + line_width as u16 + 1;
        let line_no = state.ui.input[..byte_idx].lines().count().saturating_sub(1);
        let cursor_y = input_area.y + 1 + line_no as u16;
        f.set_cursor_position((
            cursor_x.min(input_area.right().saturating_sub(1)),
            cursor_y.min(input_area.bottom().saturating_sub(1)),
        ));
    } else if is_focused && state.active_dialog.is_none() {
        f.set_cursor_position((input_area.x + 1, input_area.y + 1));
    }
}
