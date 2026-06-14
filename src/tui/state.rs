use std::collections::HashMap;
use std::sync::Arc;

use futures::future::AbortHandle;
use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::models::ModelRegistry;
use crate::reflection::ReflectionConfig;
use crate::runtime::AgentRuntime;
use crate::tools::ToolServerHandle;
use crate::tui::effect::AppEvent;

// ── Shared types ──

/// Statistics about the dual-track experience pool.
#[derive(Debug, Clone, Default)]
pub struct ExperiencePoolStats {
    pub total: usize,
    pub bedrock: usize,
    pub fluid: usize,
    pub last_flush_result: Option<String>,
}

#[derive(Clone, PartialEq)]
pub enum Focus {
    Sidebar,
    Chat,
    Input,
}

#[derive(Clone, PartialEq)]
pub enum Panel {
    Chat,
}

#[derive(Clone, PartialEq)]
pub enum AppMode {
    Plan,
    Build,
}

#[derive(Clone, PartialEq)]
pub enum MessageStatus {
    Thinking,
    Streaming,
    Completed,
    Error,
}

#[derive(Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        Self {
            role: MessageRole::System,
            content: content.into(),
            timestamp: now,
            status: MessageStatus::Completed,
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SelectedModel {
    pub provider_id: String,
    pub model_id: String,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub budget: u64,
}

#[derive(Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Suspended,
    Completed,
    Failed,
}

// ── Grouped state structs ──

/// Business / domain state — persisted or long-lived.
pub struct CoreState {
    pub messages: Vec<ChatMessage>,
    pub models: ModelRegistry,
    pub configured_providers: Vec<String>,
    pub api_keys: HashMap<String, String>,
    pub provider_clients: HashMap<String, Arc<crate::llm::LlmProvider>>,
    pub selected_models: Vec<SelectedModel>,
    pub agents: Vec<AgentEntry>,
    pub agent_pool: Arc<RwLock<crate::agent::AgentPool>>,
    pub runtime: Option<Arc<RwLock<AgentRuntime>>>,
    pub tool_server: ToolServerHandle,
    pub responsible_agent_id: Option<AgentId>,
    pub default_role: String,
    pub reflection: ReflectionConfig,
    /// Track the last chat context for retry.
    pub last_chat_request_id: u64,
}

/// Transient UI state — reset on restart.
pub struct UiState {
    pub panel: Panel,
    pub focus: Focus,
    pub mode: AppMode,
    pub input: String,
    pub input_cursor: usize,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub auto_scroll: bool,
    pub think_frame: u8,
    pub command_popup_selection: usize,
    pub input_history: Vec<String>,
    pub input_history_idx: Option<usize>,
    pub active_chat_request_id: u64,
    pub active_chat_abort: Option<AbortHandle>,
    pub active_chat_requests: usize,
    pub budget_used: u64,
    pub budget_total: u64,
    pub permits_available: usize,
    pub permits_total: usize,
    pub context_limit: u64,
    pub current_plan: Option<crate::agent::plan::Plan>,
}

// ── AppState ──

#[derive(Default)]
pub struct AppState {
    pub ui: UiState,
    pub core: CoreState,
    pub popup_mode: PopupMode,
    pub popup_selected: usize,
    pub popup_key_provider: Option<String>,
    pub keymap: super::keymap::Keymap,
    pub pool_stats: ExperiencePoolStats,
    /// Effects queued for async execution (drained by event loop).
    pub effects: Vec<crate::tui::effect::Effect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PopupMode {
    #[default]
    None,
    Commands,
    SubCommand {
        parent: String,
        items: Vec<(String, String)>,
    },
    Providers,
    KeyInput,
    ModelPicker,
    FilePicker {
        query: String,
    },
}

impl AppState {
    /// Apply an async result to the state.
    pub fn handle_event(&mut self, event: AppEvent) {
        use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
        match event {
            AppEvent::ModelRegistryFetched { count } => {
                if let Some(cached) = crate::persistence::load_provider_cache() {
                    self.core.models = cached;
                }
                self.core
                    .messages
                    .push(ChatMessage::system(format!("Loaded {} providers", count)));
            }
            AppEvent::ModelRegistryFailed { error, is_empty } => {
                let status = if is_empty {
                    MessageStatus::Error
                } else {
                    MessageStatus::Completed
                };
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status,
                });
            }
            AppEvent::ShellOutput { content, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp,
                    status: MessageStatus::Completed,
                });
            }
            AppEvent::ShellError { error, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    timestamp,
                    status: MessageStatus::Error,
                });
            }
            AppEvent::PoolQueryResult {
                content,
                timestamp,
                is_error,
            } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp,
                    status: if is_error {
                        MessageStatus::Error
                    } else {
                        MessageStatus::Completed
                    },
                });
            }
            AppEvent::ChatToken { response_index, text } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content.push_str(&text);
                        msg.status = MessageStatus::Streaming;
                    }
                }
            }
            AppEvent::ChatCompleted {
                response_index,
                request_id,
                full_response,
                input,
                runtime,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        // Only overwrite if the message hasn't already been completed.
                        // Prevents duplicate events from overwriting completed content.
                        if matches!(msg.status, MessageStatus::Thinking | MessageStatus::Streaming) {
                            msg.content = full_response.clone();
                        }
                        msg.status = MessageStatus::Completed;
                    }
                }
                if self.ui.active_chat_requests > 0 {
                    self.ui.active_chat_requests -= 1;
                }
                self.ui.active_chat_abort = None;

                // ── Trigger self-check reflection if enabled ──
                if self.core.reflection.auto_reflect {
                    self.trigger_self_check(response_index, request_id, full_response.clone(), &input, &runtime);
                }

                // ── Save exchange to responsible agent's context ──
                if let Some(agent_id) = self.core.responsible_agent_id {
                    if let Ok(mut pool) = self.core.agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.context.push(crate::llm::types::Message {
                                role: "user".to_string(),
                                content: input.clone(),
                            });
                            agent.context.push(crate::llm::types::Message {
                                role: "assistant".to_string(),
                                content: full_response.clone(),
                            });
                            // Prevent unbounded growth: keep last 100 exchanges
                            if agent.context.len() > 200 {
                                agent.context.drain(0..agent.context.len() - 200);
                            }
                        }
                    }
                }
            }
            AppEvent::ChatError {
                response_index,
                request_id: _,
                error,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content = error;
                        msg.status = MessageStatus::Error;
                    }
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
            }
            AppEvent::ChatCancelled {
                response_index,
                request_id: _,
            } => {
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content += " (cancelled)";
                        msg.status = MessageStatus::Completed;
                    }
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
            }
            AppEvent::OptimizationResult {
                role_name,
                original: _,
                improved,
                summary,
                stats: _,
            } => {
                // Auto-apply: update system_prompt, increment version, recompute embedding.
                if let Some(runtime) = &self.core.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        if let Some(mut tpl) = rt.get_role_template(&role_name) {
                            tpl.system_prompt = improved.clone();
                            tpl.version += 1;
                            tpl.updated_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            rt.save_role_template(tpl);
                            // Recompute embedding in background.
                            rt.compute_role_embeddings_async();
                        }
                    }
                }
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization complete.\n\n{}  \n\nNew prompt applied (version ++).",
                    role_name, summary
                )));
            }
            AppEvent::OptimizationError { role_name, error } => {
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization failed: {}",
                    role_name, error
                )));
            }
            AppEvent::ChatToolCall {
                response_index,
                name,
                args,
                timestamp: _,
            } => {
                // Append tool call to the streaming agent message
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        if !msg.content.is_empty() {
                            msg.content.push('\n');
                        }
                        msg.content.push_str(&format!("{} — {}", name, args));
                    }
                } else {
                    // Fallback: insert as separate message if no streaming slot found
                    self.core.messages.push(ChatMessage {
                        role: MessageRole::Decision,
                        content: format!("{} — {}", name, args),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
                }
            }
            AppEvent::SelfCheckResult {
                response_index: _,
                request_id,
                passed,
                attempt,
                input: _,
                system_prompt,
                history,
                tool_server,
                provider,
                model_id,
                runtime,
                abort_registration: _,
                feedback,
            } => {
                if passed {
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let max_attempts = self.core.reflection.max_attempts;
                if attempt >= max_attempts {
                    self.core.messages.push(ChatMessage::system(format!(
                        "⚠️ Reflection: max retries ({}) reached. Final result shown above.",
                        max_attempts
                    )));
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let new_attempt = attempt + 1;
                let now = chrono::Local::now().format("%H:%M:%S").to_string();
                let feedback_msg = format!("🔄 Reflection #{} — revisiting response\n\n{}", new_attempt, feedback);
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: feedback_msg,
                    timestamp: now,
                    status: MessageStatus::Completed,
                });

                let mut new_history = history.clone();
                new_history.push(("user".to_string(), feedback.clone()));

                let new_response_index = self.core.messages.len();
                self.core.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: String::new(),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Thinking,
                });

                let new_request_id = self.ui.active_chat_request_id.wrapping_add(1);
                self.ui.active_chat_request_id = new_request_id;
                self.ui.active_chat_requests = 1;
                self.ui.auto_scroll = true;

                let (abort_handle, new_abort_registration) = futures::future::AbortHandle::new_pair();
                self.ui.active_chat_abort = Some(abort_handle);

                self.core.last_chat_request_id = request_id;

                let new_history_clone = new_history.clone();
                self.effects.push(crate::tui::effect::Effect::StartChat {
                    input: feedback.clone(),
                    response_index: new_response_index,
                    request_id: new_request_id,
                    model_id,
                    system_prompt,
                    history: new_history_clone,
                    tool_server,
                    provider,
                    runtime,
                    abort_registration: new_abort_registration,
                });
            }
        }
    }

    /// Trigger a self-check reflection after a completed chat.
    fn trigger_self_check(
        &mut self,
        response_index: usize,
        request_id: u64,
        full_response: String,
        input: &str,
        runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
    ) {
        // Don't reflect on reflection-triggered retries
        if request_id > 0 && request_id == self.core.last_chat_request_id {
            return;
        }

        // Get provider and model_id from the current selection
        let (provider, model_id, system_prompt) = self.get_chat_context(runtime, input);

        let provider = match provider {
            Some(p) => p,
            None => return,
        };

        let tool_server = self.core.tool_server.clone();

        // Build history from messages up to the response
        let mut history: Vec<(String, String)> = Vec::new();
        for (i, msg) in self.core.messages.iter().enumerate() {
            if i >= response_index.saturating_sub(1) {
                break;
            }
            match msg.role {
                MessageRole::User => history.push(("user".to_string(), msg.content.clone())),
                MessageRole::Agent => history.push(("assistant".to_string(), msg.content.clone())),
                _ => {}
            }
        }

        let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
        self.ui.active_chat_abort = Some(abort_handle);

        self.effects.push(crate::tui::effect::Effect::SelfCheck {
            response_index,
            request_id,
            attempt: 0,
            full_response,
            input: input.to_string(),
            system_prompt,
            history,
            tool_server,
            provider,
            model_id,
            runtime: runtime.clone(),
            abort_registration,
        });
    }

    /// Get the chat context (provider, model_id, system_prompt) from current state.
    fn get_chat_context(
        &self,
        _runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
        _input: &str,
    ) -> (Option<std::sync::Arc<crate::llm::LlmProvider>>, String, String) {
        let default_tool_prompt =
            concat!("Must follow user instructions and use available tools. Remember preferences");

        let provider = self
            .core
            .runtime
            .as_ref()
            .and_then(|rt| rt.try_read().ok().and_then(|r| r.provider.clone()));

        let model_id = provider
            .as_ref()
            .and_then(|_| {
                self.core
                    .runtime
                    .as_ref()
                    .and_then(|rt| rt.try_read().ok().map(|r| r.model_id.clone()))
            })
            .unwrap_or_default();

        // Fallback: build from selected_models if runtime provider unavailable
        let provider = provider.or_else(|| {
            self.core.selected_models.first().and_then(|sel| {
                if self.core.configured_providers.iter().any(|id| id == &sel.provider_id) {
                    // Try to get existing client
                    self.core.provider_clients.get(&sel.provider_id).cloned()
                } else {
                    None
                }
            })
        });

        let agent_prompt = self
            .core
            .responsible_agent_id
            .as_ref()
            .and_then(|aid| {
                let pool = self.core.agent_pool.try_read().ok()?;
                let role = pool.get_agent(aid)?.role.clone();
                drop(pool);
                let rt = self.core.runtime.as_ref()?.try_read().ok()?;
                rt.get_role_template(&role).map(|t| t.system_prompt.clone())
            })
            .unwrap_or_else(|| default_tool_prompt.to_string());

        (provider, model_id, agent_prompt)
    }
}

/// Find the slot index of a streaming message.
/// Returns `None` if no streaming/thinking slot is found at the preferred index
/// or elsewhere, to prevent overwriting already-completed messages.
fn find_streaming_slot_response(messages: &[ChatMessage], preferred: usize) -> Option<usize> {
    // 1. Prefer the exact index if it's still in streaming/thinking state
    if messages
        .get(preferred)
        .is_some_and(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    {
        return Some(preferred);
    }
    // 2. Fall back to the last streaming/thinking message anywhere
    messages
        .iter()
        .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    // 3. Return None — don't overwrite completed messages
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            messages: vec![ChatMessage::system(
                "Workflow Agent — connected. Use `/connect` to configure a provider, then `/models` to add models to your pool.",
            )],
            models: ModelRegistry::new(),
            configured_providers: Vec::new(),
            api_keys: HashMap::new(),
            provider_clients: HashMap::new(),
            selected_models: Vec::new(),
            agents: vec![AgentEntry {
                id: "agent-000".to_string(),
                name: "Planning Agent".to_string(),
                status: AgentStatus::Running,
                budget: 0,
            }],
            agent_pool: Arc::new(RwLock::new(crate::agent::AgentPool::new())),
            runtime: None,
            tool_server: crate::tools::create_tool_server(),
            default_role: "general_business_analyst".to_string(),
            responsible_agent_id: None,
            reflection: ReflectionConfig::default(),
            last_chat_request_id: 0,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            panel: Panel::Chat,
            focus: Focus::Input,
            mode: AppMode::Plan,
            input: String::new(),
            input_cursor: 0,
            sidebar_scroll: 0,
            chat_scroll: 0,
            auto_scroll: true,
            think_frame: 0,
            command_popup_selection: 0,
            input_history: Vec::new(),
            input_history_idx: None,
            active_chat_request_id: 0,
            active_chat_abort: None,
            active_chat_requests: 0,
            budget_used: 0,
            budget_total: 10000,
            permits_available: 10,
            permits_total: 10,
            context_limit: 0,
            current_plan: None,
        }
    }
}
