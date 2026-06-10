use std::collections::HashMap;
use std::sync::Arc;

use futures::future::AbortHandle;
use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::models::ModelRegistry;
use crate::runtime::AgentRuntime;
use crate::tools::ToolServerHandle;

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

pub struct AppState {
    pub panel: Panel,
    pub focus: Focus,
    pub mode: AppMode,
    pub agents: Vec<AgentEntry>,
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_cursor: usize,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub budget_used: u64,
    pub budget_total: u64,
    pub permits_available: usize,
    pub permits_total: usize,
    pub models: ModelRegistry,
    pub configured_providers: Vec<String>,
    pub show_provider_dialog: bool,
    pub selected_provider_idx: usize,
    pub provider_search_query: String,
    pub provider_search_cursor: usize,
    pub show_key_dialog: bool,
    pub key_input: String,
    pub key_cursor: usize,
    pub key_provider_id: Option<String>,
    pub show_model_picker: bool,
    pub selected_model_picker_idx: usize,
    pub model_picker_search_query: String,
    pub model_picker_search_cursor: usize,
    pub return_to_model_picker: bool,
    pub selected_models: Vec<SelectedModel>,
    pub current_plan: Option<crate::agent::plan::Plan>,
    pub agent_pool: Arc<RwLock<crate::agent::AgentPool>>,
    pub provider_clients: HashMap<String, Arc<crate::llm::LlmProvider>>,
    pub active_chat_request_id: u64,
    pub active_chat_abort: Option<AbortHandle>,
    pub active_chat_requests: usize,
    pub command_popup_selection: usize,
    pub api_keys: HashMap<String, String>,
    pub input_history: Vec<String>,
    pub input_history_idx: Option<usize>,
    pub show_status_panel: bool,
    pub runtime: Option<Arc<RwLock<AgentRuntime>>>,
    pub keymap: super::keymap::Keymap,
    pub pool_stats: ExperiencePoolStats,
    /// Thinking animation counter (0..3) — cycles every render tick.
    pub think_frame: u8,
    /// Whether chat is auto-scrolled to bottom.
    pub auto_scroll: bool,
    /// MCP tool server handle with built-in tools.
    pub tool_server: ToolServerHandle,
    /// Agent ID of the root agent that owns the conversation.
    /// Set automatically on first chat; `spawn_agent` tool spawns children under this agent.
    pub responsible_agent_id: Option<AgentId>,
    /// Context window limit (from selected model), 0 if unknown.
    pub context_limit: u64,
    /// Custom provider wizard dialog
    pub show_custom_dialog: bool,
    pub custom_step: usize,
    pub custom_name: String,
    pub custom_url: String,
    pub custom_key: String,
    pub custom_models: String,
    pub custom_input: String,
    pub custom_cursor: usize,
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

impl Default for AppState {
    fn default() -> Self {
        Self {
            panel: Panel::Chat,
            focus: Focus::Input,
            mode: AppMode::Plan,
            agents: vec![AgentEntry {
                id: "agent-000".to_string(),
                name: "Planning Agent".to_string(),
                status: AgentStatus::Running,
                budget: 0,
            }],
            messages: vec![
                ChatMessage {
                    role: MessageRole::System,
                    content: "Workflow Agent — connected. Use `/connect` to configure a provider, then `/models` to add models to your pool.".to_string(),
                    timestamp: "00:00:00".to_string(),
                    status: MessageStatus::Completed,
                },
            ],
            input: String::new(),
            input_cursor: 0,
            sidebar_scroll: 0,
            chat_scroll: 0,
            budget_used: 0,
            budget_total: 10000,
            permits_available: 10,
            permits_total: 10,
            models: ModelRegistry::new(),
            configured_providers: Vec::new(),
            show_provider_dialog: false,
            selected_provider_idx: 0,
            provider_search_query: String::new(),
            provider_search_cursor: 0,
            show_key_dialog: false,
            key_input: String::new(),
            key_cursor: 0,
            key_provider_id: None,
            show_model_picker: false,
            selected_model_picker_idx: 0,
            model_picker_search_query: String::new(),
            model_picker_search_cursor: 0,
            return_to_model_picker: false,
            selected_models: Vec::new(),
            current_plan: None,
            agent_pool: Arc::new(RwLock::new(crate::agent::AgentPool::new())),
            provider_clients: HashMap::new(),
            active_chat_request_id: 0,
            active_chat_abort: None,
            active_chat_requests: 0,
            command_popup_selection: 0,
            api_keys: HashMap::new(),
            input_history: Vec::new(),
            input_history_idx: None,
            show_status_panel: true,
            runtime: None,
            keymap: super::keymap::Keymap::default(),
            pool_stats: ExperiencePoolStats::default(),
            think_frame: 0,
            auto_scroll: true,
            tool_server: crate::tools::create_tool_server(),
            responsible_agent_id: None,
            context_limit: 0,
            show_custom_dialog: false,
            custom_step: 0,
            custom_name: String::new(),
            custom_url: String::new(),
            custom_key: String::new(),
            custom_models: String::new(),
            custom_input: String::new(),
            custom_cursor: 0,
        }
    }
}

pub const COMMANDS: &[(&str, &str)] = &[
    ("/connect", "Configure a provider (API key + custom)"),
    ("/models", "Manage model pool (add/remove models)"),
    ("/clear", "Clear conversation"),
    ("/sh", "Run a shell command"),
    ("/pool", "Manage experience pool (flush/clear/stats/export/import)"),
    ("/keymap", "Show keyboard shortcuts"),
    ("/custom", "Add/list/remove custom providers"),
    ("/help", "Show help"),
];
