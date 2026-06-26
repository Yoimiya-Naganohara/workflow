//! Core system state — shared across tools, runtime, and TUI.
//!
//! `CoreState` holds the shared runtime state (agent pool, tool server, etc.)
//! independently of the TUI.  This allows tools to reference the state without
//! depending on the TUI module (breaking the `tools → tui` cycle).

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::tools::ToolServerHandle;

/// Shared runtime state accessible from tools and TUI.
///
/// This struct intentionally has no TUI dependencies — it can be used by
/// tool code, runtime code, and the TUI without creating circular deps.
pub struct CoreState {
    pub messages: Vec<super::types::ChatMessage>,
    pub agent_pool: Arc<RwLock<crate::agent::AgentPool>>,
    pub runtime: Option<Arc<RwLock<crate::runtime::AgentRuntime>>>,
    pub tool_server: ToolServerHandle,
    pub responsible_agent_id: Option<AgentId>,
    pub runtime_event_tx:
        Option<tokio::sync::mpsc::Sender<crate::runtime::RuntimeEvent>>,
}
