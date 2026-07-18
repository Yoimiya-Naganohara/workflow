//! Agent workflow tools — LLM-callable tools for inter-agent communication.
//!
//! Agent workflow tools — LLM-callable tools for inter-agent communication.
//!
//! Each tool implements [`rig::tool::Tool`] and can be registered on a
//! [`rig::tool::server::ToolServer`].
pub mod dynamic_tools;
pub use dynamic_tools::DynamicTools;
pub mod list_agents;
pub mod orchestrate;
pub mod send_message;

use workflow_agent::{AgentId, Message};

pub type ToolId = u32;

/// Read-only view of the role registry, used by tools that need to validate
/// or suggest roles without depending on the concrete role crate.
pub trait RoleChecker: Send + Sync {
    fn exists(&self, role: &str) -> bool;
    fn list_roles(&self) -> Vec<String>;
}
// ── Errors ──────────────────────────────────────────────────

/// Errors that can occur during tool execution.
#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    /// The target agent was not found in the pool.
    #[error("agent with id {0} not found in pool")]
    AgentNotFound(AgentId),
    /// The message could not be sent through the agent's channel.
    #[error("failed to send message to agent {receiver}: {source}")]
    SendFailed {
        receiver: AgentId,
        #[source]
        source: tokio::sync::mpsc::error::SendError<Message>,
    },
    /// The requested role does not exist; suggestions are provided.
    #[error("role '{requested}' not found. Did you mean: {suggestions:?}? Available roles: {available:?}. Use the `create_role` tool to create a new role.")]
    RoleNotFound {
        requested: String,
        suggestions: Vec<String>,
        available: Vec<String>,
    },
    /// The sending agent has exhausted its per-turn send_message budget.
    #[error("agent {0} has exhausted its send_message budget for this turn")]
    BudgetExhausted(AgentId),
    /// Orchestration planning error (invalid DAG, cycles, etc.).
    #[error("orchestrate: {0}")]
    Orchestrate(String),
}
