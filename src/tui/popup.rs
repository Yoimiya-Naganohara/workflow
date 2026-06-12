//! Inline popup rendering — commands only.
//!
//! Commands render above the input box (like autocomplete).
//! Provider, Key, ModelPicker, and CustomWizard use overlay mode.
//!
//! Layout inside the chat panel:
//! ```text
//! ┌─ chat messages ──────────────────┐
//! │ ...                               │
//! │                                   │
//! ├─ popup (conditional) ────────────┤
//! │  Commands                        │
//! ├─ input box ──────────────────────┤
//! │ [Input ...                        │
//! └───────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    text::Span,
    widgets::{Row, Table, TableState},
};

use crate::tui::commands::COMMANDS;
use crate::tui::state::{AppState, Focus};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    if has_command_popup(state) {
        let prefix = state.ui.input.trim().to_lowercase();
        let count = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).count();
        (count.min(6) as u16 + 2).min(8)
    } else {
        0
    }
}

/// Whether the command autocomplete popup should be shown.
fn has_command_popup(state: &AppState) -> bool {
    state.active_dialog.is_none()
        && state.ui.focus == Focus::Input
        && state.ui.input.starts_with('/')
        && !state.ui.input.trim().is_empty()
}

/// Render the command autocomplete popup into `area` (which sits above the input box).
/// Called by `render_chat`.  `area` must be non-empty.
pub(crate) fn render_popup(f: &mut Frame, area: Rect, state: &AppState) {
    if has_command_popup(state) {
        render_command_popup(f, area, state);
    }
}

// ──────────────────────────────────────────────
//  Command autocomplete popup
// ──────────────────────────────────────────────

fn render_command_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let prefix = state.ui.input.trim().to_lowercase();
    let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
    if matches.is_empty() {
        return;
    }

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

    let mut table_state = TableState::default();
    let sel = state.ui.command_popup_selection.min(matches.len().saturating_sub(1));
    table_state.select(Some(sel));

    f.render_stateful_widget(
        Table::new(rows, [ratatui::layout::Constraint::Length(max_cmd_len as u16), ratatui::layout::Constraint::Min(0)])
            .block(style::panel("Commands"))
            .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}
