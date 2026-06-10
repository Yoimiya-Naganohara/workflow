//! Dialog box rendering — provider picker, key input, model pool, command popup.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph},
};

use super::state::AppState;
use super::style;
use crate::models::filter_providers;

// ── Provider Dialog ──

pub(crate) fn render_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered = filter_providers(state.models.providers(), &state.provider_search_query);
    let custom_label = "➕ Add Custom Provider";
    let show_custom = state.provider_search_query.is_empty()
        || state.provider_search_query.to_lowercase().contains("custom")
        || state.provider_search_query.to_lowercase().contains("add")
        || custom_label
            .to_lowercase()
            .contains(&state.provider_search_query.to_lowercase());
    let total_items = filtered.len() + if show_custom { 1 } else { 0 };

    if total_items == 0 {
        let block = style::panel("Configure Provider");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.models.providers().is_empty() {
            "No providers loaded. Try again later."
        } else {
            "No matching providers."
        };
        f.render_widget(Paragraph::new(msg).style(style::hint_style()), inner);
        return;
    }

    let dialog_w = 64.min(area.width.saturating_sub(4));
    let search_h = 3u16;
    let list_h = total_items.min(12) as u16;
    let dialog_h = (list_h + 5 + search_h).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = style::panel("Configure Provider");
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(search_h), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    // Search box
    f.render_widget(
        Paragraph::new(state.provider_search_query.as_str())
            .style(style::value_style())
            .block(style::input_box(true)),
        chunks[0],
    );
    let prefix_width =
        crate::tui::chat_lines::display_width_up_to(&state.provider_search_query, state.provider_search_cursor);
    let cursor_x = chunks[0].x + prefix_width as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x, cursor_y));

    // Separator
    style::render_separator(f, chunks[1]);

    // Build list items
    let mut items: Vec<ListItem> = filtered
        .iter()
        .map(|p| {
            let count = p.models.len();
            let is_configured = state.configured_providers.iter().any(|id| id == &p.id);
            ListItem::new(Line::from(vec![
                if is_configured {
                    Span::styled("✓ ", Style::default().fg(style::SUCCESS))
                } else {
                    Span::raw("  ")
                },
                Span::styled(&p.name, style::value_style()),
                Span::raw("  "),
                Span::styled(format!("{} models", count), style::hint_style()),
            ]))
        })
        .collect();

    if show_custom {
        items.push(ListItem::new(Line::from(vec![
            Span::raw("  "),
            Span::styled(custom_label, Style::default().fg(style::ACTIVE)),
        ])));
    }

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_provider_idx.min(total_items.saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(style::highlight_fg())
            .highlight_style(style::highlight_bg())
            .highlight_symbol("▸ "),
        chunks[2],
        &mut list_state,
    );
}

// ── Key Dialog ──

pub(crate) fn render_key_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let provider_name = state
        .key_provider_id
        .as_ref()
        .and_then(|id| state.models.providers().iter().find(|p| &p.id == id))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let dialog_w = 54.min(area.width.saturating_sub(4));
    let dialog_h = 10;
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = style::panel(&format!("API Key — {}", provider_name));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(inner);

    // Input field
    let masked: String = state.key_input.chars().map(|_| '•').collect();
    let has_input = !state.key_input.is_empty();
    let input_display = Paragraph::new(if has_input {
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
    f.render_widget(input_display, chunks[0]);

    // Cursor
    let cursor_x = chunks[0].x + state.key_input.len() as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x.min(chunks[0].right().saturating_sub(2)), cursor_y));

    // Hint
    style::render_hint(f, chunks[1], "Enter to confirm  ·  Esc to cancel");
}

// ── Model Picker ──

pub(crate) fn render_model_picker(f: &mut Frame, area: Rect, state: &AppState) {
    let results = state
        .models
        .search_configured_models(&state.model_picker_search_query, &state.configured_providers);

    if results.is_empty() {
        let block = style::panel("Model Pool");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.configured_providers.is_empty() {
            "No providers configured. Use /connect first."
        } else {
            "No matching models."
        };
        f.render_widget(Paragraph::new(msg).style(style::hint_style()), inner);
        return;
    }

    let dialog_w = 64.min(area.width.saturating_sub(4));
    let search_h = 3u16;
    let list_h = results.len().min(12) as u16;
    let dialog_h = (list_h + 5 + search_h).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = style::panel("Model Pool");
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(search_h), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    // Search box
    f.render_widget(
        Paragraph::new(state.model_picker_search_query.as_str())
            .style(style::value_style())
            .block(style::input_box(true)),
        chunks[0],
    );
    let prefix_width =
        crate::tui::chat_lines::display_width_up_to(&state.model_picker_search_query, state.model_picker_search_cursor);
    let cursor_x = chunks[0].x + prefix_width as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x, cursor_y));

    // Separator
    style::render_separator(f, chunks[1]);

    // Build list items
    let items: Vec<ListItem> = results
        .iter()
        .map(|(p, m)| {
            let badge = m.capability_badge();
            let is_selected = state
                .selected_models
                .iter()
                .any(|sm| sm.provider_id == p.id && sm.model_id == m.id);
            ListItem::new(Line::from(vec![
                if is_selected {
                    Span::styled("✓ ", Style::default().fg(style::SUCCESS))
                } else {
                    Span::raw("  ")
                },
                Span::styled(&m.name, style::value_style()),
                Span::raw(" "),
                Span::styled(badge, style::hint_style().italic()),
                Span::raw(" "),
                Span::styled(&p.name, style::hint_style()),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(
        state.selected_model_picker_idx.min(results.len().saturating_sub(1)),
    ));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(style::highlight_fg())
            .highlight_style(style::highlight_bg())
            .highlight_symbol("▸ "),
        chunks[2],
        &mut list_state,
    );
}

// ── Custom Provider Wizard ──

pub(crate) fn render_custom_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let steps = ["Provider Name", "API Base URL", "API Key", "Model IDs"];
    let prompts = [
        "Enter a name for your custom provider:",
        "Enter the API base URL (e.g. https://api.example.com/v1):",
        "Enter the API key (leave empty for no auth):",
        "Enter model ID(s) (comma-separated, e.g. gpt-4,claude-3):",
    ];
    let step = state.custom_step.min(steps.len() - 1);
    let total_steps = steps.len();

    let dialog_w = 66.min(area.width.saturating_sub(4));
    let dialog_h = 11;
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = style::panel(&format!("Custom Provider — Step {}/{}", step + 1, total_steps));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Length(1)])
        .split(inner);

    // ── Step progress ──
    let mut progress_spans: Vec<Span> = Vec::new();
    for (i, label) in steps.iter().enumerate() {
        if i > 0 {
            progress_spans.push(Span::styled(" ─ ", style::hint_style()));
        }
        let (icon, color) = if i < step {
            ("●", style::SUCCESS)
        } else if i == step {
            ("●", style::ACTIVE)
        } else {
            ("○", style::INACTIVE)
        };
        progress_spans.push(Span::styled(format!("{} {}", icon, label), Style::default().fg(color)));
    }
    f.render_widget(Paragraph::new(Line::from(progress_spans)), chunks[0]);

    // ── Summary banner ──
    let mut summary_parts: Vec<String> = Vec::new();
    if !state.custom_name.is_empty() {
        summary_parts.push(format!("Name: {}", state.custom_name));
    }
    if !state.custom_url.is_empty() {
        let url_display = if state.custom_url.len() > 40 {
            format!("{}…", &state.custom_url[..37])
        } else {
            state.custom_url.clone()
        };
        summary_parts.push(format!("URL: {}", url_display));
    }
    if !state.custom_key.is_empty() {
        summary_parts.push("Key: ••••••••".to_string());
    }

    let summary_text: &str = if summary_parts.is_empty() {
        prompts[step]
    } else {
        // Leak for lifetime — this is a short-lived render function.
        // In practice, the string lives long enough for the Frame.
        let joined = summary_parts.join("  │  ");
        Box::leak(joined.into_boxed_str())
    };

    // ── Input ──
    let has_previous = !summary_parts.is_empty();
    let has_input = !state.custom_input.is_empty();

    let input_display = if !has_input {
        Paragraph::new(Span::styled(summary_text, style::hint_style())).block(style::input_box(has_previous))
    } else {
        Paragraph::new(state.custom_input.as_str())
            .style(style::value_style())
            .block(style::input_box(true))
    };
    f.render_widget(input_display, chunks[1]);

    // Cursor
    let prefix_width = crate::tui::chat_lines::display_width_up_to(&state.custom_input, state.custom_cursor);
    let cursor_x = chunks[1].x + prefix_width as u16 + 1;
    let cursor_y = chunks[1].y + 1;
    f.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));

    // ── Hint ──
    let hint = if step <= 1 {
        "Enter to continue  ·  Esc to cancel"
    } else if step == total_steps - 1 {
        "Enter to confirm and save  ·  Esc to cancel"
    } else {
        "Enter to continue  ·  Esc to cancel"
    };
    style::render_hint(f, chunks[2], hint);
}

// ── Command Popup ──

pub(crate) fn render_command_popup(f: &mut Frame, chat_area: Rect, state: &AppState) {
    if state.focus != super::state::Focus::Input || !state.input.starts_with('/') {
        return;
    }

    let prefix = state.input.trim().to_lowercase();
    let matches: Vec<_> = super::state::COMMANDS
        .iter()
        .filter(|(cmd, _)| cmd.starts_with(&prefix))
        .collect();

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
    let popup_w = 50.min(chat_area.width.saturating_sub(4));
    let x = chat_area.x;
    let y = chat_area.y + chat_area.height.saturating_sub(popup_h + 3);

    let mut list_state = ListState::default();
    list_state.select(Some(state.command_popup_selection.min(matches.len().saturating_sub(1))));
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
