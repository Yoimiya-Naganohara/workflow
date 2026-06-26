//! Core system state — shared across tools, runtime, and TUI.
//!
//! `CoreState` holds the shared runtime state (agent pool, tool server, etc.)
//! independently of the TUI.  This allows tools to reference the state without
//! depending on the TUI module (breaking the `tools → tui` cycle).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::agent::AgentPool;
use crate::core::types::{AgentId, ChatMessage, SelectedModel};
use crate::models::ModelRegistry;
use crate::provider::ProviderClient;
use crate::reflection::ReflectionConfig;
use crate::runtime::AgentRuntime;
use crate::runtime::RuntimeEvent;
use crate::tools::ToolServerHandle;

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
    pub agents: Vec<crate::tui::state::AgentEntry>,
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
