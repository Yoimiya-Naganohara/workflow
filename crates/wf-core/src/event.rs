//! Runtime event types — shared between tools, runtime, and TUI.
//!
//! These events flow from the tool layer and the event loop into the
//! runtime event loop and onward to the TUI broker. Moved here from
//! wf-runtime to break the circular dependency between wf-tools ↔ wf-runtime.

use crate::{AgentId, SubtaskDef, TaskId};

/// Events emitted by the agent runtime and consumed by the runtime event loop.
///
/// Every variant is also forwarded to the TUI broker so the UI can react
/// to state changes.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    /// Activate a (newly spawned) agent in the background.
    ActivateAgent {
        agent_id: AgentId,
        parent_id: Option<AgentId>,
    },

    /// A child agent has reached a terminal state (Completed).
    ChildCompleted {
        parent_id: AgentId,
        child_id: AgentId,
        result: String,
    },

    /// All children of this parent have completed.
    ReadyForAggregation { agent_id: AgentId },

    /// An agent encountered a fatal error.
    AgentFailed { agent_id: AgentId, error: String },

    /// A parent agent's aggregation synthesis has completed.
    AggregationCompleted { agent_id: AgentId, result: String },

    /// A message was delivered to an agent's inbox.
    InboxMessage {
        agent_id: AgentId,
        from_name: String,
        preview: String,
        unread_count: usize,
    },

    /// A task in the graph has completed successfully.
    TaskCompleted { task_id: TaskId, result: String },

    /// A task has failed.
    TaskFailed { task_id: TaskId, error: String },

    /// An agent escalates a task to a different role or to a human.
    EscalateTask {
        task_id: TaskId,
        reason: String,
        target_role: Option<String>,
        from_agent: AgentId,
    },

    /// Merge the result of one task into another (fan-in).
    MergeTaskResult {
        from_task: TaskId,
        into_task: TaskId,
        summary: Option<String>,
    },

    /// An agent decomposes its own task into subtasks.
    DecomposeTask {
        parent_agent: AgentId,
        subtasks: Vec<SubtaskDef>,
    },
}

impl RuntimeEvent {
    /// Human-readable label for logging / TUI.
    pub fn label(&self) -> &'static str {
        match self {
            Self::ActivateAgent { .. } => "activate-agent",
            Self::ChildCompleted { .. } => "child-completed",
            Self::ReadyForAggregation { .. } => "ready-for-aggregation",
            Self::AgentFailed { .. } => "agent-failed",
            Self::AggregationCompleted { .. } => "aggregation-completed",
            Self::InboxMessage { .. } => "inbox-message",
            Self::TaskCompleted { .. } => "task-completed",
            Self::TaskFailed { .. } => "task-failed",
            Self::EscalateTask { .. } => "escalate-task",
            Self::MergeTaskResult { .. } => "merge-task-result",
            Self::DecomposeTask { .. } => "decompose-task",
        }
    }
}
