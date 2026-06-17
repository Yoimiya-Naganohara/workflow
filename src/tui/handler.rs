//! Keyboard and mouse event handler.
//!
//! Handles all key events including popup navigation.
//! Business logic delegated to [`crate::tui::controller`] and [`commands`].

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyModifiers};

use super::Tui;
use super::commands;
use super::keymap::Action;
use super::state::{AppState, ChatMessage, Focus, MessageRole, MessageStatus, PopupMode};
use crate::tui::chat_lines::char_idx_to_byte_idx;
use crate::tui::effect::Effect;

impl Tui {
    /// Handle a key event. Returns `true` to continue, `false` to quit.
    pub(crate) fn handle_chat_keys(
        &self,
        state: &mut AppState,
        key: crossterm::event::KeyEvent,
    ) -> bool {
        // ── Popup navigation (highest priority) ──
        if state.popup_mode != PopupMode::None {
            return self.handle_popup_keys(state, key);
        }

        let ui = &mut state.ui;
        let core = &mut state.core;

        // ── Ctrl+J / Ctrl+Enter: insert newline ──
        // Enter (0x0D) and Shift+Enter are indistinguishable in terminals.
        // Ctrl+Enter sends 0x0A (Ctrl+J) which IS distinguishable.
        if key.code == KeyCode::Char('j') && key.modifiers.contains(KeyModifiers::CONTROL) {
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

            Action::CancelResponse => {
                if let Some(abort) = ui.active_chat_abort.take() {
                    abort.abort();
                    core.messages
                        .push(ChatMessage::system("Stopped current response"));
                }
            }

            Action::ScrollDown if ui.focus == Focus::Chat => {
                ui.chat_scroll = ui.chat_scroll.saturating_add(3);
            }

            Action::ScrollUp if ui.focus == Focus::Chat => {
                ui.chat_scroll = ui.chat_scroll.saturating_sub(3);
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
                state.popup_mode = PopupMode::Providers;
                state.popup_selected = 0;
                state.effects.push(Effect::FetchModelRegistry);
                ui.input.clear();
                ui.input_cursor = 0;
            }

            Action::OpenCommandPicker => {
                ui.focus = Focus::Input;
                ui.input = "/".to_string();
                ui.input_cursor = 1;
                ui.command_popup_selection = 0;
                state.popup_mode = PopupMode::Commands;
            }

            Action::InspectAgent => {
                // Open the detail popup for the currently selected tree item.
                let ids = &ui.tree_agent_ids;
                let idx = ui.selected_agent_idx.min(ids.len().saturating_sub(1));
                if let Some(&agent_id) = ids.get(idx) {
                    state.popup_mode = PopupMode::AgentDetail { agent_id };
                    state.popup_selected = 0;
                }
            }

            Action::MoveUp if !ui.tree_agent_ids.is_empty() => {
                ui.selected_agent_idx = ui.selected_agent_idx.saturating_sub(1);
            }

            Action::MoveDown if !ui.tree_agent_ids.is_empty() => {
                let max = ui.tree_agent_ids.len().saturating_sub(1);
                ui.selected_agent_idx = (ui.selected_agent_idx + 1).min(max);
            }

            Action::HistoryPrev
                if ui.focus == Focus::Input
                    && !ui.input.starts_with('/')
                    && !ui.input_history.is_empty() =>
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
                    ui.command_popup_selection = (matches.len() + ui.command_popup_selection)
                        .saturating_sub(1)
                        % matches.len();
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
                        state.popup_selected = (state.popup_selected + 1) % matches.len();
                    }
                }
            }

            Action::TypeChar(c) if ui.focus == Focus::Input => {
                let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                ui.input.insert(byte_idx, c);
                ui.input_cursor += 1;
                ui.input_history_idx = None;
                if c == '@' && state.popup_mode == PopupMode::None {
                    state.popup_mode = PopupMode::FilePicker {
                        query: String::new(),
                    };
                    state.popup_selected = 0;
                } else if ui.input.starts_with('/') && state.popup_mode == PopupMode::None {
                    state.popup_mode = PopupMode::Commands;
                    ui.command_popup_selection = 0;
                }
            }

            Action::DeleteChar if ui.focus == Focus::Input => {
                if ui.input_cursor > 0 {
                    ui.input_cursor -= 1;
                    let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                    ui.input.remove(byte_idx);
                    ui.input_history_idx = None;
                    if (matches!(state.popup_mode, PopupMode::FilePicker { .. })
                        && !ui.input.contains('@'))
                        || ((ui.input.is_empty() || !ui.input.starts_with('/'))
                            && matches!(state.popup_mode, PopupMode::Commands))
                    {
                        state.popup_mode = PopupMode::None;
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

            _ => {}
        }
        true
    }

    /// Handle keys when a popup is active.
    fn handle_popup_keys(&self, state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
        let ui = &mut state.ui;
        let core = &mut state.core;

        match key.code {
            KeyCode::Esc => {
                let was_key_input = matches!(state.popup_mode, PopupMode::KeyInput);
                state.popup_mode = PopupMode::None;
                ui.command_popup_selection = 0;
                state.popup_selected = 0;
                if was_key_input {
                    // Clear the input so the partially-typed API key doesn't remain visible.
                    ui.input.clear();
                    ui.input_cursor = 0;
                }
                true
            }

            KeyCode::Up => {
                state.popup_selected = state.popup_selected.saturating_sub(1);
                true
            }

            KeyCode::Down => {
                state.popup_selected += 1;
                true
            }

            KeyCode::Enter => {
                match &state.popup_mode {
                    PopupMode::Commands => {
                        let prefix = ui.input.trim().to_lowercase();
                        let query = prefix.trim_start_matches('/');
                        let matches: Vec<_> = commands::COMMANDS
                            .iter()
                            .filter(|(cmd, _)| cmd.to_lowercase().contains(query))
                            .collect();
                        if let Some((cmd, _)) =
                            matches.get(state.popup_selected.min(matches.len().saturating_sub(1)))
                        {
                            state.popup_mode = PopupMode::None;
                            state.popup_selected = 0;
                            ui.command_popup_selection = 0;
                            // Submit directly — dispatch handler decides whether to open
                            // a SubCommand/Providers/ShellInput popup or execute immediately.
                            ui.input = cmd.to_string();
                            ui.input_cursor = Self::char_count(&ui.input);
                            return self.handle_input_submit(state);
                        }
                    }
                    PopupMode::SubCommand { parent, items } => {
                        // Filter items by input text (same logic as popup.rs render)
                        let filter_text = {
                            let input = &ui.input;
                            if let Some(after) = input.strip_prefix(parent) {
                                if let Some(stripped) = after.strip_prefix(' ') {
                                    stripped
                                } else {
                                    after
                                }
                            } else {
                                input.as_str()
                            }
                        };
                        let fl = filter_text.to_lowercase();
                        let filtered: Vec<_> = if filter_text.is_empty() {
                            items.iter().collect()
                        } else {
                            items
                                .iter()
                                .filter(|(name, _)| name.to_lowercase().contains(&fl))
                                .collect()
                        };
                        if let Some((name, _)) =
                            filtered.get(state.popup_selected.min(filtered.len().saturating_sub(1)))
                        {
                            let full_cmd = format!("{} {}", parent, name);
                            // Submit directly — dispatch handler decides whether
                            // to execute or open a further ShellInput prompt.
                            state.popup_mode = PopupMode::None;
                            state.popup_selected = 0;
                            ui.input = full_cmd;
                            ui.input_cursor = Self::char_count(&ui.input);
                            return self.handle_input_submit(state);
                        }
                    }
                    PopupMode::ShellInput { cmd, input: _ } => {
                        // Read the actual typed text from ui.input (the popup's stored
                        // input field is never updated — keyboard input goes to ui.input).
                        let arg = ui.input.trim();
                        if arg.is_empty() {
                            // Don't close popup on empty input — show the hint.
                            return true;
                        }
                        let full_cmd = format!("{} {}", cmd, arg);
                        state.popup_mode = PopupMode::None;
                        state.popup_selected = 0;
                        ui.input = full_cmd;
                        ui.input_cursor = Self::char_count(&ui.input);
                        return self.handle_input_submit(state);
                    }
                    PopupMode::Providers => {
                        // Select provider from filtered list.
                        // Clone all needed data BEFORE any mutable access.
                        let selected: Option<(String, String, bool)> = {
                            let filtered =
                                crate::models::filter_providers(core.models.providers(), &ui.input);
                            filtered.get(state.popup_selected).copied().map(|provider| {
                                (
                                    provider.id.clone(),
                                    provider.name.clone(),
                                    crate::tui::controller::is_no_auth_provider(&provider.id),
                                )
                            })
                        };
                        if let Some((provider_id, name, is_no_auth)) = selected {
                            if is_no_auth {
                                if !core.configured_providers.contains(&provider_id) {
                                    core.configured_providers.push(provider_id.clone());
                                }
                                core.models.select_provider(&provider_id);
                                let _ = crate::tui::controller::get_or_create_provider_client(
                                    core,
                                    &provider_id,
                                );
                                let _ = crate::persistence::save_configured_provider(
                                    &provider_id,
                                    "",
                                    "",
                                );
                                core.messages
                                    .push(ChatMessage::system(format!("{} configured", name)));
                            } else {
                                state.popup_mode = PopupMode::KeyInput;
                                state.popup_key_provider = Some(provider_id);
                                state.popup_selected = 0;
                                ui.input.clear();
                                ui.input_cursor = 0;
                                return true;
                            }
                        }
                        state.popup_mode = PopupMode::None;
                    }
                    PopupMode::KeyInput => {
                        // Set API key
                        if let Some(ref provider_id) = state.popup_key_provider.clone() {
                            let key_value = ui.input.clone();
                            if !key_value.is_empty() {
                                // Clone provider info before any mutable access
                                let provider_info: Option<(String, String)> = core
                                    .models
                                    .providers()
                                    .iter()
                                    .find(|p| p.id == *provider_id)
                                    .map(|p| {
                                        let env_key = p.env.first().cloned().unwrap_or_default();
                                        let name = p.name.clone();
                                        (env_key, name)
                                    });
                                if let Some((env_key, name)) = provider_info {
                                    if !env_key.is_empty() {
                                        core.api_keys.insert(env_key.clone(), key_value.clone());
                                        core.models.select_provider(provider_id);
                                        if !core.configured_providers.contains(provider_id) {
                                            core.configured_providers.push(provider_id.clone());
                                        }
                                        core.provider_clients.remove(provider_id);
                                        let _ =
                                            crate::tui::controller::get_or_create_provider_client(
                                                core,
                                                provider_id,
                                            );
                                        core.messages
                                            .push(ChatMessage::system(format!("{} key set", name)));
                                        let _ = crate::tui::controller::save_api_key(
                                            provider_id,
                                            &env_key,
                                            &key_value,
                                        );
                                    }
                                }
                            }
                        }
                        state.popup_mode = PopupMode::None;
                        state.popup_key_provider = None;
                        // Clear the input so the API key doesn't remain visible in the chat box.
                        ui.input.clear();
                        ui.input_cursor = 0;
                    }
                    PopupMode::ModelPicker => {
                        // Toggle model selection
                        let results = {
                            let configured = &core.configured_providers;
                            core.models.search_configured_models(&ui.input, configured)
                        };
                        if let Some((p, m)) = results.get(state.popup_selected) {
                            let pid = p.id.clone();
                            let mid = m.id.clone();
                            let pname = p.name.clone();
                            let mname = m.name.clone();
                            if let Some(pos) = core
                                .selected_models
                                .iter()
                                .position(|sm| sm.provider_id == pid && sm.model_id == mid)
                            {
                                core.selected_models.remove(pos);
                                core.messages.push(ChatMessage::system(format!(
                                    "Removed: {} / {}",
                                    pname, mname
                                )));
                            } else {
                                core.selected_models.push(crate::tui::state::SelectedModel {
                                    provider_id: pid,
                                    model_id: mid,
                                    provider_name: pname.clone(),
                                    model_name: mname.clone(),
                                });
                                core.messages.push(ChatMessage::system(format!(
                                    "Added: {} / {}",
                                    pname, mname
                                )));
                            }
                            crate::tui::controller::save_selected_models(&core.selected_models)
                                .ok();
                        }
                        // Don't close — allow multi-select
                        return true;
                    }
                    PopupMode::FilePicker { .. } => {
                        // Select file and insert its path after @
                        let files = crate::tui::popup::get_project_files_cached();
                        let query = crate::tui::popup::file_picker_query(&ui.input);
                        let q = query.to_lowercase();
                        let filtered: Vec<&String> = if q.is_empty() {
                            files.iter().collect()
                        } else {
                            files
                                .iter()
                                .filter(|f| f.to_lowercase().contains(&q))
                                .collect()
                        };
                        if let Some(path) =
                            filtered.get(state.popup_selected.min(filtered.len().saturating_sub(1)))
                        {
                            // Replace text from last @ to cursor/end with @path
                            if let Some(at_pos) = ui.input.rfind('@') {
                                let new_input = format!("{}@{}", &ui.input[..at_pos], path);
                                ui.input = new_input;
                                ui.input_cursor = Self::char_count(&ui.input);
                            }
                        }
                        state.popup_mode = PopupMode::None;
                        state.popup_selected = 0;
                    }
                    PopupMode::AgentDetail { .. } => {
                        // Enter closes the agent detail popup.
                    }
                    PopupMode::None => {}
                }
                state.popup_mode = PopupMode::None;
                state.popup_selected = 0;
                true
            }

            KeyCode::Tab => {
                // Tab cycles through command matches or does nothing in other popups
                if state.popup_mode == PopupMode::Commands {
                    let prefix = ui.input.trim().to_lowercase();
                    let query = prefix.trim_start_matches('/');
                    let matches: Vec<_> = commands::COMMANDS
                        .iter()
                        .filter(|(cmd, _)| cmd.to_lowercase().contains(query))
                        .collect();
                    if !matches.is_empty() {
                        state.popup_selected = (state.popup_selected + 1) % matches.len();
                    }
                }
                true
            }

            // All other keys: let the input handle them (typing, backspace, etc.)
            _ => {
                // Forward to normal input handling
                match key.code {
                    KeyCode::Char(c) => {
                        let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                        ui.input.insert(byte_idx, c);
                        ui.input_cursor += 1;
                        state.popup_selected = 0;
                    }
                    KeyCode::Backspace => {
                        if ui.input_cursor > 0 {
                            ui.input_cursor -= 1;
                            let byte_idx = char_idx_to_byte_idx(&ui.input, ui.input_cursor);
                            ui.input.remove(byte_idx);
                            state.popup_selected = 0;
                            // Close popup if input no longer matches
                            if (matches!(state.popup_mode, PopupMode::FilePicker { .. })
                                && !ui.input.contains('@'))
                                || ui.input.is_empty()
                                || (!ui.input.starts_with('/')
                                    && matches!(state.popup_mode, PopupMode::Commands))
                            {
                                state.popup_mode = PopupMode::None;
                            }
                        }
                    }
                    KeyCode::Left => {
                        ui.input_cursor = ui.input_cursor.saturating_sub(1);
                    }
                    KeyCode::Right => {
                        if ui.input_cursor < Self::char_count(&ui.input) {
                            ui.input_cursor += 1;
                        }
                    }
                    _ => {}
                }
                true
            }
        }
    }

    // ── Input submit ──

    fn handle_input_submit(&self, state: &mut AppState) -> bool {
        let raw = state.ui.input.clone();
        if raw.is_empty() {
            return true;
        }

        // Resolve paste marker → actual text so commands like /role work
        // even when a paste marker is at the start of the input.
        let (input, paste_content) = if let Some(pc) = state.ui.pending_paste.take() {
            if let Some(start) = raw.find("[Pasted") {
                let after = raw[start..]
                    .find(']')
                    .map(|e| start + e + 1)
                    .unwrap_or(raw.len());
                (format!("{}{}{}", &raw[..start], pc, &raw[after..]), None)
            } else {
                (raw, Some(pc))
            }
        } else {
            (raw, None)
        };

        if input.is_empty() {
            return true;
        }

        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let trimmed = input.trim();

        // ── Slash commands ──
        if trimmed.starts_with('/') && commands::dispatch(trimmed, state, &now) {
            // Keep input for interactive popups where the user continues
            // typing to filter args (parent with space like "/role default").
            let is_interactive = matches!(
                &state.popup_mode,
                PopupMode::SubCommand { parent, .. } if parent.contains(' ')
            );
            if !is_interactive {
                state.ui.input.clear();
                state.ui.input_cursor = 0;
            }
            return true;
        }

        // ── Regular chat message ──
        let core = &mut state.core;
        let ui = &mut state.ui;

        if core.selected_models.is_empty() {
            core.messages.push(ChatMessage::system(
                "No model selected. Use `/models` to pick one.",
            ));
            ui.input.clear();
            ui.input_cursor = 0;
            return true;
        }

        if ui.active_chat_requests > 0 {
            core.messages.push(ChatMessage::system(
                "Already processing a request. Wait or press Ctrl+X to cancel.",
            ));
            return true;
        }

        ui.auto_scroll = true;
        ui.input_history.push(input.clone());
        ui.input_history_idx = None;

        // Append paste content (if not merged into input above) as a code block
        let content = if let Some(pc) = paste_content {
            format!("{}\n```\n{}```", input, pc)
        } else {
            input.clone()
        };

        core.messages.push(ChatMessage {
            role: MessageRole::User,
            content,
            reasoning: String::new(),
            timestamp: now.clone(),
            status: MessageStatus::Completed,
        });

        let response_index = core.messages.len();
        core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            reasoning: String::new(),
            timestamp: now.clone(),
            status: MessageStatus::Thinking,
        });

        let request_id = ui.active_chat_request_id.wrapping_add(1);
        ui.active_chat_request_id = request_id;
        ui.active_chat_requests = 1;

        const DEFAULT_TOOL_PROMPT: &str = "Must follow user instructions and use available tools.";

        let default_tool_prompt = DEFAULT_TOOL_PROMPT;

        let (provider, model_id, system_prompt) = {
            let selected_model = core.selected_models.first().cloned();
            if let Some(ref sel) = selected_model {
                let pid = sel.provider_id.clone();
                if core.configured_providers.iter().any(|id| id == &pid) {
                    if let Ok(client) =
                        crate::tui::controller::get_or_create_provider_client(core, &pid)
                    {
                        if let Some(rt) = &core.runtime {
                            if let Ok(mut rt_guard) = rt.try_write() {
                                rt_guard.set_provider_from_state(Arc::new(client.inner.clone()));
                                rt_guard.set_default_model(&sel.model_id);
                            }
                        }
                    }
                }
            }

            let agent_id = crate::tui::controller::ensure_initial_agent_sync(core, &input);
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

            let (agent_prompt, agent_role) = agent_id
                .as_ref()
                .and_then(|aid| {
                    let pool = core.agent_pool.try_read().ok()?;
                    let role = pool.get_agent(aid)?.role.clone();
                    let prompt = {
                        let rt = core.runtime.as_ref()?.try_read().ok()?;
                        rt.get_role_template(&role).map(|t| t.system_prompt.clone())
                    }?;
                    Some((prompt, role))
                })
                .unwrap_or_else(|| (default_tool_prompt.to_string(), String::new()));

            // Inject role memos into the root agent's system prompt.
            let memos = core
                .agent_pool
                .try_read()
                .ok()
                .and_then(|pool| pool.format_role_memos(&agent_role))
                .unwrap_or_default();

            let agent_prompt = format!(
                "{}\n\n{}\n\n{}{}",
                agent_prompt,
                crate::core::types::MEMO_INSTRUCTIONS,
                crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
                memos,
            );

            (provider, mid, agent_prompt)
        };

        let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
        ui.active_chat_abort = Some(abort_handle);

        let history = {
            let mut hist: Vec<(String, String)> = Vec::new();
            for (i, msg) in core.messages.iter().enumerate() {
                if i >= response_index.saturating_sub(1) {
                    break;
                }
                match msg.role {
                    MessageRole::User => hist.push(("user".to_string(), msg.content.clone())),
                    MessageRole::Agent => hist.push(("assistant".to_string(), msg.content.clone())),
                    MessageRole::System => {
                        // System messages (child completion notifications, etc.)
                        // are included so the agent sees async results.
                        hist.push(("system".to_string(), msg.content.clone()))
                    }
                    _ => {}
                }
            }
            hist
        };

        if let Some(provider) = provider {
            // Reset API token tracking for the new request
            ui.has_api_tokens = false;
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
        true
    }

    fn char_count(s: &str) -> usize {
        s.chars().count()
    }
}
