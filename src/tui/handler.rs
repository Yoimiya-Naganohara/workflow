//! Keyboard and mouse event handler.
//!
//! Handles input events and mutates UI-level state (cursor position,
//! scroll, selection, dialog opens).  Dialog dispatch is handled by
//! the event loop in [`mod.rs`] — handler only runs when NO dialog
//! is active.
//!
//! Key events are resolved through [`keymap`] so changing a keybinding
//! only requires editing `keymap.rs`.
//! Business logic (provider config, shell, persistence) is delegated
//! to [`crate::tui::controller`] and [`commands`].

use crossterm::event::{KeyCode, KeyModifiers};

use super::Tui;
use super::commands;
use super::keymap::Action;
use super::state::{AppState, ChatMessage, Focus, MessageRole, MessageStatus};
use crate::tui::chat_lines::char_idx_to_byte_idx;
use crate::tui::dialogs::ActiveDialog;
use crate::tui::effect::Effect;

impl Tui {
    /// Handle a key event when no dialog is active.
    /// Returns `true` to continue the event loop, `false` to quit.
    pub(crate) fn handle_chat_keys(&self, state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
        let ui = &mut state.ui;
        let core = &mut state.core;

        // ── Alt+Enter: insert newline (special case not in keymap) ──
        if key.code == KeyCode::Enter && key.modifiers.contains(KeyModifiers::ALT) {
            let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
            ui.input.insert(byte_idx, '\n');
            ui.input_cursor += 1;
            return true;
        }

        // ── Resolve through keymap ──
        match state.keymap.resolve(key) {
            Action::Cancel => {
                ui.focus = Focus::Input;
                ui.input.clear();
                ui.input_cursor = 0;
                ui.command_popup_selection = 0;
            }

            Action::ToggleStatusPanel => {
                ui.show_status_panel = !ui.show_status_panel;
            }

            Action::CancelResponse => {
                if let Some(abort) = ui.active_chat_abort.take() {
                    abort.abort();
                    core.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Stopped current response".to_string(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
                }
            }

            Action::ScrollDown if ui.focus == Focus::Chat => {
                ui.chat_scroll = ui.chat_scroll.saturating_add(1);
            }

            Action::ScrollUp if ui.focus == Focus::Chat => {
                ui.chat_scroll = ui.chat_scroll.saturating_sub(1);
                ui.auto_scroll = false;
            }

            Action::ScrollToTop if ui.focus == Focus::Chat => {
                ui.chat_scroll = 0;
                ui.auto_scroll = false;
            }

            Action::ScrollToBottom if ui.focus == Focus::Chat => {
                ui.chat_scroll = usize::MAX / 2;
                ui.auto_scroll = true;
            }

            Action::SendMessage if ui.focus == Focus::Input => {
                return self.handle_input_submit(state);
            }

            Action::OpenProviderPicker => {
                use crate::tui::dialogs::provider::ProviderDialog;
                core.messages.push(ChatMessage::system(
                    "Select a provider to configure",
                ));
                state.active_dialog = Some(ActiveDialog::Provider(ProviderDialog::new()));
                state.effects.push(Effect::FetchModelRegistry);
                ui.input.clear();
                ui.input_cursor = 0;
            }

            Action::OpenCommandPicker => {
                ui.focus = Focus::Input;
                ui.input = "/".to_string();
                ui.input_cursor = 1;
                ui.command_popup_selection = 0;
            }

            Action::HistoryPrev
                if ui.focus == Focus::Input && !ui.input.starts_with('/') && !ui.input_history.is_empty() =>
            {
                let idx = ui.input_history_idx.unwrap_or(ui.input_history.len());
                let new_idx = idx.saturating_sub(1);
                ui.input_history_idx = Some(new_idx);
                ui.input = ui.input_history[new_idx].clone();
                ui.input_cursor = Self::char_count(&ui.input);
            }

            Action::HistoryNext if ui.focus == Focus::Input && !ui.input.starts_with('/') => {
                if let Some(idx) = ui.input_history_idx {
                    if idx + 1 < ui.input_history.len() {
                        ui.input_history_idx = Some(idx + 1);
                        ui.input = ui.input_history[idx + 1].clone();
                    } else {
                        ui.input_history_idx = None;
                        ui.input.clear();
                    }
                    ui.input_cursor = Self::char_count(&ui.input);
                }
            }

            Action::CommandPrev if ui.focus == Focus::Input && ui.input.starts_with('/') => {
                let prefix = ui.input.trim().to_lowercase();
                let matches: Vec<_> = commands::COMMANDS
                    .iter()
                    .filter(|(cmd, _)| cmd.starts_with(&prefix))
                    .collect();
                if !matches.is_empty() {
                    ui.command_popup_selection =
                        (matches.len() + ui.command_popup_selection).saturating_sub(1) % matches.len();
                }
            }

            Action::CommandNext if ui.focus == Focus::Input && ui.input.starts_with('/') => {
                let prefix = ui.input.trim().to_lowercase();
                let matches: Vec<_> = commands::COMMANDS
                    .iter()
                    .filter(|(cmd, _)| cmd.starts_with(&prefix))
                    .collect();
                if !matches.is_empty() {
                    ui.command_popup_selection = (ui.command_popup_selection + 1) % matches.len();
                }
            }

            Action::TabComplete => {
                if ui.focus == Focus::Input && ui.input.starts_with('/') {
                    let prefix = ui.input.trim().to_lowercase();
                    let matches: Vec<_> = commands::COMMANDS
                        .iter()
                        .filter(|(cmd, _)| cmd.starts_with(&prefix))
                        .collect();
                    if !matches.is_empty() {
                        ui.command_popup_selection = (ui.command_popup_selection + 1) % matches.len();
                    }
                } else {
                    ui.show_status_panel = !ui.show_status_panel;
                }
            }

            Action::TypeChar(c) if ui.focus == Focus::Input => {
                let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                ui.input.insert(byte_idx, c);
                ui.input_cursor += 1;
                ui.input_history_idx = None;
                if ui.input.starts_with('/') {
                    ui.command_popup_selection = 0;
                }
            }

            Action::DeleteChar if ui.focus == Focus::Input => {
                if ui.input_cursor > 0 {
                    ui.input_cursor -= 1;
                    let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                    ui.input.remove(byte_idx);
                    ui.input_history_idx = None;
                    if ui.input.starts_with('/') {
                        ui.command_popup_selection = 0;
                    }
                }
            }

            Action::MoveLeft if ui.focus == Focus::Input => {
                ui.input_cursor = ui.input_cursor.saturating_sub(1);
            }

            Action::MoveRight if ui.focus == Focus::Input => {
                if ui.input_cursor < Self::char_count(&ui.input) {
                    ui.input_cursor += 1;
                }
            }

            // Tab without command prefix — handled at top of call (keymap returns None for bare Tab)
            // Fall through to default match below.
            _ => {}
        }
        true
    }

    // ── Input submit ──

    fn handle_input_submit(&self, state: &mut AppState) -> bool {
        // Auto-complete partial command from popup
        if state.ui.input.starts_with('/') && !state.ui.input.trim().is_empty() {
            let prefix = state.ui.input.trim().to_lowercase();
            let matches: Vec<_> = commands::COMMANDS
                .iter()
                .filter(|(cmd, _)| cmd.starts_with(&prefix))
                .collect();
            if let Some((cmd, _)) = matches.get(state.ui.command_popup_selection.min(matches.len().saturating_sub(1))) {
                state.ui.input = cmd.to_string();
                state.ui.input_cursor = Self::char_count(&state.ui.input);
                state.ui.command_popup_selection = 0;
            }
        }

        let input = state.ui.input.clone();
        if input.is_empty() {
            return true;
        }

        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let trimmed = input.trim();

        // ── Slash commands — delegated to commands module ──
        if trimmed.starts_with('/') && commands::dispatch(trimmed, state, &self.state, &now) {
            return true;
        }

        // ── Regular chat message ──
        let core = &mut state.core;
        let ui = &mut state.ui;

        if ui.active_chat_requests > 0 {
            core.messages.push(ChatMessage::system(
                "Already processing a request. Wait or press Ctrl+X to cancel.",
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            return true;
        }

        ui.auto_scroll = true;
        ui.input_history.push(input.clone());
        ui.input_history_idx = None;

        core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: input.clone(),
            timestamp: now.clone(),
            status: MessageStatus::Completed,
        });

        let response_index = core.messages.len();
        core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            timestamp: now.clone(),
            status: MessageStatus::Thinking,
        });

        let request_id = ui.active_chat_request_id.wrapping_add(1);
        ui.active_chat_request_id = request_id;
        ui.active_chat_requests = 1;

        // Collect data for the chat effect
        let default_tool_prompt = concat!(
            "You are a helpful assistant with access to tools. ",
            "You can read/write files, execute shell commands, and list directories. ",
            "Always use the appropriate tool when asked. ",
            "Produce a concrete result."
        );
        let (provider, model_id, system_prompt) = {
            // ── Configure runtime provider from selected model ──
            let selected_model = core.selected_models.first().cloned();
            if let Some(ref sel) = selected_model {
                let pid = sel.provider_id.clone();
                if core.configured_providers.iter().any(|id| id == &pid) {
                    if let Ok(client) = crate::tui::controller::get_or_create_provider_client(core, &pid) {
                        if let Some(rt) = &core.runtime {
                            if let Ok(mut rt_guard) = rt.try_write() {
                                rt_guard.set_provider_from_state(client);
                                rt_guard.set_default_model(&sel.model_id);
                            }
                        }
                    }
                }
            }

            // Try to get an initial agent
            let agent_id = crate::tui::controller::ensure_initial_agent_sync(core, &input);

            // Read provider + model from the (now-configured) runtime
            let provider = core
                .runtime
                .as_ref()
                .and_then(|rt| rt.try_read().ok().and_then(|r| r.provider.clone()));
            let mid = provider
                .as_ref()
                .and_then(|_| {
                    core.runtime
                        .as_ref()
                        .and_then(|rt| rt.try_read().ok().map(|r| r.model_id.clone()))
                })
                .unwrap_or_default();

            let agent_prompt = agent_id
                .as_ref()
                .and_then(|aid| {
                    let pool = core.agent_pool.try_read().ok()?;
                    pool.get_agent(aid).map(|a| a.config.system_prompt.clone())
                })
                .unwrap_or_else(|| default_tool_prompt.to_string());

            let sp = format!(
                "{}\n\nYou are the workflow agent. Chat with the user, clarify the goal, and delegate tasks by calling the `spawn_agent` tool (roles: planner, developer, tester, reviewer, worker, etc.). You are fully responsible for all spawned agents.\n\nYou have access to tools: read_file, write_file, sh, list_dir, and spawn_agent.",
                agent_prompt
            );
            (provider, mid, sp)
        };

        let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
        ui.active_chat_abort = Some(abort_handle);

        // Build conversation history (exclude current turn's messages)
        let history = {
            let mut hist: Vec<(String, String)> = Vec::new();
            for (i, msg) in core.messages.iter().enumerate() {
                if i >= response_index.saturating_sub(1) {
                    break;
                }
                match msg.role {
                    crate::tui::state::MessageRole::User => {
                        hist.push(("user".to_string(), msg.content.clone()));
                    }
                    crate::tui::state::MessageRole::Agent => {
                        hist.push(("assistant".to_string(), msg.content.clone()));
                    }
                    _ => {}
                }
            }
            hist
        };

        if let Some(provider) = provider {
            state.effects.push(Effect::StartChat {
                input: input.clone(),
                response_index,
                request_id,
                model_id,
                system_prompt,
                history,
                tool_server: core.tool_server.clone(),
                provider,
                runtime: core.runtime.clone(),
                abort_registration,
            });
        } else {
            if let Some(msg) = core.messages.get_mut(response_index) {
                msg.content = "No LLM provider configured".to_string();
                msg.status = MessageStatus::Error;
            }
            ui.active_chat_requests = 0;
            ui.active_chat_abort = None;
        }

        ui.input.clear();
        ui.input_cursor = 0;
        // Don't reset chat_scroll — auto_scroll = true already pins to bottom.
        true
    }

    // ── Utility ──

    fn char_count(s: &str) -> usize {
        s.chars().count()
    }
}
