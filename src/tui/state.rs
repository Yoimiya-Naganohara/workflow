use std::collections::HashMap;
use std::sync::Arc;

use futures::future::AbortHandle;
use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::models::ModelRegistry;
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
                request_id: _,
                full_response,
                input: _,
                runtime: _,
            } => {
                // Prepend any tool call annotations that were streamed during the response
                if let Some(slot) = find_streaming_slot_response(&self.core.messages, response_index) {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        // Preserve tool call annotations that were appended during streaming
                        let tool_annotations = if !full_response.is_empty() && msg.content != full_response {
                            // Extract any text that was added after full_response content
                            if msg.content.starts_with(&full_response) {
                                msg.content[full_response.len()..].to_string()
                            } else {
                                // Content diverged — keep what we have and annotate
                                String::new()
                            }
                        } else {
                            String::new()
                        };

                        if !full_response.is_empty() {
                            msg.content = full_response;
                        }
                        if !tool_annotations.is_empty() {
                            msg.content.push_str(&tool_annotations);
                        }
                        msg.status = MessageStatus::Completed;
                    }
                }
                if self.ui.active_chat_requests > 0 {
                    self.ui.active_chat_requests -= 1;
                }
                self.ui.active_chat_abort = None;
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
                // Insert tool call right after the streaming agent message
                let insert_pos = find_streaming_slot_response(&self.core.messages, response_index)
                    .map(|s| s + 1)
                    .unwrap_or_else(|| self.core.messages.len());
                self.core.messages.insert(
                    insert_pos,
                    ChatMessage {
                        role: MessageRole::Decision,
                        content: format!("{} — {}", name, args),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    },
                );
            }
        }
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
