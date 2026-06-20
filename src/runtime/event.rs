//! Runtime event types for the background agent state machine.
//!
//! These events flow from the tool layer and the event loop into the
//! [`RuntimeEventLoop`](super::runtime_loop::RuntimeEventLoop) and onward
//! to the TUI broker.  They form the sole communication channel between
//! the synchronous tool‑call path and the asynchronous agent lifecycle.

use crate::core::types::AgentId;

/// Events emitted by the agent runtime and consumed by
/// [`RuntimeEventLoop`](super::runtime_loop::RuntimeEventLoop).
///
/// Every variant is also forwarded to the TUI broker (via
/// `tui/runtime_bridge.rs`) so the UI can react to state changes.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// Activate a (newly spawned) agent in the background.
    ///
    /// The loop will set its status to `Planning`, execute its LLM
    /// call with tools, then emit `ChildCompleted` or `AgentFailed`.
    ActivateAgent {
        agent_id: AgentId,
        parent_id: Option<AgentId>,
    },

    /// A child agent has reached a terminal state (Completed).
    ///
    /// The event loop uses this to check whether the parent's entire
    /// delegation tree is done, and if so emits `ReadyForAggregation`.
    ChildCompleted {
        parent_id: AgentId,
        child_id: AgentId,
        result: String,
    },

    /// All children of this parent have completed.
    /// The parent should transition to `Aggregating` and a new
    /// LLM synthesis call should be scheduled.
    ReadyForAggregation { agent_id: AgentId },

    /// An agent encountered a fatal error.
    AgentFailed { agent_id: AgentId, error: String },

    /// A parent agent's aggregation synthesis has completed.
    /// The `result` is the final merged output ready for display.
    AggregationCompleted { agent_id: AgentId, result: String },

    /// A message was delivered to an agent's inbox.
    ///
    /// The event loop checks whether the recipient is currently active
    /// and re-activates idle/completed agents so they process the
    /// message promptly (notification mode for online agents).
    InboxMessage {
        /// Recipient agent ID.
        agent_id: AgentId,
        /// Sender's human-readable name.
        from_name: String,
        /// Message preview (first 200 chars).
        preview: String,
        /// Total unread message count in the inbox.
        unread_count: usize,
    },
}
