//! Dialog box rendering — provider picker, key input, model picker, command popup.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use super::state::AppState;
use crate::models::filter_providers;

pub(crate) fn render_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered = filter_providers(state.models.providers(), &state.provider_search_query);

    if filtered.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Provider ")
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.models.providers().is_empty() {
            "No providers loaded. Try again later."
        } else {
            "No matching providers."
        };
        f.render_widget(Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)), inner);
        return;
    }

    let dialog_w = 60.min(area.width.saturating_sub(4));
    let search_h = 3u16;
    let list_h = filtered.len() as u16;
    let dialog_h = (list_h + 4 + search_h).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Select Provider ")
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(search_h), Constraint::Min(0)])
        .split(inner);

    let search_style = Style::default().fg(Color::Cyan);
    f.render_widget(
        Paragraph::new(state.provider_search_query.as_str())
            .style(search_style)
            .block(Block::default().borders(Borders::ALL).title("Search")),
        chunks[0],
    );
    let prefix_width =
        crate::tui::chat_lines::display_width_up_to(&state.provider_search_query, state.provider_search_cursor);
    let cursor_x = chunks[0].x + prefix_width as u16 + 1;
    let cursor_y = chunks[0].y + 1;
    f.set_cursor_position((cursor_x, cursor_y));

    let items: Vec<ListItem> = filtered
        .iter()
        .map(|p| {
            let count = p.models.len();
            let env = p.env.first().map(|e| e.as_str()).unwrap_or("no key");
            ListItem::new(Line::from(vec![
                Span::styled(&p.name, Style::default()),
                Span::raw("  "),
                Span::styled(format!("{} models", count), Style::default().fg(Color::DarkGray)),
                Span::raw("  "),
                Span::styled(format!("env: {}", env), Style::default().fg(Color::Yellow)),
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(state.selected_provider_idx.min(filtered.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .highlight_symbol("❯ "),
        chunks[1],
        &mut list_state,
    );
}

pub(crate) fn render_key_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let provider_name = state
        .key_provider_id
        .as_ref()
        .and_then(|id| state.models.providers().iter().find(|p| &p.id == id))
        .map(|p| p.name.as_str())
        .unwrap_or("Unknown");

    let dialog_w = 50.min(area.width.saturating_sub(4));
    let dialog_h = 7;
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" API Key for {} ", provider_name))
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let masked: String = state.key_input.chars().map(|_| '•').collect();
    f.render_widget(
        Paragraph::new(if masked.is_empty() {
            " (type and press Enter) "
        } else {
            &masked
        })
        .style(Style::default().fg(Color::Cyan)),
        inner,
    );

    // Show cursor at the end of input
    let cursor_x = inner.x + state.key_input.len() as u16 + 1;
    let cursor_y = inner.y;
    f.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));
}

pub(crate) fn render_model_picker(f: &mut Frame, area: Rect, state: &AppState) {
    let results = state.models.search_models(&state.model_picker_search_query);

    if results.is_empty() {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Select Model ")
            .style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let msg = if state.models.providers().is_empty() {
            "No providers loaded. Type /connect to fetch."
        } else {
            "No matching models."
        };
        f.render_widget(Paragraph::new(msg).style(Style::default().fg(Color::DarkGray)), inner);
        return;
    }

    let dialog_w = 60.min(area.width.saturating_sub(4));
    let search_h = 3u16;
    let list_h = results.len() as u16;
    let dialog_h = (list_h + 4 + search_h).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Select Model ")
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(search_h), Constraint::Min(0)])
        .split(inner);

    let search_style = Style::default().fg(Color::Cyan);
    f.render_widget(
        Paragraph::new(state.model_picker_search_query.as_str())
            .style(search_style)
            .block(Block::default().borders(Borders::ALL).title("Search")),
        chunks[0],
    );

    let items: Vec<ListItem> = results
        .iter()
        .map(|(p, m)| {
            let needs_key = !state.configured_providers.iter().any(|id| id == &p.id)
                && !crate::tui::controller::is_no_auth_provider(&p.id);
            ListItem::new(Line::from(vec![
                Span::styled(&p.name, Style::default().fg(Color::DarkGray)),
                Span::raw(" / "),
                Span::styled(&m.name, Style::default()),
                Span::raw("  "),
                if needs_key {
                    Span::styled("⌁", Style::default().fg(Color::Yellow))
                } else {
                    Span::raw("")
                },
                Span::raw("  "),
                if state
                    .selected_models
                    .iter()
                    .any(|sm| sm.provider_id == p.id && sm.model_id == m.id)
                {
                    Span::styled("✓", Style::default().fg(Color::Green))
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    let mut list_state = ListState::default();
    list_state.select(Some(
        state.selected_model_picker_idx.min(results.len().saturating_sub(1)),
    ));
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .highlight_symbol("❯ "),
        chunks[1],
        &mut list_state,
    );
}

pub(crate) fn render_custom_provider_dialog(f: &mut Frame, area: Rect, state: &AppState) {
    let steps = ["Provider Name", "API Base URL", "API Key", "Model IDs"];
    let prompts = [
        "Enter a name for your custom provider:",
        "Enter the API base URL (e.g. https://api.example.com/v1):",
        "Enter the API key (leave empty for no auth):",
        "Enter model ID(s) (comma-separated, e.g. gpt-4,claude-3):",
    ];
    let step = state.custom_step.min(steps.len() - 1);

    let dialog_w = 64.min(area.width.saturating_sub(4));
    let dialog_h = 12;
    let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
    let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
    let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Custom Provider — Step {}/{} ", step + 1, steps.len()))
        .style(Style::default().fg(Color::Cyan));
    let inner = block.inner(dialog_area);
    f.render_widget(block, dialog_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    // Summary of previous steps
    let mut summary = Vec::new();
    if !state.custom_name.is_empty() {
        summary.push(format!("Name: {}", state.custom_name));
    }
    if !state.custom_url.is_empty() {
        summary.push(format!("URL: {}", state.custom_url));
    }
    if !state.custom_key.is_empty() {
        summary.push("Key: ••••••••".to_string());
    }
    if !summary.is_empty() {
        f.render_widget(
            Paragraph::new(summary.join("  |  ")).style(Style::default().fg(Color::DarkGray)),
            chunks[0],
        );
    }

    // Current step prompt + input
    let input_style = Style::default().fg(Color::Cyan);
    let display = if state.custom_input.is_empty() {
        prompts[step]
    } else {
        &state.custom_input
    };
    f.render_widget(
        Paragraph::new(display)
            .style(if state.custom_input.is_empty() {
                input_style.fg(Color::DarkGray)
            } else {
                input_style
            })
            .block(Block::default().borders(Borders::ALL).title(steps[step])),
        chunks[1],
    );

    // Cursor
    let prefix_width = crate::tui::chat_lines::display_width_up_to(&state.custom_input, state.custom_cursor);
    let cursor_x = chunks[1].x + prefix_width as u16 + 1;
    let cursor_y = chunks[1].y + 1;
    f.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));

    // Hints
    let hint = if step <= 1 {
        "Enter to continue · Esc to cancel"
    } else {
        "Enter to confirm · Esc to cancel"
    };
    f.render_widget(
        Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
        chunks[2],
    );
}

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
                Span::styled(*cmd, Style::default()),
                Span::styled(format!("  — {}", desc), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let popup_h = (matches.len() as u16).clamp(3, 6) + 2;
    let popup_w = 50.min(chat_area.width.saturating_sub(4));
    let x = chat_area.x;
    let y = chat_area.y + chat_area.height.saturating_sub(popup_h + 3);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Commands ")
        .style(Style::default().fg(Color::Cyan));

    let mut list_state = ListState::default();
    list_state.select(Some(state.command_popup_selection.min(matches.len().saturating_sub(1))));
    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .highlight_symbol("❯ "),
        Rect::new(x, y, popup_w, popup_h),
        &mut list_state,
    );
}
