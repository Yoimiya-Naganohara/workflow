//! Inline popup rendering — commands, provider picker, and key input.
//!
//! These popups render above the input box (like the command autocomplete)
//! rather than as centered modal overlays.
//!
//! Layout inside the chat panel:
//! ```text
//! ┌─ chat messages ──────────────────┐
//! │ ...                               │
//! │                                   │
//! ├─ popup (conditional) ────────────┤
//! │  Commands / Provider / Key input  │
//! ├─ input box ──────────────────────┤
//! │ [Input ...                        │
//! └───────────────────────────────────┘
//! ```

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Block, Borders, Paragraph, Row, Table, TableState},
};

use crate::models::filter_providers;
use crate::tui::commands::COMMANDS;
use crate::tui::dialogs::ActiveDialog;
use crate::tui::state::{AppState, Focus};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    if has_command_popup(state) {
        // Count matching commands (capped at 6) + header + border
        let prefix = state.ui.input.trim().to_lowercase();
        let count = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).count();
        (count.min(6) as u16 + 2).min(8)
    } else if let Some(dialog) = &state.active_dialog {
        if !dialog.is_overlay() {
            use crate::tui::dialogs::ActiveDialog::*;
            match dialog {
                Provider(d) => {
                    // Search box (3) + separator (1) + list rows (capped) + border (2)
                    let items = filter_providers(state.core.models.providers(), &d.search_query).len();
                    let list_h = (items.min(6) as u16).max(1);
                    (list_h + 4 + 2).min(12) // +4 for header+search+sep, +2 border
                }
                Key(_) => 10, // fixed height
                _ => 0,
            }
        } else {
            0
        }
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

/// Render the appropriate inline popup into `area` (which sits above the input box).
/// Called by `render_chat`.  `area` must be non-empty.
pub(crate) fn render_popup(f: &mut Frame, area: Rect, state: &AppState) {
    if has_command_popup(state) {
        render_command_popup(f, area, state);
    } else if let Some(dialog) = &state.active_dialog {
        if !dialog.is_overlay() {
            match dialog {
                ActiveDialog::Provider(d) => render_provider_popup(f, area, state, d),
                ActiveDialog::Key(d) => render_key_popup(f, area, state, d),
                _ => {}
            }
        }
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
        Table::new(rows, [Constraint::Length(max_cmd_len as u16), Constraint::Min(0)])
            .block(style::panel("Commands"))
            .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}

// ──────────────────────────────────────────────
//  Provider picker popup
// ──────────────────────────────────────────────

fn render_provider_popup(
    f: &mut Frame,
    area: Rect,
    state: &AppState,
    dialog: &crate::tui::dialogs::provider::ProviderDialog,
) {
    let inner = style::panel("Configure Provider").inner(area);
    f.render_widget(style::panel("Configure Provider"), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // search box
            Constraint::Length(1), // separator
            Constraint::Min(0),    // provider list
        ])
        .split(inner);

    // ── Search box ──
    let search_input = Paragraph::new(dialog.search_query.as_str())
        .style(style::value_style())
        .block(Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(style::ACTIVE)));
    f.render_widget(search_input, chunks[0]);
    // Cursor in search box
    let prefix_width = crate::tui::chat_lines::display_width_up_to(
        &dialog.search_query,
        dialog.search_cursor,
    );
    f.set_cursor_position((
        (chunks[0].x + prefix_width as u16 + 1).min(chunks[0].right().saturating_sub(2)),
        chunks[0].y + 1,
    ));

    style::render_separator(f, chunks[1]);

    // ── Provider list ──
    let filtered = filter_providers(state.core.models.providers(), &dialog.search_query);
    let show_custom = dialog.show_custom();
    let total_items = filtered.len() + if show_custom { 1 } else { 0 };

    if filtered.is_empty() && !show_custom {
        f.render_widget(
            Paragraph::new("No matching providers.").style(style::hint_style()),
            chunks[2],
        );
        return;
    }

    let max_name = filtered.iter().map(|p| p.name.len()).max().unwrap_or(0);
    let max_count = filtered
        .iter()
        .map(|p| format!("{} models", p.models.len()).len())
        .max()
        .unwrap_or(8);

    let mut rows: Vec<Row> = filtered
        .iter()
        .map(|p| {
            let is_configured = state.core.configured_providers.iter().any(|id| id == &p.id);
            Row::new(vec![
                Span::styled(
                    if is_configured { "✓" } else { "" },
                    Style::default().fg(style::SUCCESS),
                ),
                Span::styled(&p.name, style::value_style()),
                Span::styled(format!("{} models", p.models.len()), style::hint_style()),
            ])
        })
        .collect();

    if show_custom {
        rows.push(Row::new(vec![
            Span::raw(""),
            Span::styled("+ Add Custom Provider", Style::default().fg(style::ACTIVE)),
            Span::raw(""),
        ]));
    }

    let mut table_state = TableState::default();
    table_state.select(Some(dialog.selected_idx.min(total_items.saturating_sub(1))));
    f.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(1),
                Constraint::Length(max_name as u16 + 1),
                Constraint::Length(max_count as u16),
            ],
        )
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        chunks[2],
        &mut table_state,
    );
}

// ──────────────────────────────────────────────
//  Key input popup
// ──────────────────────────────────────────────

fn render_key_popup(
    f: &mut Frame,
    area: Rect,
    state: &AppState,
    dialog: &crate::tui::dialogs::key::KeyDialog,
) {
    let provider_name = state
        .core
        .models
        .providers()
        .iter()
        .find(|p| p.id == dialog.provider_id)
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let title = format!("API Key — {}", provider_name);
    let inner = style::panel(&title).inner(area);
    f.render_widget(style::panel(&title), area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(inner);

    // Input field
    let masked: String = dialog.input.chars().map(|_| '•').collect();
    let has_input = !dialog.input.is_empty();
    let display = Paragraph::new(if has_input {
        masked.as_str()
    } else {
        " Type or paste your API key…"
    })
    .style(if has_input {
        Style::default().fg(style::WARNING)
    } else {
        style::hint_style()
    })
    .block(style::input_box(has_input));
    f.render_widget(display, chunks[0]);

    // Cursor
    let cx = chunks[0].x + dialog.input.chars().count() as u16 + 1;
    let cy = chunks[0].y + 1;
    f.set_cursor_position((cx.min(chunks[0].right().saturating_sub(2)), cy));

    // Hint
    style::render_hint(
        f,
        chunks[1],
        if dialog.return_to_picker {
            "Enter to confirm (returns to model picker)  ·  Esc to cancel"
        } else {
            "Enter to confirm  ·  Esc to cancel"
        },
    );
}
