use anyhow::Result;
use crossterm::event::KeyCode;
use futures::future::{AbortHandle, Abortable};
use std::sync::Arc;
use tokio::sync::RwLock;
use unicode_width::UnicodeWidthChar;

use rig::client::Nothing;
use rig::providers::{llamafile, ollama};

use super::Tui;
use super::state::{
    AgentEntry, AgentStatus, AppMode, AppState, COMMANDS, ChatMessage, Focus, MessageRole, MessageStatus, Panel,
    SelectedModel,
};
use crate::models::filter_providers;

impl Tui {
    pub(crate) fn handle_chat_keys(&self, state: &mut AppState, key: crossterm::event::KeyEvent) -> bool {
        let code = key.code;
        // Key dialog (API key input)
        if state.show_key_dialog {
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
                            let _ = Self::get_or_create_provider_client(state, &provider_id);
                            let now = chrono::Local::now().format("%H:%M:%S").to_string();
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!("{} key set for {}", env_key, provider_name),
                                timestamp: now,
                                status: MessageStatus::Completed,
                            });
                            // Save configured provider
                            if let Err(e) =
                                crate::persistence::save_configured_provider(&provider_id, &env_key, &state.key_input)
                            {
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
            return true;
        }

        // Model picker dialog
        if state.show_model_picker {
            let results = state.models.search_models(&state.model_picker_search_query);
            match code {
                KeyCode::Esc => {
                    state.show_model_picker = false;
                    state.model_picker_search_query.clear();
                    state.model_picker_search_cursor = 0;
                    state.selected_model_picker_idx = 0;
                }
                KeyCode::Char('a')
                    if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                        && !state.models.providers().is_empty() =>
                {
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
                        let byte_idx = Self::char_idx_to_byte_idx(
                            &state.model_picker_search_query,
                            state.model_picker_search_cursor,
                        );
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

                        // If provider not configured, auto-prompt for API key
                        if !state.configured_providers.contains(&provider_id)
                            && !Self::is_no_auth_provider(&provider_id)
                        {
                            state.show_model_picker = false;
                            state.show_key_dialog = true;
                            state.key_provider_id = Some(provider_id);
                            state.key_input.clear();
                            state.key_cursor = 0;
                            state.return_to_model_picker = true;
                            return true;
                        }

                        // Toggle selection
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
                    }
                        // Save selected models
                        if let Err(e) = crate::persistence::save_selected_models(&state.selected_models) {
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: format!("Failed to save: {}", e),
                                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                status: MessageStatus::Completed,
                            });
                        }
                    }
                _ => {}
            }
            return true;
        }

        // Provider dialog
        if state.show_provider_dialog {
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
                    let byte_idx =
                        Self::char_idx_to_byte_idx(&state.provider_search_query, state.provider_search_cursor);
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
                        if Self::is_no_auth_provider(&provider_id) {
                            state.show_provider_dialog = false;
                            let return_to_picker = state.return_to_model_picker;
                            state.provider_search_query.clear();
                            state.provider_search_cursor = 0;
                            state.selected_provider_idx = 0;
                            Self::setup_no_auth_provider(state, &provider_id);
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
            return true;
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
                // If command popup is visible, replace input with selected command
                if state.input.starts_with('/') && !state.input.trim().is_empty() {
                    let prefix = state.input.trim().to_lowercase();
                    let matches: Vec<_> = COMMANDS.iter().filter(|(cmd, _)| cmd.starts_with(&prefix)).collect();
                    if let Some((cmd, _)) =
                        matches.get(state.command_popup_selection.min(matches.len().saturating_sub(1)))
                    {
                        state.input = cmd.to_string();
                        state.input_cursor = Self::char_count(&state.input);
                        state.command_popup_selection = 0;
                    }
                }
                let input = state.input.clone();
                if !input.is_empty() {
                    let now = chrono::Local::now().format("%H:%M:%S").to_string();

                    // Slash commands
                    let trimmed = input.trim();
                    if trimmed == "/connect" {
                        state.input.clear();
                        state.input_cursor = 0;

                        // Show cached providers immediately, if available
                        if state.models.providers().is_empty()
                            && let Some(cached) = crate::persistence::load_provider_cache()
                        {
                            state.models = cached;
                        }
                        state.show_provider_dialog = true;
                        state.selected_provider_idx = 0;
                        state.provider_search_query.clear();
                        state.provider_search_cursor = 0;

                        // Background fetch fresh data
                        let state_clone = self.state.clone();
                        tokio::spawn(async move {
                            let mut registry = crate::models::ModelRegistry::new();
                            match registry.fetch().await {
                                Ok(()) => {
                                    let count = registry.providers().len();
                                    let _ = crate::persistence::save_provider_cache(&registry);
                                    let mut state = state_clone.write().await;
                                    state.models = registry;
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("Loaded {} providers", count),
                                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                        status: MessageStatus::Completed,
                                    });
                                }
                                Err(e) => {
                                    let mut state = state_clone.write().await;
                                    if state.models.providers().is_empty() {
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: format!("Failed to load providers: {}", e),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            status: MessageStatus::Error,
                                        });
                                    } else {
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: format!("Background refresh failed: {}", e),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            status: MessageStatus::Completed,
                                        });
                                    }
                                }
                            }
                        });
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
                            lines.push(format!("  {:20} {}", key, format_action(action)));
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

                    // Handle /sh command - run shell command
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
                            let state_clone = self.state.clone();
                            let arg = arg.to_string();
                            tokio::spawn(async move {
                                let output = tokio::process::Command::new("sh").arg("-c").arg(&arg).output().await;
                                let mut state = state_clone.write().await;
                                let now = chrono::Local::now().format("%H:%M:%S").to_string();
                                match output {
                                    Ok(out) => {
                                        let stdout = String::from_utf8_lossy(&out.stdout);
                                        let stderr = String::from_utf8_lossy(&out.stderr);
                                        let mut content = String::new();
                                        if !stdout.is_empty() {
                                            content.push_str(&stdout);
                                        }
                                        if !stderr.is_empty() {
                                            if !content.is_empty() {
                                                content.push('\n');
                                            }
                                            content.push_str(&stderr);
                                        }
                                        if content.is_empty() {
                                            content = format!("(exit code: {})", out.status.code().unwrap_or(-1));
                                        }
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content,
                                            timestamp: now,
                                            status: MessageStatus::Completed,
                                        });
                                    }
                                    Err(e) => {
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: format!("Error: {}", e),
                                            timestamp: now,
                                            status: MessageStatus::Error,
                                        });
                                    }
                                }
                            });
                        }
                        state.input.clear();
                        state.input_cursor = 0;
                        state.chat_scroll = 0;
                        return true;
                    }

                    // Handle /apply command - switch to build mode
                    if trimmed == "/apply" {
                        if let Some(plan) = &mut state.current_plan {
                            if plan.status == crate::plan::PlanStatus::Draft {
                                // First /apply: approve plan
                                plan.approve();
                                state.mode = AppMode::Build;
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("Plan approved. Entered build mode. {}", plan.summary()),
                                    timestamp: now.clone(),
                                    status: MessageStatus::Completed,
                                });
                            } else if plan.status == crate::plan::PlanStatus::Approved {
                                // Second /apply: start execution
                                plan.status = crate::plan::PlanStatus::Executing;
                                let plan_summary = plan.summary();
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("Starting execution: {}", plan_summary),
                                    timestamp: now.clone(),
                                    status: MessageStatus::Completed,
                                });

                                // Get all pending tasks
                                let tasks: Vec<(usize, String)> = plan
                                    .tasks
                                    .iter()
                                    .filter(|t| t.status == crate::plan::TaskStatus::Pending)
                                    .map(|t| (t.id, t.description.clone()))
                                    .collect();

                                // Execute all tasks
                                let agent_pool = state.agent_pool.clone();
                                let state_clone2 = self.state.clone();
                                tokio::spawn(async move {
                                    for (task_id, task_desc) in tasks {
                                        // Mark task as running
                                        {
                                            let mut state = state_clone2.write().await;
                                            if let Some(p) = &mut state.current_plan {
                                                p.mark_task_running(task_id);
                                            }

                                            // Add worker agent to sidebar
                                            state.agents.push(AgentEntry {
                                                id: format!("worker-{:03}", task_id),
                                                name: format!(
                                                    "Task {}: {}",
                                                    task_id,
                                                    task_desc.chars().take(20).collect::<String>()
                                                ),
                                                status: AgentStatus::Running,
                                                budget: 0,
                                            });
                                        }

                                        // Spawn worker
                                        {
                                            let mut pool = agent_pool.write().await;
                                            let agent = crate::agent::Agent {
                                                id: rand::random(),
                                                name: format!("worker-{}", task_id),
                                                role: "worker".to_string(),
                                                parent_id: None,
                                                children: Vec::new(),
                                                depth: 0,
                                                goal: task_desc.clone(),
                                                config: crate::agent::AgentConfig::default(),
                                                status: crate::agent::AgentStatus::Planning,
                                                result: None,
                                                child_results: Vec::new(),
                                            };
                                            pool.add_agent(agent);
                                        }

                                        // Simulate execution (in real app, would call LLM)
                                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                                        // Mark task completed
                                        {
                                            let mut state = state_clone2.write().await;
                                            if let Some(p) = &mut state.current_plan {
                                                p.mark_task_completed(task_id, "Completed".to_string());
                                            }

                                            // Update worker status
                                            if let Some(agent) = state
                                                .agents
                                                .iter_mut()
                                                .find(|a| a.id == format!("worker-{:03}", task_id))
                                            {
                                                agent.status = AgentStatus::Completed;
                                            }

                                            state.messages.push(ChatMessage {
                                                role: MessageRole::Agent,
                                                content: format!("Task {} completed: {}", task_id, task_desc),
                                                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                                status: MessageStatus::Completed,
                                            });
                                        }
                                    }

                                    // Check if plan is complete
                                    let mut state = state_clone2.write().await;
                                    if let Some(p) = &state.current_plan
                                        && p.status == crate::plan::PlanStatus::Completed
                                    {
                                        state.messages.push(ChatMessage {
                                            role: MessageRole::System,
                                            content: "Plan execution completed!".to_string(),
                                            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                                            status: MessageStatus::Completed,
                                        });
                                    }
                                });
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

                    // Guard against concurrent requests
                    if state.active_chat_requests > 0 {
                        state.messages.push(ChatMessage {
                            role: MessageRole::System,
                            content: "Already processing a request. Wait or press Ctrl+X to cancel.".to_string(),
                            timestamp: now.clone(),
                            status: MessageStatus::Completed,
                        });
                        return true;
                    }

                    // Save to input history
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

                    let state_clone = self.state.clone();
                    let input_clone = input.clone();
                    let request_id = state.active_chat_request_id.wrapping_add(1);
                    state.active_chat_request_id = request_id;
                    state.active_chat_requests = 1;
                    let (abort_handle, abort_registration) = AbortHandle::new_pair();
                    state.active_chat_abort = Some(abort_handle.clone());

                    tokio::spawn(async move {
                        let task = async {
                            let mut state = state_clone.write().await;

                            // Get runtime
                            let runtime = match &state.runtime {
                                Some(r) => r.clone(),
                                None => {
                                    return Err::<String, anyhow::Error>(anyhow::anyhow!(
                                        "Runtime not initialized"
                                    ));
                                }
                            };

                            // Ensure provider is synced to runtime
                            if let Some(selected) = state.selected_models.first() {
                                let provider_id = selected.provider_id.clone();
                                if !state.configured_providers.iter().any(|id| id == &provider_id) {
                                    return Err::<String, anyhow::Error>(anyhow::anyhow!(
                                        "Provider {} is not configured",
                                        provider_id
                                    ));
                                }
                                if let Ok(client) =
                                    crate::tui::handler::Tui::get_or_create_provider_client(&mut state, &provider_id)
                                {
                                    let mut rt = runtime.write().await;
                                    rt.set_provider_from_state(client);
                                }
                            }
                            drop(state);

                            // Run the agent pipeline
                            let rt = runtime.read().await;
                            let pool = Arc::new(RwLock::new(crate::agent::AgentPool::new()));
                            let result = rt.chat_with_goal(&input_clone, &pool).await?;
                            Ok::<String, anyhow::Error>(result)
                        };

                        let result = Abortable::new(task, abort_registration).await;

                        let mut state = state_clone.write().await;
                        let now = chrono::Local::now().format("%H:%M:%S").to_string();
                        match result {
                            Ok(Ok(response)) => {
                                if let Some(message) = state.messages.get_mut(response_index) {
                                    message.content = if response.is_empty() {
                                        "(no text response)".to_string()
                                    } else {
                                        response.clone()
                                    };
                                    message.status = MessageStatus::Completed;
                                }
                                if let Some(mut plan) =
                                    crate::plan::Plan::parse_from_response(&response)
                                {
                                    plan.goal = input_clone.clone();
                                    state.current_plan = Some(plan);
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: "Plan detected. Type /apply to approve and execute."
                                            .to_string(),
                                        timestamp: now.clone(),
                                        status: MessageStatus::Completed,
                                    });
                                }
                            }
                            Ok(Err(e)) => {
                                if let Some(message) = state.messages.get_mut(response_index) {
                                    message.content = format!("Error: {}", e);
                                    message.status = MessageStatus::Error;
                                } else {
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::Agent,
                                        content: format!("Error: {}", e),
                                        timestamp: now,
                                        status: MessageStatus::Error,
                                    });
                                }
                            }
                            Err(_) => {
                                if let Some(message) = state.messages.get_mut(response_index) {
                                    message.content += " (cancelled)";
                                    message.status = MessageStatus::Completed;
                                }
                            }
                        }

                        if state.active_chat_request_id == request_id {
                            state.active_chat_abort = None;
                            state.active_chat_requests = 0;
                        }
                    });

                    state.input.clear();
                    state.input_cursor = 0;
                    state.chat_scroll = 0;
                }
            }
            // Input history navigation (only when not in command mode)
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
            // Command popup navigation
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
            KeyCode::Char(c) if state.focus == Focus::Input && (key.modifiers.is_empty() || key.modifiers == crossterm::event::KeyModifiers::SHIFT) => {
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

    pub(crate) fn is_no_auth_provider(provider_id: &str) -> bool {
        matches!(provider_id, "ollama" | "llamafile")
    }

    fn setup_no_auth_provider(state: &mut AppState, provider_id: &str) {
        if state.configured_providers.contains(&provider_id.to_string()) {
            return;
        }
        state.configured_providers.push(provider_id.to_string());
        state.models.select_provider(provider_id);
        let _ = Self::get_or_create_provider_client(state, provider_id);
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        let provider_name = state
            .models
            .providers()
            .iter()
            .find(|p| p.id == provider_id)
            .map(|p| p.name.as_str())
            .unwrap_or(provider_id);
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("{} configured (no API key required)", provider_name),
            timestamp: now,
            status: MessageStatus::Completed,
        });
    }

    pub(crate) fn get_or_create_provider_client(
        state: &mut AppState,
        provider_id: &str,
    ) -> Result<Arc<crate::llm::LlmProvider>> {
        if let Some(client) = state.provider_clients.get(provider_id) {
            return Ok(client.clone());
        }

        let provider = state
            .models
            .providers()
            .iter()
            .find(|p| p.id == provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider not found: {}", provider_id))?;

        // Handle no-auth providers (ollama, llamafile)
        if Self::is_no_auth_provider(provider_id) {
            let client = match provider_id {
                "ollama" => {
                    let mut builder = ollama::Client::builder().api_key(Nothing);
                    if let Some(url) = provider.api.as_deref() {
                        builder = builder.base_url(url);
                    }
                    Arc::new(crate::llm::LlmProvider::Ollama(builder.build()?))
                }
                "llamafile" => {
                    let url = provider.api.as_deref().unwrap_or("http://localhost:8080");
                    Arc::new(crate::llm::LlmProvider::Llamafile(llamafile::Client::from_url(url)?))
                }
                _ => anyhow::bail!("unexpected no-auth provider: {}", provider_id),
            };
            state.provider_clients.insert(provider_id.to_string(), client.clone());
            return Ok(client);
        }

        let env_key = provider.env.first().cloned().unwrap_or_default();
        if env_key.is_empty() {
            anyhow::bail!("Provider {} has no env var configured", provider_id);
        }

        let api_key = state
            .api_keys
            .get(&env_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{} not set. Press Ctrl+P to configure.", env_key))?;
        let client = Arc::new(crate::llm::LlmProvider::from_key(
            &api_key,
            provider.api.as_deref(),
            provider_id,
        )?);
        state.provider_clients.insert(provider_id.to_string(), client.clone());
        Ok(client)
    }

    fn char_idx_to_byte_idx(s: &str, char_idx: usize) -> usize {
        s.char_indices()
            .nth(char_idx)
            .map(|(byte_idx, _)| byte_idx)
            .unwrap_or(s.len())
    }

    fn char_count(s: &str) -> usize {
        s.chars().count()
    }

    pub(crate) fn display_width_up_to(s: &str, char_idx: usize) -> usize {
        s.chars()
            .take(char_idx)
            .map(|c| UnicodeWidthChar::width(c).unwrap_or(0))
            .sum()
    }
}

fn format_action(action: &super::keymap::Action) -> String {
    match action {
        super::keymap::Action::Quit => "Quit the application",
        super::keymap::Action::CancelResponse => "Cancel current response",
        super::keymap::Action::ToggleStatusPanel => "Show/hide status panel",
        super::keymap::Action::MoveUp => "Move up / Previous item",
        super::keymap::Action::MoveDown => "Move down / Next item",
        super::keymap::Action::MoveLeft => "Move cursor left",
        super::keymap::Action::MoveRight => "Move cursor right",
        super::keymap::Action::ScrollUp => "Scroll chat up",
        super::keymap::Action::ScrollDown => "Scroll chat down",
        super::keymap::Action::ScrollToTop => "Scroll to top",
        super::keymap::Action::ScrollToBottom => "Scroll to bottom",
        super::keymap::Action::Confirm => "Confirm selection",
        super::keymap::Action::Cancel => "Cancel / Close dialog",
        super::keymap::Action::OpenModelPicker => "Open model picker",
        super::keymap::Action::OpenProviderDialog => "Open provider dialog",
        super::keymap::Action::SwitchPanel => "Switch panel",
        super::keymap::Action::SendMessage => "Send message",
        super::keymap::Action::TypeChar(_) => "Type character",
        super::keymap::Action::DeleteChar => "Delete character",
        super::keymap::Action::HistoryPrev => "Previous input history",
        super::keymap::Action::HistoryNext => "Next input history",
        super::keymap::Action::CommandPrev => "Previous command",
        super::keymap::Action::CommandNext => "Next command",
        super::keymap::Action::None => "",
    }
    .to_string()
}
