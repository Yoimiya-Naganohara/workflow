//! DispatchDecider — the **single authority** for task execution decisions.
//!
//! # Why this exists
//!
//! Before Phase 2C, the system had three overlapping decision authorities:
//!
//! 1. **TaskGraph** — decided completion semantics via `aggregation_policy`
//! 2. **Scheduler** — ran pipeline, decided spawn/reject/retry
//! 3. **Pipeline/L1/L2** — decided approval via opaque `process_with_text()`
//!
//! This led to **decision authority drift**: a single task spawn could be
//! rejected at the pipeline, retried by the scheduler, or silently ignored.
//!
//! # What this does
//!
//! `DispatchDecider` is the **single entry point** for ALL task execution
//! decisions.  Every task goes through exactly one `decide()` call, which
//! returns one of four outcomes:
//!
//! ```text
//! dispatching_task
//!   └── DispatchDecider::decide()
//!         ├── Approved(config)  → agent created, mark_running
//!         ├── Rejected(reason)  → mark_rejected, terminal
//!         ├── RetryLater        → mark_created, next tick
//!         └── Escalate(target)  → mark_blocked, notify
//! ```
//!
//! The scheduler no longer knows about pipelines, L1, L2, or retry
//! strategies — it only dispatches.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::core::types::{ChildAgentConfig, SpawnDecision, SpawnRejection, TaskId};
use crate::runtime::AgentRuntime;

// ============================================================================
//  DispatchDecision — the single outcome type
// ============================================================================

/// The exclusive set of outcomes from `DispatchDecider::decide()`.
///
/// No other decision path should exist in the system.  If you need a new
/// outcome, add it here, not as a special case in the scheduler.
#[derive(Debug)]
pub enum DispatchDecision {
    /// Approved: create an agent and execute.
    Approved { config: ChildAgentConfig },
    /// Rejected: the task will never run (L1/L2 rejection, budget, etc.).
    Rejected { reason: SpawnRejection },
    /// Transient error: retry on the next dispatch tick.
    RetryLater { reason: String },
    /// The task should be escalated to a different role or a human.
    Escalate { target_role: String, reason: String },
}

// ============================================================================
//  DispatchDecider trait — single decision entry point
// ============================================================================

/// The **single authority** for whether a task should run, wait, or stop.
///
/// # Contract
///
/// - Must be `Send + Sync` (called from the event loop).
/// - Must be deterministic for the same input (no hidden state machine).
/// - Must NOT call back into the `TaskGraph` or `Scheduler` (no reentrancy).
#[async_trait]
pub trait DispatchDecider: Send + Sync {
    /// Decide what to do with a task that is about to be scheduled.
    ///
    /// The task is already in `Dispatching` state (anti-double-dispatch lock).
    /// The scheduler will apply the result without further policy calls.
    async fn decide(&self, task_id: TaskId, goal: &str, role: &str) -> DispatchDecision;
}

// ============================================================================
//  PipelineDispatchDecider — wraps the existing L-1/L0/L1/L2 pipeline
// ============================================================================

/// The standard decider that runs the full decision pipeline
/// (L-1 admission → L0 circuit breaker → L1 experience → L2 audit).
///
/// This is what Phase 2A/B used as inline code in the scheduler.  Extracted
/// here so the scheduler no longer knows about pipeline internals.
///
/// # Future
///
/// - Phase 3: `ComplexityDispatchDecider` adds complexity estimation
/// - Phase 4: `LearningDispatchDecider` incorporates experience feedback
/// - Always wrapped, never modified in-place
pub struct PipelineDispatchDecider {
    runtime: Arc<RwLock<AgentRuntime>>,
}

impl PipelineDispatchDecider {
    pub fn new(runtime: Arc<RwLock<AgentRuntime>>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl DispatchDecider for PipelineDispatchDecider {
    async fn decide(&self, _task_id: TaskId, goal: &str, role: &str) -> DispatchDecision {
        let rt = self.runtime.read().await;
        match rt
            .process_with_text(goal, role, "default", 1000, 0, None, None)
            .await
        {
            Ok(SpawnDecision::Approved(config)) => DispatchDecision::Approved { config },
            Ok(SpawnDecision::Rejected(reason)) => DispatchDecision::Rejected { reason },
            Err(e) => DispatchDecision::RetryLater {
                reason: format!("Pipeline error: {}", e),
            },
        }
    }
}
