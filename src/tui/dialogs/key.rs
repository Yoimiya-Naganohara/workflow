//! API key entry dialog.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    widgets::Paragraph,
};

use crate::tui::chat_lines::char_idx_to_byte_idx;
use crate::tui::controller;
use crate::tui::state::{ChatMessage, CoreState};

use super::DialogTransition;
use crate::tui::style;

/// State for the API key input dialog.
#[derive(Clone, Debug)]
pub struct KeyDialog {
    pub input: String,
    pub cursor: usize,
    pub provider_id: String,
    /// When true, closing this dialog re-opens the model picker
    /// (used when the model picker triggered the key prompt).
    pub return_to_picker: bool,
}

impl KeyDialog {
    /// Create a new key dialog for a specific provider.
    pub fn for_provider(provider_id: String) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            provider_id,
            return_to_picker: false,
        }
    }

    /// Create a key dialog with return-to-picker flag.
    pub fn for_provider_with_picker(provider_id: String) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            provider_id,
            return_to_picker: true,
        }
    }

    pub fn handle_key(&mut self, state: &mut CoreState, key: KeyEvent) -> DialogTransition {
        match key.code {
            KeyCode::Esc => DialogTransition::Close,
            KeyCode::Char(c) => {
                let byte_idx = char_idx_to_byte_idx(&self.input, self.cursor);
                self.input.insert(byte_idx, c);
                self.cursor += 1;
                DialogTransition::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    let byte_idx = char_idx_to_byte_idx(&self.input, self.cursor);
                    self.input.remove(byte_idx);
                }
                DialogTransition::None
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                DialogTransition::None
            }
            KeyCode::Right => {
                if self.cursor < self.input.chars().count() {
                    self.cursor += 1;
                }
                DialogTransition::None
            }
            KeyCode::Enter => {
                self.commit(state);
                if self.return_to_picker {
                    DialogTransition::Switch(super::ActiveDialog::ModelPicker(
                        crate::tui::dialogs::model_picker::ModelPicker::new(),
                    ))
                } else {
                    DialogTransition::Close
                }
            }
            _ => DialogTransition::None,
        }
    }

    fn commit(&self, state: &mut CoreState) {
        let provider_id = &self.provider_id;
        let provider = match state.models.providers().iter().find(|p| p.id == *provider_id) {
            Some(p) => p,
            None => return,
        };

        let env_key = provider.env.first().cloned().unwrap_or_default();
        let provider_name = provider.name.clone();

        if !env_key.is_empty() && !self.input.is_empty() {
            state.api_keys.insert(env_key.clone(), self.input.clone());
            state.models.select_provider(provider_id);
            if !state.configured_providers.contains(provider_id) {
                state.configured_providers.push(provider_id.clone());
            }
            state.provider_clients.remove(provider_id);
            let _ = controller::get_or_create_provider_client(state, provider_id);
            state.messages.push(ChatMessage::system(format!(
                "{} key set for {}",
                env_key, provider_name
            )));
            if let Err(e) = controller::save_api_key(provider_id, &env_key, &self.input) {
                state
                    .messages
                    .push(ChatMessage::system(format!("Failed to save config: {}", e)));
            }
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, state: &CoreState) {
        let provider_name = state
            .models
            .providers()
            .iter()
            .find(|p| p.id == self.provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or("Unknown");

        let dialog_w = 54u16.min(area.width.saturating_sub(4));
        let dialog_h = 10u16;
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
        let masked: String = self.input.chars().map(|_| '•').collect();
        let has_input = !self.input.is_empty();
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
        let cursor_x = chunks[0].x + self.input.len() as u16 + 1;
        let cursor_y = chunks[0].y + 1;
        f.set_cursor_position((cursor_x.min(chunks[0].right().saturating_sub(2)), cursor_y));

        // Hint
        style::render_hint(
            f,
            chunks[1],
            if self.return_to_picker {
                "Enter to confirm (returns to model picker)  ·  Esc to cancel"
            } else {
                "Enter to confirm  ·  Esc to cancel"
            },
        );
    }
}
