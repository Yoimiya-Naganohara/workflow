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
    widgets::{Paragraph, Row, Table, TableState},
};

use crate::models::filter_providers;
use crate::tui::commands::COMMANDS;
use crate::tui::dialogs::ActiveDialog;
use crate::tui::state::{AppState, Focus};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    if has_command_popup(state) {
        let prefix = state.ui.input.trim().to_lowercase();
        let count = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).count();
        (count.min(6) as u16 + 2).min(8)
    } else if let Some(dialog) = &state.active_dialog {
        if !dialog.is_overlay() {
            use crate::tui::dialogs::ActiveDialog::*;
            match dialog {
                Provider(d) => {
                    let items = filter_providers(state.core.models.providers(), &d.search_query).len();
                    let list_h = (items.min(6) as u16).max(1) + 1; // providers + custom row
                    // 1 search + 1 separator + list_h + 2 borders
                    (list_h + 4).min(12)
                }
                Key(_) => 5, // 1 provider line + 1 input + 1 hint + 2 borders
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

    let filtered = filter_providers(state.core.models.providers(), &dialog.search_query);
    let show_custom = dialog.show_custom();
    let total_items = filtered.len() + if show_custom { 1 } else { 0 };

    // No search box — main input IS the search field.
    // Just show the list directly.
    if filtered.is_empty() && !show_custom {
        f.render_widget(
            Paragraph::new("No matching providers.").style(style::hint_style()),
            inner,
        );
        return;
    }

    let max_name = filtered.iter().map(|p| p.name.len()).max().unwrap_or(0);
    let max_count = filtered
        .iter()
        .map(|p| format!("{} models", p.models.len()).len())
        .max()
        .unwrap_or(8);

    // Status badge labels
    let status_conf = "configured";
    let status_key = "needs key";
    let status_na = "no auth";
    let max_status = status_conf.len().max(status_key.len()).max(status_na.len());

    let mut rows: Vec<Row> = filtered
        .iter()
        .map(|p| {
            let is_configured = state.core.configured_providers.iter().any(|id| id == &p.id);
            let needs_key = !p.env.is_empty();
            let (icon, status_label, status_style) = if is_configured {
                ("✓", status_conf, Style::default().fg(style::SUCCESS))
            } else if needs_key {
                ("", status_key, Style::default().fg(style::HINT))
            } else {
                ("", status_na, Style::default().fg(style::ACTIVE))
            };
            Row::new(vec![
                Span::styled(icon, Style::default().fg(style::SUCCESS)),
                Span::styled(&p.name, style::value_style()),
                Span::styled(format!("{} models", p.models.len()), style::hint_style()),
                Span::styled(status_label, status_style),
            ])
        })
        .collect();

    if show_custom {
        rows.push(Row::new(vec![
            Span::raw(""),
            Span::styled("+ Add Custom Provider", Style::default().fg(style::ACTIVE)),
            Span::raw(""),
            Span::raw(""),
        ]));
    }

    let mut table_state = TableState::default();
    table_state.select(Some(dialog.selected_idx.min(total_items.saturating_sub(1))));
    f.render_stateful_widget(
        Table::new(
            rows,
            [
                Constraint::Length(1),                   // icon
                Constraint::Length(max_name as u16 + 1), // name
                Constraint::Length(max_count as u16),    // model count
                Constraint::Length(max_status as u16),   // status badge
            ],
        )
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        inner,
        &mut table_state,
    );
}

// ──────────────────────────────────────────────
//  Key input popup
// ──────────────────────────────────────────────

fn render_key_popup(f: &mut Frame, area: Rect, state: &AppState, dialog: &crate::tui::dialogs::key::KeyDialog) {
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
        .constraints([
            Constraint::Length(1), // "Enter API key for ..."
            Constraint::Length(1), // input line
            Constraint::Length(1), // hint
        ])
        .split(inner);

    // Provider name line
    let info_line = format!("Enter API key for {}", provider_name);
    f.render_widget(Paragraph::new(Span::styled(info_line, style::hint_style())), chunks[0]);

    // Input line: "sk- <masked text>"
    let masked: String = dialog.input.chars().map(|_| '•').collect();
    let display_text = if dialog.input.is_empty() {
        "sk- …".to_string()
    } else {
        format!("sk- {}", masked)
    };
    f.render_widget(
        Paragraph::new(Span::styled(
            &display_text,
            if dialog.input.is_empty() {
                style::hint_style()
            } else {
                Style::default().fg(style::WARNING)
            },
        )),
        chunks[1],
    );
    // Cursor handled by main input — don't set cursor here

    // Hint line
    let hint = if dialog.return_to_picker {
        "Enter to confirm (returns to model picker)  ·  Esc to cancel"
    } else {
        "Enter to confirm  ·  Esc to back to providers"
    };
    f.render_widget(Paragraph::new(Span::styled(hint, style::hint_style())), chunks[2]);
}
