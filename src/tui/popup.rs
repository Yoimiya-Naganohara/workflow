//! Inline popup rendering — all popups render above the input box.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Row, Table, TableState},
};

use crate::models::filter_providers;
use crate::tui::commands::COMMANDS;
use crate::tui::state::{AppState, PopupMode};

use crate::tui::style;

/// Height reserved for the inline popup. 0 when no popup is shown.
pub(crate) fn popup_height(state: &AppState) -> u16 {
    match &state.popup_mode {
        PopupMode::None => 0,
        PopupMode::Commands => {
            let prefix = state.ui.input.trim().to_lowercase();
            let count = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).count();
            (count.min(6) as u16 + 2).min(8)
        }
        PopupMode::SubCommand { items, .. } => {
            (items.len().min(8) as u16 + 2).min(10)
        }
        PopupMode::Providers => {
            let count = filter_providers(state.core.models.providers(), &state.ui.input).len();
            ((count.min(8) as u16 + 1) + 1).min(12)
        }
        PopupMode::KeyInput => 5,
        PopupMode::ModelPicker => {
            let count = state.core.models.search_configured_models(&state.ui.input, &state.core.configured_providers).len();
            ((count.min(8) as u16) + 1).min(10)
        }
    }
}

/// Render the appropriate inline popup.
pub(crate) fn render_popup(f: &mut Frame, area: Rect, state: &AppState) {
    match &state.popup_mode {
        PopupMode::None => {}
        PopupMode::Commands => render_command_popup(f, area, state),
        PopupMode::SubCommand { parent, items } => render_subcommand_popup(f, area, state, parent, items),
        PopupMode::Providers => render_provider_popup(f, area, state),
        PopupMode::KeyInput => render_key_popup(f, area, state),
        PopupMode::ModelPicker => render_model_popup(f, area, state),
    }
}

// ── Command popup ──

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
    let sel = state.popup_selected.min(matches.len().saturating_sub(1));
    table_state.select(Some(sel));

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(max_cmd_len as u16),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .block(style::panel("Commands"))
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}

fn render_subcommand_popup(f: &mut Frame, area: Rect, state: &AppState, parent: &str, items: &[(String, String)]) {
    if items.is_empty() {
        return;
    }

    let max_name_len = items.iter().map(|(name, _)| name.len()).max().unwrap_or(10);
    let rows: Vec<Row> = items
        .iter()
        .map(|(name, desc)| {
            Row::new(vec![
                Span::styled(name.as_str(), Style::default().fg(style::ACTIVE)),
                Span::styled(desc.as_str(), style::hint_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    let sel = state.popup_selected.min(items.len().saturating_sub(1));
    table_state.select(Some(sel));

    let title = format!("{} · sub-commands", parent);
    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(max_name_len as u16),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .block(style::panel(&title))
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        area,
        &mut table_state,
    );
}

// ── Provider popup ──

fn render_provider_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered = filter_providers(state.core.models.providers(), &state.ui.input);
    let show_custom = state.ui.input.is_empty()
        || state.ui.input.to_lowercase().contains("custom")
        || state.ui.input.to_lowercase().contains("add");
    let total_items = filtered.len() + if show_custom { 1 } else { 0 };

    if total_items == 0 {
        let block = style::panel("Providers");
        let inner = block.inner(area);
        f.render_widget(block, area);
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

    let mut rows: Vec<Row> = filtered
        .iter()
        .map(|p| {
            let is_configured = state.core.configured_providers.iter().any(|id| id == &p.id);
            let needs_key = !p.env.is_empty();
            let (icon, status_label, status_style) = if is_configured {
                ("\u{2713}", "configured", Style::default().fg(style::SUCCESS))
            } else if needs_key {
                ("", "needs key", style::hint_style())
            } else {
                ("", "no auth", Style::default().fg(style::ACTIVE))
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
    table_state.select(Some(state.popup_selected.min(total_items.saturating_sub(1))));

    let block = style::panel("Providers");
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(max_name as u16 + 1),
                ratatui::layout::Constraint::Length(max_count as u16),
                ratatui::layout::Constraint::Length(12),
            ],
        )
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        inner,
        &mut table_state,
    );
}

// ── Key input popup ──

fn render_key_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let provider_name = state
        .popup_key_provider
        .as_ref()
        .and_then(|pid| state.core.models.providers().iter().find(|p| p.id == *pid))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let block = style::panel(&format!("API Key \u{2014} {}", provider_name));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1)])
        .split(inner);

    // Masked input
    let masked: String = state.ui.input.chars().map(|_| '\u{2022}').collect();
    let has_input = !state.ui.input.is_empty();
    let display = if has_input {
        masked.as_str()
    } else {
        "Type or paste your API key\u{2026}"
    };
    let input_style = if has_input {
        Style::default().fg(style::WARNING)
    } else {
        style::hint_style()
    };
    f.render_widget(
        Paragraph::new(Span::styled(display, input_style)).block(style::input_box(has_input)),
        chunks[0],
    );

    // Cursor
    let cursor_x = chunks[0].x + state.ui.input.chars().count() as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x.min(chunks[0].right().saturating_sub(2)), cursor_y));

    // Hint
    style::render_hint(f, chunks[1], "Enter to confirm  \u{00b7}  Esc to cancel");
}

// ── Model picker popup ──

fn render_model_popup(f: &mut Frame, area: Rect, state: &AppState) {
    let results = state
        .core
        .models
        .search_configured_models(&state.ui.input, &state.core.configured_providers);

    if results.is_empty() {
        let block = style::panel("Models");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.core.configured_providers.is_empty() {
            "No providers configured. Use /connect first."
        } else {
            "No matching models."
        };
        f.render_widget(Paragraph::new(msg).style(style::hint_style()), inner);
        return;
    }

    let max_name = results.iter().map(|(_, m)| m.name.len()).max().unwrap_or(0);

    let rows: Vec<Row> = results
        .iter()
        .map(|(p, m)| {
            let is_selected = state
                .core
                .selected_models
                .iter()
                .any(|sm| sm.provider_id == p.id && sm.model_id == m.id);
            Row::new(vec![
                if is_selected {
                    Span::styled("\u{2713}", Style::default().fg(style::SUCCESS))
                } else {
                    Span::raw("")
                },
                Span::styled(&m.name, style::value_style()),
                Span::styled(&p.name, style::hint_style()),
            ])
        })
        .collect();

    let mut table_state = TableState::default();
    table_state.select(Some(state.popup_selected.min(results.len().saturating_sub(1))));

    let block = style::panel("Models");
    let inner = block.inner(area);
    f.render_widget(block, area);

    f.render_stateful_widget(
        Table::new(
            rows,
            [
                ratatui::layout::Constraint::Length(1),
                ratatui::layout::Constraint::Length(max_name as u16 + 1),
                ratatui::layout::Constraint::Min(0),
            ],
        )
        .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
        inner,
        &mut table_state,
    );
}
