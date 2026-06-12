//! Role template wizard — 3-step form for creating/editing role templates.

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
use crate::runtime::config::RoleTemplate;
use crate::tui::style;

/// Multi-step role template wizard.
#[derive(Clone, Debug)]
pub struct RoleWizard {
    pub step: usize,
    pub role_name: String,
    pub label: String,
    pub system_prompt: String,
    pub editing: bool,
    pub editing_id: u32,
    pub input: String,
    pub cursor: usize,
}

impl RoleWizard {
    /// Create a new role wizard (create mode).
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a role wizard pre-filled with an existing template (edit mode).
    pub fn from_template(template: RoleTemplate) -> Self {
        Self {
            step: 0,
            role_name: template.role,
            label: template.label,
            system_prompt: template.system_prompt,
            editing: true,
            editing_id: template.template_id,
            input: String::new(),
            cursor: 0,
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
                let input = self.input.trim().to_string();
                match self.step {
                    0 => {
                        // Role name
                        if !input.is_empty() {
                            self.role_name = input;
                            self.step = 1;
                            self.input.clear();
                            self.cursor = 0;
                        } else if self.editing && !self.role_name.is_empty() {
                            // Skip if editing and prefilled
                            self.step = 1;
                            self.input.clear();
                            self.cursor = 0;
                        }
                    }
                    1 => {
                        // Label
                        if !input.is_empty() {
                            self.label = input;
                        }
                        // If editing and label is already set (even if empty), allow proceeding
                        self.step = 2;
                        self.input.clear();
                        self.cursor = 0;
                    }
                    2 => {
                        // System prompt — save
                        let prompt_text = if self.input.is_empty() {
                            // If editing and prompt already set, keep it
                            if self.editing && !self.system_prompt.is_empty() {
                                self.system_prompt.clone()
                            } else {
                                // Default prompt
                                format!("You are a {}. Execute the given goal.", self.role_name)
                            }
                        } else {
                            std::mem::take(&mut self.input)
                        };
                        self.system_prompt = prompt_text;

                        // Save via runtime
                        let tpl = RoleTemplate {
                            role: self.role_name.clone(),
                            label: if self.label.is_empty() {
                                self.role_name.clone()
                            } else {
                                self.label.clone()
                            },
                            system_prompt: self.system_prompt.clone(),
                            template_id: self.editing_id,
                            embedding: None,
                        };

                        if let Some(runtime) = &state.runtime {
                            if let Ok(rt) = runtime.try_read() {
                                rt.save_role_template(tpl);
                                let action = if self.editing { "updated" } else { "created" };
                                state.messages.push(
                                    crate::tui::state::ChatMessage::system(
                                        format!("Role '{}' {}.", self.role_name, action),
                                    ),
                                );
                            }
                        }

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
        let steps = ["Role Name", "Label", "System Prompt"];
        let prompts = [
            "Enter the role name (e.g. security_auditor):",
            "Enter a human-readable label (e.g. Security Auditor):",
            "Enter the system prompt (multi-line, Alt+Enter for newline):",
        ];
        let step = self.step.min(steps.len() - 1);
        let total_steps = steps.len();

        let dialog_w = 72u16.min(area.width.saturating_sub(4));
        let dialog_h = if step == 2 { 16u16 } else { 11u16 };
        let x = area.x + (area.width.saturating_sub(dialog_w)) / 2;
        let y = area.y + (area.height.saturating_sub(dialog_h)) / 2;
        let dialog_area = Rect::new(x, y, dialog_w, dialog_h);

        let title = if self.editing {
            format!("Edit Role — Step {}/{}", step + 1, total_steps)
        } else {
            format!("Create Role — Step {}/{}", step + 1, total_steps)
        };
        let block = style::panel(&title);
        let inner = block.inner(dialog_area);
        f.render_widget(block, dialog_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                if step == 2 {
                    vec![Constraint::Length(3), Constraint::Length(10), Constraint::Length(1)]
                } else {
                    vec![Constraint::Length(3), Constraint::Length(3), Constraint::Length(1)]
                },
            )
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
        if !self.role_name.is_empty() {
            summary_parts.push(format!("Name: {}", self.role_name));
        }
        if !self.label.is_empty() {
            summary_parts.push(format!("Label: {}", self.label));
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

        if step == 2 {
            // Multi-line prompt area
            let prompt_text = if has_input {
                self.input.clone()
            } else if !self.system_prompt.is_empty() {
                self.system_prompt.clone()
            } else {
                String::new()
            };
            let display = if prompt_text.is_empty() {
                Paragraph::new(Span::styled(summary_text, style::hint_style())).block(style::input_box(has_previous))
            } else {
                Paragraph::new(prompt_text.as_str())
                    .style(style::value_style())
                    .block(style::input_box(true))
            };
            f.render_widget(display, chunks[1]);

            let prefix_width = display_width_up_to(&self.input, self.cursor);
            let cursor_x = chunks[1].x + prefix_width as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));
        } else {
            let display = if !has_input {
                Paragraph::new(Span::styled(summary_text, style::hint_style())).block(style::input_box(has_previous))
            } else {
                Paragraph::new(self.input.as_str())
                    .style(style::value_style())
                    .block(style::input_box(true))
            };
            f.render_widget(display, chunks[1]);

            let prefix_width = display_width_up_to(&self.input, self.cursor);
            let cursor_x = chunks[1].x + prefix_width as u16 + 1;
            let cursor_y = chunks[1].y + 1;
            f.set_cursor_position((cursor_x.min(inner.right().saturating_sub(1)), cursor_y));
        }

        // ── Hint ──
        let hint = if step == 2 {
            "Type or edit the system prompt  ·  Enter to save  ·  Esc to cancel"
        } else {
            "Enter to continue  ·  Esc to cancel"
        };
        style::render_hint(f, chunks[2], hint);
    }
}

impl Default for RoleWizard {
    fn default() -> Self {
        Self {
            step: 0,
            role_name: String::new(),
            label: String::new(),
            system_prompt: String::new(),
            editing: false,
            editing_id: 0,
            input: String::new(),
            cursor: 0,
        }
    }
}
