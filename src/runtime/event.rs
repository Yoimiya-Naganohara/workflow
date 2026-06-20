//! Runtime event types for the background agent state machine.
//!
//! These events flow from the tool layer and the event loop into the
//! [`RuntimeEventLoop`](super::runtime_loop::RuntimeEventLoop) and onward
//! to the TUI broker.  They form the sole communication channel between
//! the synchronous tool‑call path and the asynchronous agent lifecycle.
//!
//! # Event taxonomy
//!
//! Events fall into two streams:
//!
//! 1. **Execution events** — normal tool use, agent lifecycle
//! 2. **Delegation events** — task graph mutation (Phase 2+)
//!
//! See [`ARCHTECHTURE.md`](../../ARCHTECHTURE.md) for the full design.

use crate::core::types::AgentId;
use crate::core::types::TaskId;

/// Events emitted by the agent runtime and consumed by
/// [`RuntimeEventLoop`](super::runtime_loop::RuntimeEventLoop).
///
/// Every variant is also forwarded to the TUI broker (via
/// `tui/runtime_bridge.rs`) so the UI can react to state changes.
#[derive(Debug, Clone)]
pub enum RuntimeEvent {
    // ── Execution events ──
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

    // ════════════════════════════════════════════════════════════════
    //  Delegation events (Phase 2+)
    //  These mutate the TaskGraph and are the "write capability"
    //  that agents use to spawn, escalate, and merge work.
    // ════════════════════════════════════════════════════════════════
    /// An agent requests spawning a new sub-task in the task graph.
    ///
    /// The parent agent is identified by `parent_agent`.  The event loop
    /// (see [`RuntimeEventLoop`](super::runtime_loop::RuntimeEventLoop))
    /// creates the task node via `TaskGraph::spawn_child`, updating the
    /// parent agent's `task_id` if needed.
    ///
    /// After inserting the node, the loop calls `schedule_ready_tasks()`
    /// which runs the pipeline for each ready task and may emit
    /// `ActivateAgent` events.
    SpawnTask {
        /// The goal/purpose of the new task.
        goal: String,
        /// The role that should execute this task.
        role: String,
        /// The agent requesting the spawn (becomes the task's owner).
        parent_agent: AgentId,
    },

    /// A task in the graph has completed successfully.
    ///
    /// The runtime checks the DAG for newly-runnable downstream tasks
    /// and potentially triggers parent aggregation.
    TaskCompleted { task_id: TaskId, result: String },

    /// A task has failed.
    ///
    /// The runtime marks it in the graph and checks whether any
    /// downstream tasks should be blocked or escalated.
    TaskFailed { task_id: TaskId, error: String },

    /// An agent escalates a task to a different role or to a human.
    ///
    /// This is how agents signal "I cannot handle this — reassign".
    EscalateTask {
        task_id: TaskId,
        reason: String,
        /// The role that should handle this task next.
        target_role: Option<String>,
        /// The agent requesting the escalation.
        from_agent: AgentId,
    },

    /// Merge the result of one task into another (fan-in).
    ///
    /// Used when a decomposed task's children produce results that
    /// should be aggregated into the parent's output.
    MergeTaskResult {
        /// Source task whose result is being merged.
        from_task: TaskId,
        /// Destination task receiving the merged result.
        into_task: TaskId,
        /// Optional summary/synthesis text.
        summary: Option<String>,
    },
}

impl RuntimeEvent {
    /// Human-readable label for logging / TUI.
    pub fn label(&self) -> &'static str {
        match self {
            // Execution
            RuntimeEvent::ActivateAgent { .. } => "activate-agent",
            RuntimeEvent::ChildCompleted { .. } => "child-completed",
            RuntimeEvent::ReadyForAggregation { .. } => "ready-for-aggregation",
            RuntimeEvent::AgentFailed { .. } => "agent-failed",
            RuntimeEvent::AggregationCompleted { .. } => "aggregation-completed",
            RuntimeEvent::InboxMessage { .. } => "inbox-message",
            // Delegation
            RuntimeEvent::SpawnTask { .. } => "spawn-task",
            RuntimeEvent::TaskCompleted { .. } => "task-completed",
            RuntimeEvent::TaskFailed { .. } => "task-failed",
            RuntimeEvent::EscalateTask { .. } => "escalate-task",
            RuntimeEvent::MergeTaskResult { .. } => "merge-task-result",
        }
    }
}
