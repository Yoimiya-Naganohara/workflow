//! Task Scheduler — the **pure dispatcher** between TaskGraph and agents.
//!
//! # Responsibility
//!
//! The scheduler does exactly one thing: turn a `Dispatching` task into a
//! running agent or a decomposed set of subtasks.  It does NOT know about:
//!
//! - Pipelines (L-1/L0/L1/L2)
//! - Retry strategies
//! - Escalation logic
//! - Decomposition internals
//!
//! All of those belong in their respective trait implementations
//! ([`DispatchDecider`](super::dispatch::DispatchDecider),
//! [`DecompositionEngine`](super::decomposition::DecompositionEngine)).
//!
//! # Dispatch loop (freeze contract)
//!
//! ```text
//! dispatch()
//!   ├── graph.ready_tasks()             ← query
//!   ├── for each task:
//!   │     ├── mark_dispatching()          ← lock
//!   │     ├── engine.should_decompose()?  ← split check (Phase 3)
//!   │     │     └── engine.decompose()    ← graph mutation
//!   │     ├── decider.decide(task)        ← exec decision
//!   │     └── apply decision
//! ```
//!
//! The scheduler is called after every graph mutation (`SpawnTask`,
//! `TaskCompleted`, `TaskFailed`) from the runtime event loop.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use crate::agent::{Agent, AgentConfig, AgentPool, AgentStatus};
use crate::core::types::AgentId;
use crate::runtime::AgentRuntime;
use crate::runtime::decomposition::DecompositionEngine;
use crate::runtime::dispatch::{DispatchDecider, DispatchDecision};
use crate::runtime::event::RuntimeEvent;

/// Pure dispatcher — no pipeline logic, no retry policy.
/// Optionally includes a `DecompositionEngine` for task splitting (Phase 3).
pub struct TaskScheduler {
    runtime: Arc<RwLock<AgentRuntime>>,
    pool: Arc<RwLock<AgentPool>>,
    broker_tx: mpsc::Sender<RuntimeEvent>,
    /// The single decision authority for task execution.
    decider: Box<dyn DispatchDecider>,
    /// Optional decomposition engine (Phase 3).
    /// When `Some`, tasks that exceed structural tension thresholds
    /// are split before reaching the decider.
    decomposition: Option<Box<dyn DecompositionEngine>>,
}

impl TaskScheduler {
    pub fn new(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        decider: Box<dyn DispatchDecider>,
    ) -> Self {
        Self {
            runtime,
            pool,
            broker_tx,
            decider,
            decomposition: None,
        }
    }

    /// Attach a decomposition engine (Phase 3 entry point).
    pub fn with_decomposition(mut self, engine: Box<dyn DecompositionEngine>) -> Self {
        self.decomposition = Some(engine);
        self
    }

    /// Run one dispatch tick.
    ///
    /// Called after every graph mutation.  For each ready task:
    ///
    /// 1. `mark_dispatching()` — anti-double-dispatch lock
    /// 2. `engine.should_decompose()?` — optional split check (Phase 3)
    /// 3. `decider.decide()` — single decision call
    /// 4. Apply decision
    pub async fn dispatch(&self) {
        // ── Phase 1: Query ready tasks from the graph ──
        let ready: Vec<(AgentId, String, Option<String>)> = {
            let rt = self.runtime.read().await;
            let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            g.ready_tasks()
                .iter()
                .filter_map(|tid| {
                    let node = g.get(tid)?;
                    Some((*tid, node.goal.clone(), node.role.clone()))
                })
                .collect()
        };

        if ready.is_empty() {
            return;
        }

        tracing::info!("TaskScheduler::dispatch: {} ready task(s)", ready.len());

        for (task_id, task_goal, task_role) in ready {
            let role = task_role.clone().unwrap_or_else(|| "worker".to_string());

            // ── Phase 2: Anti-double-dispatch lock ──
            {
                let rt = self.runtime.read().await;
                let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = g.mark_dispatching(task_id) {
                    tracing::warn!(
                        "scheduler: mark_dispatching({:02x}..) failed: {} — skipping",
                        task_id[0],
                        e
                    );
                    continue;
                }
            }

            // ── Phase 2.5: Check for decomposition (Phase 3 entry) ──
            // The scheduler does NOT know how decomposition works — it
            // just asks the engine.  If the engine says "decompose",
            // it mutates the graph and we skip this task on this tick.
            // The subtasks will be picked up by the next dispatch call.
            if let Some(ref engine) = self.decomposition {
                let should_split = {
                    let rt = self.runtime.read().await;
                    let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    engine.should_decompose(task_id, &g)
                };
                if should_split {
                    let rt = self.runtime.read().await;
                    let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    let children = engine.decompose(task_id, &mut g);
                    if !children.is_empty() {
                        tracing::info!(
                            "scheduler: task {:02x}.. decomposed into {} subtask(s)",
                            task_id[0],
                            children.len()
                        );
                    }
                    continue; // subtasks picked up by next dispatch
                }
            }

            // ── Phase 3: Single decision call — no pipeline knowledge ──
            let decision = self.decider.decide(task_id, &task_goal, &role).await;

            // ── Phase 4: Apply decision ──
            match decision {
                DispatchDecision::Approved { config } => {
                    self.apply_approved(task_id, &task_goal, &role, config)
                        .await;
                }
                DispatchDecision::Rejected { reason } => {
                    tracing::warn!(
                        "scheduler: task {:02x}.. rejected: {:?}",
                        task_id[0],
                        reason
                    );
                    let rt = self.runtime.read().await;
                    let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    g.mark_rejected(task_id, &format!("{}", reason)).ok();
                }
                DispatchDecision::RetryLater { reason } => {
                    tracing::info!(
                        "scheduler: task {:02x}.. will retry: {}",
                        task_id[0],
                        reason
                    );
                    let rt = self.runtime.read().await;
                    let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    if g.mark_created(task_id).is_err() {
                        g.mark_rejected(task_id, &format!("RetryLater fallback: {}", reason))
                            .ok();
                    }
                }
                DispatchDecision::Escalate {
                    target_role,
                    reason,
                } => {
                    tracing::warn!(
                        "scheduler: task {:02x}.. escalated to {}: {}",
                        task_id[0],
                        target_role,
                        reason
                    );
                    let rt = self.runtime.read().await;
                    let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(node) = g.get_mut(&task_id) {
                        node.metadata
                            .insert("escalated_to".into(), target_role.clone());
                        node.metadata.insert("escalated_reason".into(), reason);
                    }
                    g.mark_blocked(task_id).ok();
                }
            }
        }
    }

    // ── Decision application helpers ──

    /// Create an agent and mark the task Running.
    async fn apply_approved(
        &self,
        task_id: AgentId,
        task_goal: &str,
        role: &str,
        config: crate::core::types::ChildAgentConfig,
    ) {
        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();

        let agent = Agent {
            id: agent_id,
            name: format!(
                "{}-{:04x}",
                role,
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            role: role.to_string(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: task_goal.to_string(),
            config: AgentConfig {
                model_id: String::new(),
                allowed_tools: config.allowed_tools,
                ..Default::default()
            },
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: crate::agent::now_secs(),
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            task_id: Some(task_id),
            sandbox,
        };

        // Transition Dispatching → Running.
        {
            let rt = self.runtime.read().await;
            let mut g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(e) = g.mark_running(task_id, agent_id) {
                tracing::warn!(
                    "scheduler: mark_running({:02x}..) failed: {}",
                    task_id[0],
                    e
                );
            }
        }

        // Add to pool.
        {
            let mut p = self.pool.write().await;
            p.add_agent(agent);
            let guard = {
                let rt = self.runtime.read().await;
                rt.take_pending_guard()
            };
            if let Some(g) = guard {
                p.attach_budget_guard(agent_id, g);
            }
        }

        // Activate.
        let _ = self
            .broker_tx
            .send(RuntimeEvent::ActivateAgent {
                agent_id,
                parent_id: None,
            })
            .await;
    }
}
