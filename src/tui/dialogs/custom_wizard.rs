//! Custom provider wizard — 4-step form for adding a custom OpenAI-compatible provider.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::chat_lines::{char_idx_to_byte_idx, display_width_up_to};
use crate::tui::state::CoreState;

use super::DialogTransition;
use crate::tui::style;

/// Multi-step custom provider configuration wizard.
#[derive(Clone, Debug, Default)]
pub struct CustomWizard {
    pub step: usize,
    pub name: String,
    pub url: String,
    pub api_key: String,
    pub models: String,
    pub input: String,
    pub cursor: usize,
}

impl CustomWizard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle_key(&mut self, state: &mut CoreState, key: KeyEvent) -> DialogTransition {
        let step = self.step;
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
                let input = self.input.trim().to_string();
                match step {
                    0 => {
                        // Provider name
                        if !input.is_empty() {
                            self.name = input;
                            self.step = 1;
                            self.input.clear();
                            self.cursor = 0;
                        }
                    }
                    1 => {
                        // API URL
                        if !input.is_empty() {
                            self.url = input;
                            self.step = 2;
                            self.input.clear();
                            self.cursor = 0;
                        }
                    }
                    2 => {
                        // API key (can be empty for no-auth)
                        self.api_key = input;
                        self.step = 3;
                        self.input.clear();
                        self.cursor = 0;
                    }
                    3 => {
                        // Model IDs — save
                        let name = self.name.clone();
                        let url = self.url.clone();
                        let key = self.api_key.clone();
                        let models_str = input;
                        self.models = models_str.clone();
                        // Save via controller
                        crate::tui::controller::save_custom_provider(state, &name, &url, &key, &models_str);
                        return DialogTransition::Close;
                    }
                    _ => {}
                }
                DialogTransition::None
            }
            _ => DialogTransition::None,
        }
    }

    pub fn render(&self, f: &mut Frame, area: Rect, _state: &CoreState) {
        let steps = ["Provider Name", "API Base URL", "API Key", "Model IDs"];
        let prompts = [
            "Enter a name for your custom provider:",
            "Enter the API base URL (e.g. https://api.example.com/v1):",
            "Enter the API key (leave empty for no auth):",
            "Enter model ID(s) (comma-separated, e.g. gpt-4,claude-3):",
        ];
        let step = self.step.min(steps.len() - 1);
        let total_steps = steps.len();

        let dialog_w = 66u16.min(area.width.saturating_sub(4));
        let dialog_h = 11u16;
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
        if !self.name.is_empty() {
            summary_parts.push(format!("Name: {}", self.name));
        }
        if !self.url.is_empty() {
            let url_display = if self.url.len() > 40 {
                // Safely truncate at char boundary — avoid splitting multi-byte UTF-8.
                let end = self
                    .url
                    .char_indices()
                    .nth(37)
                    .map(|(i, _)| i)
                    .unwrap_or(self.url.len());
                format!("{}…", &self.url[..end])
            } else {
                self.url.clone()
            };
            summary_parts.push(format!("URL: {}", url_display));
        }
        if !self.api_key.is_empty() {
            summary_parts.push("Key: ••••••••".to_string());
        }

        let summary_text: &str = if summary_parts.is_empty() {
            prompts[step]
        } else {
            let joined = summary_parts.join("  │  ");
            Box::leak(joined.into_boxed_str())
        };

        // ── Input ──
        let has_previous = !summary_parts.is_empty();
        let has_input = !self.input.is_empty();

        let input_display = if !has_input {
            Paragraph::new(Span::styled(summary_text, style::hint_style())).block(style::input_box(has_previous))
        } else {
            Paragraph::new(self.input.as_str())
                .style(style::value_style())
                .block(style::input_box(true))
        };
        f.render_widget(input_display, chunks[1]);

        // Cursor
        let prefix_width = display_width_up_to(&self.input, self.cursor);
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
}
