//! Chat area rendering — message list + popup + borderless input.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

use super::state::AppState;
use super::style;
use crate::tui::chat_lines::{char_idx_to_byte_idx, display_width_up_to};

pub(crate) fn render_chat(f: &mut Frame, area: Rect, state: &AppState, visible_lines: &[Line<'static>]) {
    let input_lines = state.ui.input.lines().count().clamp(1, 5) as u16;
    let input_height = input_lines.max(1);
    let pop_h = crate::tui::popup::popup_height(state);

    let mut constraints = vec![Constraint::Min(0)];
    if pop_h > 0 { constraints.push(Constraint::Length(pop_h)); }
    constraints.push(Constraint::Length(input_height));

    let chunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);

    let msg_area = chunks[0];
    f.render_widget(
        Paragraph::new(ratatui::text::Text::from(visible_lines.to_vec())).wrap(Wrap { trim: false }),
        msg_area,
    );

    if pop_h > 0 { crate::tui::popup::render_popup(f, chunks[1], state); }

    let input_idx = if pop_h > 0 { 2 } else { 1 };
    let input_area = chunks[input_idx];
    let is_focused = state.ui.focus == crate::tui::state::Focus::Input;

    let prompt = Span::styled("> ", Style::default().fg(if is_focused { style::BLUE } else { style::OVERLAY0 }).add_modifier(ratatui::style::Modifier::BOLD));
    let input_style = if is_focused { style::value_style() } else { style::hint_style() };
    let placeholder = "type / for commands";

    let input_display = if state.ui.input.is_empty() && is_focused {
        Paragraph::new(Line::from(vec![prompt, Span::styled(placeholder, input_style)]))
    } else if state.ui.input.is_empty() {
        Paragraph::new(Line::from(vec![Span::styled("> ", Style::default().fg(style::OVERLAY0)), Span::styled(placeholder, style::hint_style())]))
    } else {
        Paragraph::new(Line::from(vec![prompt, Span::styled(state.ui.input.as_str(), input_style)]))
    };

    f.render_widget(input_display, input_area);

    if is_focused && state.active_dialog.is_none() {
        if !state.ui.input.is_empty() {
            let byte_idx = char_idx_to_byte_idx(&state.ui.input, state.ui.input_cursor);
            let line_start_byte = state.ui.input[..byte_idx].rfind('\n').map(|pos| pos + 1).unwrap_or(0);
            let chars_on_this_line = state.ui.input[line_start_byte..byte_idx].chars().count();
            let line_width = display_width_up_to(&state.ui.input[line_start_byte..], chars_on_this_line);
            let cursor_x = input_area.x + 2 + line_width as u16;
            let line_no = state.ui.input[..byte_idx].lines().count().saturating_sub(1);
            let cursor_y = input_area.y + line_no as u16;
            f.set_cursor_position((cursor_x.min(input_area.right().saturating_sub(1)), cursor_y.min(input_area.bottom().saturating_sub(1))));
        } else {
            f.set_cursor_position((input_area.x + 2, input_area.y));
        }
    }
}
