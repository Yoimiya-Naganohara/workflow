//! Model selection dialog — pick models from configured providers.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Row, Table, TableState},
};

use crate::tui::chat_lines::char_idx_to_byte_idx;
use crate::tui::controller;
use crate::tui::state::{ChatMessage, CoreState, SelectedModel};

use super::DialogTransition;
use crate::tui::style;

/// State for the model picker dialog.
#[derive(Clone, Debug, Default)]
pub struct ModelPicker {
    pub search_query: String,
    pub search_cursor: usize,
    pub selected_idx: usize,
}

impl ModelPicker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_key(&mut self, state: &mut CoreState, key: KeyEvent) -> DialogTransition {
        match key.code {
            KeyCode::Esc => DialogTransition::Close,
            KeyCode::Down => {
                let count = self.result_count(state);
                if count > 0 {
                    self.selected_idx = (self.selected_idx + 1).min(count - 1);
                }
                DialogTransition::None
            }
            KeyCode::Up => {
                self.selected_idx = self.selected_idx.saturating_sub(1);
                DialogTransition::None
            }
            KeyCode::Char(c) => {
                let byte_idx = char_idx_to_byte_idx(&self.search_query, self.search_cursor);
                self.search_query.insert(byte_idx, c);
                self.search_cursor += 1;
                self.selected_idx = 0;
                DialogTransition::None
            }
            KeyCode::Backspace => {
                if self.search_cursor > 0 {
                    self.search_cursor -= 1;
                    let byte_idx = char_idx_to_byte_idx(&self.search_query, self.search_cursor);
                    self.search_query.remove(byte_idx);
                    self.selected_idx = 0;
                }
                DialogTransition::None
            }
            KeyCode::Left => {
                self.search_cursor = self.search_cursor.saturating_sub(1);
                DialogTransition::None
            }
            KeyCode::Right => {
                if self.search_cursor < self.search_query.chars().count() {
                    self.search_cursor += 1;
                }
                DialogTransition::None
            }
            KeyCode::Enter => {
                // Inline toggle — avoid borrow conflict with state
                let entry = {
                    let results = state
                        .models
                        .search_configured_models(&self.search_query, &state.configured_providers);
                    results.get(self.selected_idx).map(|(p, m)| {
                        let provider_id = p.id.clone();
                        let model_id = m.id.clone();
                        let provider_name = p.name.clone();
                        let model_name = m.name.clone();
                        (provider_id, model_id, provider_name, model_name)
                    })
                };
                if let Some((provider_id, model_id, provider_name, model_name)) = entry {
                    if let Some(pos) = state
                        .selected_models
                        .iter()
                        .position(|sm| sm.provider_id == provider_id && sm.model_id == model_id)
                    {
                        state.selected_models.remove(pos);
                        state.messages.push(ChatMessage::system(format!(
                            "Removed: {} / {}",
                            provider_name, model_name
                        )));
                    } else {
                        state.selected_models.push(SelectedModel {
                            provider_id,
                            model_id,
                            provider_name,
                            model_name,
                        });
                    }
                    if let Err(e) = controller::save_selected_models(&state.selected_models) {
                        state
                            .messages
                            .push(ChatMessage::system(format!("Failed to save: {}", e)));
                    }
                }
                DialogTransition::None
            }
            _ => DialogTransition::None,
        }
    }

    fn result_count(&self, state: &CoreState) -> usize {
        state
            .models
            .search_configured_models(&self.search_query, &state.configured_providers)
            .len()
    }

    pub fn scroll_down(&mut self, state: &CoreState) {
        let results = state
            .models
            .search_configured_models(&self.search_query, &state.configured_providers);
        if !results.is_empty() {
            self.selected_idx = (self.selected_idx + 1).min(results.len() - 1);
        }
    }

    pub fn scroll_up(&mut self, _state: &CoreState) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    pub fn render(&self, f: &mut Frame, area: Rect, state: &CoreState) {
        let results = state
            .models
            .search_configured_models(&self.search_query, &state.configured_providers);

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

        let dialog_w = 64u16.min(area.width.saturating_sub(4));
        let list_h = (results.len() as u16).min(12);
        let dialog_h = (list_h + 5 + 1).min(area.height.saturating_sub(4));
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let block = style::panel("Model Pool");
        let inner = block.inner(dialog_area);
        f.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        // Search line — reads from dialog.search_query (synced from main input)
        let search_text = if self.search_query.is_empty() {
            "⌕ search models...".to_string()
        } else {
            format!("search: {}", self.search_query)
        };
        f.render_widget(
            Paragraph::new(search_text).style(if self.search_query.is_empty() { style::hint_style() } else { style::value_style() }),
            chunks[0],
        );

        style::render_separator(f, chunks[1]);

        // Build table rows
        let rows: Vec<Row> = results
            .iter()
            .map(|(p, m)| {
                let badge = m.capability_badge();
                let is_selected = state
                    .selected_models
                    .iter()
                    .any(|sm| sm.provider_id == p.id && sm.model_id == m.id);
                Row::new(vec![
                    if is_selected {
                        Span::styled("✓", Style::default().fg(style::SUCCESS))
                    } else {
                        Span::raw("")
                    },
                    Span::styled(&m.name, style::value_style()),
                    Span::styled(badge, style::hint_style().italic()),
                    Span::styled(&p.name, style::hint_style()),
                ])
            })
            .collect();

        // Compute column widths from content.
        let max_name = results.iter().map(|(_, m)| m.name.len()).max().unwrap_or(0);
        let max_badge = results
            .iter()
            .map(|(_, m)| m.capability_badge().len())
            .max()
            .unwrap_or(0);
        let max_provider = results.iter().map(|(p, _)| p.name.len()).max().unwrap_or(0);

        let mut table_state = TableState::default();
        table_state.select(Some(self.selected_idx.min(results.len().saturating_sub(1))));
        f.render_stateful_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(1),                    // checkmark
                    Constraint::Length(max_name as u16 + 1),  // model name
                    Constraint::Length(max_badge as u16 + 1), // badge
                    Constraint::Length(max_provider as u16),  // provider
                ],
            )
            .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
            chunks[2],
            &mut table_state,
        );
    }
}
