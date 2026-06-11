//! Command popup — shows matching `/` commands while the user types.
//!
//! This is NOT a dialog (not in `ActiveDialog`).  It is input decoration
//! rendered inline above the input box when focus is Input and input starts with `/`.

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState},
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

    let items: Vec<ListItem> = matches
        .iter()
        .map(|(cmd, desc)| {
            ListItem::new(Line::from(vec![
                Span::styled(*cmd, Style::default().fg(style::ACTIVE)),
                Span::raw("  "),
                Span::styled(format!("— {}", desc), style::hint_style()),
            ]))
        })
        .collect();

    let popup_h = (matches.len() as u16).clamp(3, 6) + 2;
    let popup_w = 50u16.min(chat_area.width.saturating_sub(4));
    let x = chat_area.x;
    let y = chat_area.y + chat_area.height.saturating_sub(popup_h + 3);

    let mut list_state = ListState::default();
    list_state.select(Some(ui.command_popup_selection.min(matches.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .block(style::panel("Commands"))
            .highlight_style(style::highlight_fg())
            .highlight_style(style::highlight_bg())
            .highlight_symbol("▸ "),
        Rect::new(x, y, popup_w, popup_h),
        &mut list_state,
    );
}
