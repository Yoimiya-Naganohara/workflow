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
    pub show_status_panel: bool,
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
    pub active_dialog: Option<crate::tui::dialogs::ActiveDialog>,
    pub keymap: super::keymap::Keymap,
    pub pool_stats: ExperiencePoolStats,
    /// Effects queued for async execution (drained by event loop).
    pub effects: Vec<crate::tui::effect::Effect>,
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
                let slot = find_streaming_slot_response(&self.core.messages, response_index);
                if let Some(msg) = self.core.messages.get_mut(slot) {
                    msg.content.push_str(&text);
                    msg.status = MessageStatus::Streaming;
                }
            }
            AppEvent::ChatToolCall {
                response_index,
                name,
                args,
                timestamp,
            } => {
                let tool_msg = format!("🔧 {} — {}", name, args);
                self.core.messages.push(ChatMessage {
                    role: MessageRole::Decision,
                    content: tool_msg,
                    timestamp,
                    status: MessageStatus::Completed,
                });
                // Shift future response indices — we inserted a message
                if response_index >= self.core.messages.len() {
                    // message was pushed, not inserted before response_index
                }
            }
            AppEvent::ChatCompleted {
                response_index,
                full_response,
                input: _,
                runtime: _,
            } => {
                let slot = find_streaming_slot_response(&self.core.messages, response_index);
                if let Some(msg) = self.core.messages.get_mut(slot) {
                    if !full_response.is_empty() {
                        msg.content = full_response;
                    }
                    msg.status = MessageStatus::Completed;
                }
                if self.ui.active_chat_requests > 0 {
                    self.ui.active_chat_requests -= 1;
                }
                self.ui.active_chat_abort = None;
            }
            AppEvent::ChatError { response_index, error } => {
                let slot = find_streaming_slot_response(&self.core.messages, response_index);
                if let Some(msg) = self.core.messages.get_mut(slot) {
                    msg.content = error;
                    msg.status = MessageStatus::Error;
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
            }
            AppEvent::ChatCancelled { response_index } => {
                let slot = find_streaming_slot_response(&self.core.messages, response_index);
                if let Some(msg) = self.core.messages.get_mut(slot) {
                    msg.content += " (cancelled)";
                    msg.status = MessageStatus::Completed;
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
            }
        }
    }
}

/// Find the slot index of a streaming message (fallback to last thinking/streaming).
fn find_streaming_slot_response(messages: &[ChatMessage], preferred: usize) -> usize {
    messages
        .get(preferred)
        .filter(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
        .map(|_| preferred)
        .unwrap_or_else(|| {
            messages
                .iter()
                .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
                .unwrap_or(preferred)
        })
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
            show_status_panel: true,
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
