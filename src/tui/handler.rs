//! Keyboard and mouse event handler.
//!
//! This file only handles input events and mutates UI-level state
//! (cursor position, dialog visibility, scroll, selection).
//! All business logic (provider config, chat, persistence, shell)
//! is delegated to [`crate::controller`].

use crossterm::event::KeyCode;
use futures::future::AbortHandle;

use super::Tui;
use super::state::{AppMode, AppState, COMMANDS, ChatMessage, Focus, MessageRole, MessageStatus, Panel, SelectedModel};
use crate::controller;
use crate::models::filter_providers;

impl Tui {
    pub(crate) fn handle_chat_keys(&self, state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
        let code = key.code;

        // ── Key dialog ──
        if state.show_key_dialog {
            return self.handle_key_dialog(state, code, key);
        }

        // ── Model picker ──
        if state.show_model_picker {
            return self.handle_model_picker(state, code);
        }

        // ── Provider dialog ──
        if state.show_provider_dialog {
            return self.handle_provider_dialog(state, code);
        }

        match code {
            KeyCode::Esc => {
                state.focus = Focus::Input;
                state.input.clear();
                state.input_cursor = 0;
                state.command_popup_selection = 0;
            }
            KeyCode::Tab => {
                if state.focus == Focus::Input && state.input.starts_with('/') {
                    let prefix = state.input.trim().to_lowercase();
                    let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
                    if !matches.is_empty() {
                        state.command_popup_selection = (state.command_popup_selection + 1) % matches.len();
                    }
                } else {
                    state.mode = match state.mode {
                        AppMode::Plan => AppMode::Build,
                        AppMode::Build => AppMode::Plan,
                    };
                }
            }
            KeyCode::Char('1') => state.panel = Panel::Chat,
            KeyCode::Char('x') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                if let Some(abort) = state.active_chat_abort.take() {
                    abort.abort();
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Stopped current response".to_string(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
                }
            }
            KeyCode::Char('p') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                state.show_model_picker = true;
                state.selected_model_picker_idx = 0;
                state.model_picker_search_query.clear();
                state.model_picker_search_cursor = 0;
            }
            KeyCode::Down if state.focus == Focus::Chat => {
                state.chat_scroll = state.chat_scroll.saturating_add(1);
            }
            KeyCode::Up if state.focus == Focus::Chat => {
                state.chat_scroll = state.chat_scroll.saturating_sub(1);
            }
            KeyCode::Char('g') if state.focus == Focus::Chat => {
                state.chat_scroll = usize::MAX / 2;
            }
            KeyCode::Char('G') if state.focus == Focus::Chat => {
                state.chat_scroll = 0;
            }
            KeyCode::Enter if state.focus == Focus::Input => {
                return self.handle_input_submit(state);
            }
            KeyCode::Up
                if state.focus == Focus::Input && !state.input.starts_with('/') && !state.input_history.is_empty() =>
            {
                let idx = state.input_history_idx.unwrap_or(state.input_history.len());
                let new_idx = idx.saturating_sub(1);
                state.input_history_idx = Some(new_idx);
                state.input = state.input_history[new_idx].clone();
                state.input_cursor = Self::char_count(&state.input);
            }
            KeyCode::Down
                if state.focus == Focus::Input
                    && !state.input.starts_with('/')
                    && state.input_history_idx.is_some() =>
            {
                let idx = state.input_history_idx.unwrap();
                if idx + 1 < state.input_history.len() {
                    state.input_history_idx = Some(idx + 1);
                    state.input = state.input_history[idx + 1].clone();
                } else {
                    state.input_history_idx = None;
                    state.input.clear();
                }
                state.input_cursor = Self::char_count(&state.input);
            }
            KeyCode::Up if state.focus == Focus::Input && state.input.starts_with('/') => {
                let prefix = state.input.trim().to_lowercase();
                let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
                if !matches.is_empty() {
                    state.command_popup_selection =
                        state.command_popup_selection.saturating_sub(1).max(matches.len() - 1);
                }
            }
            KeyCode::Down if state.focus == Focus::Input && state.input.starts_with('/') => {
                let prefix = state.input.trim().to_lowercase();
                let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
                if !matches.is_empty() {
                    state.command_popup_selection = (state.command_popup_selection + 1) % matches.len();
                }
            }
            KeyCode::Char(c)
                if state.focus == Focus::Input
                    && (key.modifiers.is_empty() || key.modifiers == crossterm::event::KeyModifiers::SHIFT) =>
            {
                let byte_idx = Self::char_idx_to_byte_idx(&state.input, state.input_cursor);
                state.input.insert(byte_idx, c);
                state.input_cursor += 1;
                state.input_history_idx = None;
                if state.input.starts_with('/') {
                    state.command_popup_selection = 0;
                }
            }
            KeyCode::Backspace if state.focus == Focus::Input => {
                if state.input_cursor > 0 {
                    state.input_cursor -= 1;
                    let byte_idx = Self::char_idx_to_byte_idx(&state.input, state.input_cursor);
                    state.input.remove(byte_idx);
                    state.input_history_idx = None;
                    if state.input.starts_with('/') {
                        state.command_popup_selection = 0;
                    }
                }
            }
            KeyCode::Left if state.focus == Focus::Input => {
                state.input_cursor = state.input_cursor.saturating_sub(1);
            }
            KeyCode::Right if state.focus == Focus::Input => {
                if state.input_cursor < Self::char_count(&state.input) {
                    state.input_cursor += 1;
                }
            }
            _ => {}
        }
        true
    }

    // ── Dialog sub-handlers ──

    fn handle_key_dialog(&self, state: &mut AppState, code: KeyCode, _key: crossterm::event::KeyEvent) -> bool {
        match code {
            KeyCode::Esc => {
                state.show_key_dialog = false;
                state.key_input.clear();
                state.key_cursor = 0;
            }
            KeyCode::Char(c) => {
                let byte_idx = Self::char_idx_to_byte_idx(&state.key_input, state.key_cursor);
                state.key_input.insert(byte_idx, c);
                state.key_cursor += 1;
            }
            KeyCode::Backspace => {
                if state.key_cursor > 0 {
                    state.key_cursor -= 1;
                    let byte_idx = Self::char_idx_to_byte_idx(&state.key_input, state.key_cursor);
                    state.key_input.remove(byte_idx);
                }
            }
            KeyCode::Left => {
                state.key_cursor = state.key_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if state.key_cursor < Self::char_count(&state.key_input) {
                    state.key_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(provider_id) = state.key_provider_id.clone()
                    && let Some(provider) = state.models.providers().iter().find(|p| p.id == provider_id)
                {
                    let env_key = provider.env.first().cloned().unwrap_or_default();
                    let provider_name = provider.name.clone();
                    if !env_key.is_empty() && !state.key_input.is_empty() {
                        state.api_keys.insert(env_key.clone(), state.key_input.clone());
                        state.models.select_provider(&provider_id);
                        if !state.configured_providers.contains(&provider_id) {
                            state.configured_providers.push(provider_id.clone());
                        }
                        state.provider_clients.remove(&provider_id);
                        let _ = controller::get_or_create_provider_client(state, &provider_id);
                        let now = chrono::Local::now().format("%H:%M:%S").to_string();
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("{} key set for {}", env_key, provider_name),
                            timestamp: now,
                            status: MessageStatus::Completed,
                        });
                        if let Err(e) = controller::save_api_key(&provider_id, &env_key, &state.key_input) {
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!("Failed to save config: {}", e),
                                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                status: MessageStatus::Completed,
                            });
                        }
                    }
                }
                let return_to_picker = state.return_to_model_picker;
                state.show_key_dialog = false;
                state.key_input.clear();
                state.key_cursor = 0;
                state.key_provider_id = None;
                state.show_provider_dialog = false;
                state.provider_search_query.clear();
                state.provider_search_cursor = 0;
                state.selected_provider_idx = 0;
                if return_to_picker {
                    state.show_model_picker = true;
                    state.selected_model_picker_idx = 0;
                    state.model_picker_search_query.clear();
                    state.model_picker_search_cursor = 0;
                    state.return_to_model_picker = false;
                }
            }
            _ => {}
        }
        true
    }

    fn handle_model_picker(&self, state: &mut AppState, code: KeyCode) -> bool {
        let results = state.models.search_models(&state.model_picker_search_query);
        match code {
            KeyCode::Esc => {
                state.show_model_picker = false;
                state.model_picker_search_query.clear();
                state.model_picker_search_cursor = 0;
                state.selected_model_picker_idx = 0;
            }
            KeyCode::Char('a') if !state.models.providers().is_empty() => {
                state.show_model_picker = false;
                state.show_provider_dialog = true;
                state.selected_provider_idx = 0;
                state.provider_search_query.clear();
                state.provider_search_cursor = 0;
                state.return_to_model_picker = true;
            }
            KeyCode::Down if !results.is_empty() => {
                state.selected_model_picker_idx = (state.selected_model_picker_idx + 1).min(results.len() - 1);
            }
            KeyCode::Up => {
                state.selected_model_picker_idx = state.selected_model_picker_idx.saturating_sub(1);
            }
            KeyCode::Char(c) => {
                let byte_idx =
                    Self::char_idx_to_byte_idx(&state.model_picker_search_query, state.model_picker_search_cursor);
                state.model_picker_search_query.insert(byte_idx, c);
                state.model_picker_search_cursor += 1;
                state.selected_model_picker_idx = 0;
            }
            KeyCode::Backspace => {
                if state.model_picker_search_cursor > 0 {
                    state.model_picker_search_cursor -= 1;
                    let byte_idx =
                        Self::char_idx_to_byte_idx(&state.model_picker_search_query, state.model_picker_search_cursor);
                    state.model_picker_search_query.remove(byte_idx);
                    state.selected_model_picker_idx = 0;
                }
            }
            KeyCode::Left => {
                state.model_picker_search_cursor = state.model_picker_search_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if state.model_picker_search_cursor < Self::char_count(&state.model_picker_search_query) {
                    state.model_picker_search_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some((provider, model)) = results.get(state.selected_model_picker_idx) {
                    let provider_id = provider.id.clone();
                    let model_id = model.id.clone();
                    let provider_name = provider.name.clone();
                    let model_name = model.name.clone();

                    if !state.configured_providers.contains(&provider_id)
                        && !controller::is_no_auth_provider(&provider_id)
                    {
                        state.show_model_picker = false;
                        state.show_key_dialog = true;
                        state.key_provider_id = Some(provider_id);
                        state.key_input.clear();
                        state.key_cursor = 0;
                        state.return_to_model_picker = true;
                        return true;
                    }

                    if let Some(pos) = state
                        .selected_models
                        .iter()
                        .position(|sm| sm.provider_id == provider_id && sm.model_id == model_id)
                    {
                        state.selected_models.remove(pos);
                        let now = chrono::Local::now().format("%H:%M:%S").to_string();
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("Removed: {} / {}", provider_name, model_name),
                            timestamp: now,
                            status: MessageStatus::Completed,
                        });
                    } else {
                        state.selected_models.push(SelectedModel {
                            provider_id,
                            model_id,
                            provider_name: provider_name.clone(),
                            model_name: model_name.clone(),
                        });
                        state.input.clear();
                        state.input_cursor = 0;
                        state.chat_scroll = 0;
                    }
                    if let Err(e) = controller::save_selected_models(&state.selected_models) {
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: format!("Failed to save: {}", e),
                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                            status: MessageStatus::Completed,
                        });
                    }
                }
            }
            _ => {}
        }
        true
    }

    fn handle_provider_dialog(&self, state: &mut AppState, code: KeyCode) -> bool {
        let providers = filter_providers(state.models.providers(), &state.provider_search_query);
        match code {
            KeyCode::Esc => {
                state.show_provider_dialog = false;
                state.provider_search_query.clear();
                state.provider_search_cursor = 0;
                state.selected_provider_idx = 0;
            }
            KeyCode::Down if !providers.is_empty() => {
                state.selected_provider_idx = (state.selected_provider_idx + 1).min(providers.len() - 1);
            }
            KeyCode::Up => {
                state.selected_provider_idx = state.selected_provider_idx.saturating_sub(1);
            }
            KeyCode::Char(c) => {
                let byte_idx = Self::char_idx_to_byte_idx(&state.provider_search_query, state.provider_search_cursor);
                state.provider_search_query.insert(byte_idx, c);
                state.provider_search_cursor += 1;
                state.selected_provider_idx = 0;
            }
            KeyCode::Backspace => {
                if state.provider_search_cursor > 0 {
                    state.provider_search_cursor -= 1;
                    let byte_idx =
                        Self::char_idx_to_byte_idx(&state.provider_search_query, state.provider_search_cursor);
                    state.provider_search_query.remove(byte_idx);
                    state.selected_provider_idx = 0;
                }
            }
            KeyCode::Left => {
                state.provider_search_cursor = state.provider_search_cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if state.provider_search_cursor < Self::char_count(&state.provider_search_query) {
                    state.provider_search_cursor += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(provider) = providers.get(state.selected_provider_idx) {
                    let provider_id = provider.id.clone();
                    if controller::is_no_auth_provider(&provider_id) {
                        state.show_provider_dialog = false;
                        let return_to_picker = state.return_to_model_picker;
                        state.provider_search_query.clear();
                        state.provider_search_cursor = 0;
                        state.selected_provider_idx = 0;
                        controller::setup_no_auth_provider(state, &provider_id);
                        if return_to_picker {
                            state.show_model_picker = true;
                            state.selected_model_picker_idx = 0;
                            state.model_picker_search_query.clear();
                            state.model_picker_search_cursor = 0;
                            state.return_to_model_picker = false;
                        }
                    } else {
                        state.show_provider_dialog = false;
                        state.show_key_dialog = true;
                        state.key_provider_id = Some(provider_id);
                        state.key_input.clear();
                        state.key_cursor = 0;
                        state.provider_search_query.clear();
                        state.provider_search_cursor = 0;
                    }
                }
            }
            _ => {}
        }
        true
    }

    // ── Input submit ──

    fn handle_input_submit(&self, state: &mut AppState) -> bool {
        if state.input.starts_with('/') && !state.input.trim().is_empty() {
            let prefix = state.input.trim().to_lowercase();
            let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
            if let Some((cmd, _)) = matches.get(state.command_popup_selection.min(matches.len().saturating_sub(1))) {
                state.input = cmd.to_string();
                state.input_cursor = Self::char_count(&state.input);
                state.command_popup_selection = 0;
            }
        }

        let input = state.input.clone();
        if input.is_empty() {
            return true;
        }

        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let trimmed = input.trim();

        // ── Slash commands (UI state only; business logic delegated to controller) ──

        if trimmed == "/connect" {
            state.input.clear();
            state.input_cursor = 0;
            state.show_provider_dialog = true;
            state.selected_provider_idx = 0;
            state.provider_search_query.clear();
            state.provider_search_cursor = 0;
            tokio::spawn(controller::fetch_model_registry(self.state.clone()));
            return true;
        }

        if trimmed == "/models" || trimmed == "/model" {
            state.show_model_picker = true;
            state.selected_model_picker_idx = 0;
            state.model_picker_search_query.clear();
            state.model_picker_search_cursor = 0;
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "Select a model to use".to_string(),
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            state.input.clear();
            state.input_cursor = 0;
            return true;
        }

        if trimmed == "/keymap" {
            let bindings = state.keymap.all_bindings();
            let mut lines = vec!["Keyboard Shortcuts:".to_string(), String::new()];
            for (key, action) in &bindings {
                lines.push(format!("  {:20} {}", key, super::render::format_action(action)));
            }
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: lines.join("\n"),
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            state.input.clear();
            state.input_cursor = 0;
            return true;
        }

        if trimmed == "/help" || trimmed == "/?" {
            let help_text = [
                "/connect  - Configure a provider with API key",
                "/models   - Select a model for chat",
                "/apply    - Approve and execute plan",
                "/clear    - Clear conversation",
                "/sh <cmd> - Run a shell command",
                "/help     - Show this help",
                "",
                "Ctrl+P    - Open model picker",
                "Ctrl+X    - Stop current response",
                "Ctrl+C    - Quit",
            ]
            .join("\n");
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: help_text,
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            state.input.clear();
            state.input_cursor = 0;
            return true;
        }

        if trimmed == "/clear" || trimmed == "/new" {
            state.messages.clear();
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "Workflow Agent v0.1.0".to_string(),
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            state.input.clear();
            state.input_cursor = 0;
            state.chat_scroll = 0;
            return true;
        }

        if trimmed == "/sh" {
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "Usage: /sh <command>".to_string(),
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            state.input.clear();
            state.input_cursor = 0;
            return true;
        }

        if let Some(cmd) = trimmed.strip_prefix("/sh ") {
            let arg = cmd.trim();
            if !arg.is_empty() {
                state.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("$ {}", arg),
                    timestamp: now.clone(),
                    status: MessageStatus::Completed,
                });
                controller::execute_shell(&self.state, arg);
            }
            state.input.clear();
            state.input_cursor = 0;
            state.chat_scroll = 0;
            return true;
        }

        if trimmed == "/apply" {
            if let Some(plan) = &mut state.current_plan {
                if plan.status == crate::plan::PlanStatus::Draft {
                    plan.approve();
                    state.mode = AppMode::Build;
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Plan approved. Entered build mode. {}", plan.summary()),
                        timestamp: now.clone(),
                        status: MessageStatus::Completed,
                    });
                } else if plan.status == crate::plan::PlanStatus::Approved {
                    plan.status = crate::plan::PlanStatus::Executing;
                    let plan_summary = plan.summary();
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Starting execution: {}", plan_summary),
                        timestamp: now.clone(),
                        status: MessageStatus::Completed,
                    });
                    controller::execute_plan(&self.state);
                } else {
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Plan not executable. {}", plan.summary()),
                        timestamp: now,
                        status: MessageStatus::Completed,
                    });
                }
            } else {
                state.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: "No plan to apply. Chat first to create a plan.".to_string(),
                    timestamp: now,
                    status: MessageStatus::Completed,
                });
            }
            state.input.clear();
            state.input_cursor = 0;
            state.chat_scroll = 0;
            return true;
        }

        // ── Regular chat message ──

        if state.active_chat_requests > 0 {
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "Already processing a request. Wait or press Ctrl+X to cancel.".to_string(),
                timestamp: now.clone(),
                status: MessageStatus::Completed,
            });
            return true;
        }

        state.input_history.push(input.clone());
        state.input_history_idx = None;

        state.messages.push(ChatMessage {
            role: MessageRole::User,
            content: input.clone(),
            timestamp: now.clone(),
            status: MessageStatus::Completed,
        });

        let response_index = state.messages.len();
        state.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            timestamp: now.clone(),
            status: MessageStatus::Thinking,
        });

        let request_id = state.active_chat_request_id.wrapping_add(1);
        state.active_chat_request_id = request_id;
        state.active_chat_requests = 1;
        let abort_handle: AbortHandle = controller::submit_chat(&self.state, &input, response_index, request_id);
        state.active_chat_abort = Some(abort_handle);

        state.input.clear();
        state.input_cursor = 0;
        state.chat_scroll = 0;
        true
    }

    // ── Utility — kept here because they're purely string-manipulation ──

    fn char_idx_to_byte_idx(s: &str, char_idx: usize) -> usize {
        s.char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(s.len())
    }

    fn char_count(s: &str) -> usize {
        s.chars().count()
    }
}
