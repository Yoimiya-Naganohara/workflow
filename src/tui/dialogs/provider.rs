//! Provider selection dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::Span,
    widgets::{Paragraph, Row, Table, TableState},
};

use crate::models::filter_providers;
use crate::tui::chat_lines::char_idx_to_byte_idx;

use crate::tui::style;

use super::DialogTransition;
use crate::tui::state::{ChatMessage, CoreState, MessageRole, MessageStatus};

#[derive(Clone, Debug, Default)]
pub struct ProviderDialog {
    pub search_query: String,
    pub search_cursor: usize,
    pub selected_idx: usize,
}

impl ProviderDialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.search_query.clear();
        self.search_cursor = 0;
        self.selected_idx = 0;
    }

    pub fn handle_key(&mut self, state: &mut CoreState, key: KeyEvent) -> DialogTransition {
        match key.code {
            KeyCode::Esc => DialogTransition::Close,
            KeyCode::Down => {
                let total = self.total_items(state);
                if total > 0 {
                    self.selected_idx = (self.selected_idx + 1).min(total - 1);
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
                if self.search_cursor < char_count(&self.search_query) {
                    self.search_cursor += 1;
                }
                DialogTransition::None
            }
            KeyCode::Enter => self.select(state),
            _ => DialogTransition::None,
        }
    }

    fn filtered<'a>(&self, state: &'a CoreState) -> Vec<(usize, &'a crate::models::Provider)> {
        filter_providers(state.models.providers(), &self.search_query)
            .into_iter()
            .enumerate()
            .collect()
    }

    pub(crate) fn show_custom(&self) -> bool {
        self.search_query.is_empty()
            || self.search_query.to_lowercase().contains("custom")
            || self.search_query.to_lowercase().contains("add")
    }

    fn total_items(&self, state: &CoreState) -> usize {
        let count = filter_providers(state.models.providers(), &self.search_query).len();
        count + if self.show_custom() { 1 } else { 0 }
    }

    fn select(&mut self, state: &mut CoreState) -> DialogTransition {
        let filtered = self.filtered(state);
        if filtered.is_empty() && !self.show_custom() {
            return DialogTransition::None;
        }

        let is_custom = self.show_custom() && self.selected_idx == filtered.len();
        if is_custom {
            return DialogTransition::Switch(super::ActiveDialog::CustomWizard(
                crate::tui::dialogs::custom_wizard::CustomWizard::new(),
            ));
        }

        if let Some((_, provider)) = filtered.get(self.selected_idx) {
            let provider_id = provider.id.clone();
            if crate::tui::controller::is_no_auth_provider(&provider_id) {
                self.commit_no_auth(state, &provider_id);
                return DialogTransition::Close;
            } else {
                return DialogTransition::Switch(super::ActiveDialog::Key(
                    crate::tui::dialogs::key::KeyDialog::for_provider(provider_id),
                ));
            }
        }
        DialogTransition::None
    }

    fn commit_no_auth(&self, state: &mut CoreState, provider_id: &str) {
        if state.configured_providers.contains(&provider_id.to_string()) {
            return;
        }
        state.configured_providers.push(provider_id.to_string());
        state.models.select_provider(provider_id);
        let _ = crate::tui::controller::get_or_create_provider_client(state, provider_id);
        let provider_name = state
            .models
            .providers()
            .iter()
            .find(|p| p.id == provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or(provider_id);
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("{} configured (no API key required)", provider_name),
            timestamp: now,
            status: MessageStatus::Completed,
        });
    }

    pub fn scroll_down(&mut self, state: &CoreState) {
        let total = self.total_items(state);
        if total > 0 {
            self.selected_idx = (self.selected_idx + 1).min(total - 1);
        }
    }

    pub fn scroll_up(&mut self, _state: &CoreState) {
        self.selected_idx = self.selected_idx.saturating_sub(1);
    }

    pub fn render(&self, f: &mut Frame, area: Rect, state: &CoreState) {
        let filtered = filter_providers(state.models.providers(), &self.search_query);
        let show_custom = self.show_custom();
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

        let dialog_w = 64u16.min(area.width.saturating_sub(4));
        let search_h = 3u16;
        let list_h = (total_items as u16).min(12);
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
            Paragraph::new(self.search_query.as_str())
                .style(style::value_style())
                .block(style::input_box(true)),
            chunks[0],
        );
        let prefix_width = crate::tui::chat_lines::display_width_up_to(&self.search_query, self.search_cursor);
        let cursor_x = chunks[0].x + prefix_width as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x, cursor_y));

        style::render_separator(f, chunks[1]);

        let mut rows: Vec<Row> = filtered
            .iter()
            .map(|p| {
                let count = p.models.len();
                let is_configured = state.configured_providers.iter().any(|id| id == &p.id);
                Row::new(vec![
                    if is_configured {
                        Span::styled("✓", Style::default().fg(style::SUCCESS))
                    } else {
                        Span::raw("")
                    },
                    Span::styled(&p.name, style::value_style()),
                    Span::styled(format!("{} models", count), style::hint_style()),
                ])
            })
            .collect();

        if show_custom {
            rows.push(Row::new(vec![
                Span::raw(""),
                Span::styled("Add Custom Provider", Style::default().fg(style::ACTIVE)),
                Span::raw(""),
            ]));
        }

        // Compute column widths from content.
        let max_name = filtered.iter().map(|p| p.name.len()).max().unwrap_or(0);
        let max_count = filtered
            .iter()
            .map(|p| format!("{} models", p.models.len()).len())
            .max()
            .unwrap_or(8);

        let mut table_state = TableState::default();
        table_state.select(Some(self.selected_idx.min(total_items.saturating_sub(1))));
        f.render_stateful_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(1),                   // checkmark
                    Constraint::Length(max_name as u16 + 1), // provider name
                    Constraint::Length(max_count as u16),    // model count
                ],
            )
            .row_highlight_style(Style::default().fg(style::HIGHLIGHT_FG).bg(style::HIGHLIGHT_BG)),
            chunks[2],
            &mut table_state,
        );
    }
}

fn char_count(s: &str) -> usize {
    s.chars().count()
}
