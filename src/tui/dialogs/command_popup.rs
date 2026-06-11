//! Command popup — shows matching `/` commands while the user types.
//!
//! This is NOT a dialog (not in `ActiveDialog`).  It is input decoration
//! rendered inline above the input box when focus is Input and input starts with `/`.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::Style,
    text::Span,
    widgets::{Row, Table, TableState},
};

use crate::tui::commands::COMMANDS;
use crate::tui::state::{CoreState, UiState};

use crate::tui::style;

/// Render the command autocomplete popup above the input box.
///
/// Only draws when focus is Input and the input starts with `/`.
pub fn render_command_popup(f: &mut Frame, chat_area: Rect, ui: &UiState, _core: &CoreState) {
    if ui.focus != crate::tui::state::Focus::Input || !ui.input.starts_with('/') {
        return;
    }

    let prefix = ui.input.trim().to_lowercase();
    let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();

    if matches.is_empty() {
        return;
    }
    // Use a Table with column constraints (Length + Min) instead of manual
    // string padding — the layout engine handles alignment.
    let max_cmd_len = matches.iter().map(|(cmd, _)| cmd.len()).max().unwrap_or(10);
    let rows: Vec<Row> = matches
        .iter()
        .map(|(cmd, desc)| {
            Row::new(vec![
                Span::styled(*cmd, Style::default().fg(style::ACTIVE)),
                Span::styled(*desc, style::hint_style()),
            ])
        })
        .collect();

    let popup_h = (matches.len() as u16).clamp(3, 6) + 2;
    // Compute width from content (command + gap + description), capped to chat area.
    let content_max = matches
        .iter()
        .map(|(cmd, desc)| cmd.len() + 4 + desc.len())
        .max()
        .unwrap_or(50);
    let popup_w = (content_max as u16).min(chat_area.width.saturating_sub(4));
    let x = chat_area.x;
    // Position the popup just above the input box (not at chat_area bottom).
    let input_height = ui.input.lines().count().clamp(1, 5) as u16 + 2;
    let input_top = chat_area.y + chat_area.height - input_height;
    let y = input_top.saturating_sub(popup_h + 1);

    let mut table_state = TableState::default();
    table_state.select(Some(ui.command_popup_selection.min(matches.len().saturating_sub(1))));
    f.render_stateful_widget(
        Table::new(rows, [Constraint::Length(max_cmd_len as u16), Constraint::Min(0)])
            .block(style::panel("Commands"))
            .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        Rect::new(x, y, popup_w, popup_h),
        &mut table_state,
    );
}
