//! Core system state — shared across tools, runtime, and TUI.
//!
//! `CoreState` holds the shared runtime state (agent pool, tool server, etc.)
//! independently of the TUI.  This allows tools to reference the state without
//! depending on the TUI module (breaking the `tools → tui` cycle).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::runtime::AgentRuntime;
use crate::runtime::RuntimeEvent;
use wf_agent::AgentPool;
use wf_core::{AgentId, ChatMessage, SelectedModel};
use wf_models::models::ModelRegistry;
use wf_models::provider::ProviderClient;
use wf_reflection::ReflectionConfig;
use wf_tools::ToolServerHandle;

/// Shared runtime state accessible from tools and TUI.
///
/// This struct intentionally has no TUI dependencies — it can be used by
/// tool code, runtime code, and the TUI without creating circular deps.
pub struct CoreState {
    pub messages: Vec<ChatMessage>,
    pub models: ModelRegistry,
    pub configured_providers: Vec<String>,
    pub api_keys: HashMap<String, String>,
    pub provider_clients: HashMap<String, Arc<ProviderClient>>,
    pub selected_models: Vec<SelectedModel>,
    pub agents: Vec<wf_core::AgentEntry>,
    pub agent_pool: Arc<RwLock<AgentPool>>,
    pub runtime: Option<Arc<RwLock<AgentRuntime>>>,
    pub tool_server: ToolServerHandle,
    pub responsible_agent_id: Option<AgentId>,
    pub default_role: String,
    pub reflection: ReflectionConfig,
    /// Track the last chat context for retry.
    pub last_chat_request_id: u64,
    /// Channel sender to the background `RuntimeEventLoop`.
    /// Tools use this to dispatch async work without blocking the LLM stream.
    pub runtime_event_tx: Option<tokio::sync::mpsc::Sender<RuntimeEvent>>,
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            messages: vec![ChatMessage::system(
                "Workflow Agent — connected. Use /connect to configure a provider, then /models to add models to your pool.",
            )],
            models: wf_models::models::ModelRegistry::new(),
            configured_providers: Vec::new(),
            api_keys: std::collections::HashMap::new(),
            provider_clients: std::collections::HashMap::new(),
            selected_models: Vec::new(),
            agents: vec![wf_core::AgentEntry {
                id: "agent-000".to_string(),
                name: "Planning Agent".to_string(),
                status: wf_core::AgentStatus::Running,
                budget: 0,
            }],
            agent_pool: std::sync::Arc::new(tokio::sync::RwLock::new(wf_agent::AgentPool::new())),
            runtime: None,
            tool_server: wf_tools::create_tool_server(),
            default_role: "general_business_analyst".to_string(),
            responsible_agent_id: None,
            reflection: wf_reflection::ReflectionConfig::default(),
            last_chat_request_id: 0,
            runtime_event_tx: None,
        }
    }
}
