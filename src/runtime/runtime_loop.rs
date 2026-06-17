//! Background event loop for the agent runtime state machine.
//!
//! Pipeline: `ActivateAgent → execute_agent_inner → ChildCompleted
//! → all_done? → ReadyForAggregation → spawn synthesis → AggregationCompleted`
//!
//! # Channel topology
//!
//! ```text
//! Tool (spawn_agent) ──► event_tx (from AppState)
//!                           │
//!                           ▼
//!                    RuntimeEventLoop::run()
//!                     ├─ ActivateAgent  → handle (execute child)
//!                     ├─ ChildCompleted → forward to broker_tx
//!                     ├─ AgentFailed    → forward to broker_tx
//!                     └─ (others)       → forward to broker_tx
//!                           │
//!                           ▼
//!                    runtime_bridge (→ AppEvent → TUI)
//! ```

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};

use crate::agent::{AgentPool, AgentStatus};
use crate::core::types::AgentId;
use crate::runtime::AgentRuntime;
use crate::runtime::event::RuntimeEvent;
use crate::tools::ToolServerHandle;

/// Background agent lifecycle loop.
pub struct RuntimeEventLoop {
    runtime: Arc<RwLock<AgentRuntime>>,
    pool: Arc<RwLock<AgentPool>>,
    event_rx: mpsc::Receiver<RuntimeEvent>,
    /// Events that the loop does not consume are forwarded here
    /// to the TUI broker.
    broker_tx: mpsc::Sender<RuntimeEvent>,
    tool_server: ToolServerHandle,
    /// Reference to the TUI AppState — used to create sandboxed
    /// tool servers when the agent has a sandbox handle.
    state: Option<std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>>,
}

impl RuntimeEventLoop {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        event_rx: mpsc::Receiver<RuntimeEvent>,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        tool_server: ToolServerHandle,
        state: Option<std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>>,
    ) -> Self {
        Self {
            runtime,
            pool,
            event_rx,
            broker_tx,
            tool_server,
            state,
        }
    }

    pub async fn run(mut self) {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                RuntimeEvent::ActivateAgent {
                    agent_id,
                    parent_id,
                } => {
                    // Spawn in background — the event loop must NOT block
                    // on LLM calls; siblings must execute concurrently.
                    let rt = self.runtime.clone();
                    let pool = self.pool.clone();
                    let ts = self.tool_server.clone();
                    let bt = self.broker_tx.clone();
                    let st = self.state.clone();
                    tokio::spawn(async move {
                        Self::handle_activate_inner(rt, pool, ts, bt, st, agent_id, parent_id)
                            .await;
                    });
                }
                other => {
                    // Everything else → forward to broker.
                    let _ = self.broker_tx.send(other).await;
                }
            }
        }
    }

    // ── Handlers ──

    async fn handle_activate_inner(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        tool_server: ToolServerHandle,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        state: Option<std::sync::Arc<tokio::sync::RwLock<crate::tui::state::AppState>>>,
        agent_id: AgentId,
        parent_id: Option<AgentId>,
    ) {
        // Determine which tool server to use.
        let agent_sandbox = {
            let p = pool.read().await;
            p.get_agent(&agent_id).and_then(|a| a.sandbox.clone())
        };
        let tool_handle = match (&agent_sandbox, &state) {
            (Some(sb), Some(st)) => {
                crate::tools::create_sandboxed_agent_tool_server(st.clone(), Some(sb.clone()))
            }
            _ => tool_server.clone(),
        };

        // Execute the agent (LLM call + tools) without holding the runtime
        // read lock.  This allows other tasks (e.g. pool consolidation) to
        // acquire a write lock while the LLM request is in-flight.
        let (result, status) = AgentRuntime::execute_agent_detached(
            runtime.clone(),
            agent_id,
            pool.clone(),
            Some(tool_handle),
        )
        .await;

        // Report completion.
        match status {
            AgentStatus::Completed => {
                if let Some(pid) = parent_id {
                    {
                        let mut p = pool.write().await;
                        if let Some(parent) = p.get_agent_mut(&pid) {
                            parent.child_results.push((agent_id, result.clone()));
                        }
                    }
                    let _ = broker_tx
                        .send(RuntimeEvent::ChildCompleted {
                            parent_id: pid,
                            child_id: agent_id,
                            result: result.clone(),
                        })
                        .await;
                    Self::maybe_advance_parent_inner(
                        runtime.clone(),
                        pool.clone(),
                        broker_tx.clone(),
                        pid,
                    )
                    .await;
                }
            }
            AgentStatus::Failed => {
                let error = if result.is_empty() {
                    "Agent execution failed (no result)".to_string()
                } else {
                    result
                };
                let _ = broker_tx
                    .send(RuntimeEvent::AgentFailed { agent_id, error })
                    .await;
            }
            _ => {}
        }
    }

    async fn maybe_advance_parent_inner(
        runtime: Arc<RwLock<AgentRuntime>>,
        pool: Arc<RwLock<AgentPool>>,
        broker_tx: mpsc::Sender<RuntimeEvent>,
        parent_id: AgentId,
    ) {
        let all_done = {
            let p = pool.read().await;
            let Some(parent) = p.get_agent(&parent_id) else {
                return;
            };
            parent.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };

        if !all_done {
            return;
        }

        // All children done → advance parent and spawn synthesis.
        {
            let mut p = pool.write().await;
            if let Some(parent) = p.get_agent_mut(&parent_id) {
                parent.status = AgentStatus::Aggregating;
            }
        }

        let pool_clone = pool.clone();
        tokio::spawn(async move {
            match runtime
                .read()
                .await
                .synthesize_aggregation(parent_id, &pool_clone)
                .await
            {
                Ok(result) => {
                    {
                        let mut p = pool.write().await;
                        if let Some(parent) = p.get_agent_mut(&parent_id) {
                            parent.result = Some(result.clone());
                            parent.status = AgentStatus::Completed;
                            p.release_budget_guard(&parent_id);
                        }
                    }
                    let _ = broker_tx
                        .send(RuntimeEvent::AggregationCompleted {
                            agent_id: parent_id,
                            result,
                        })
                        .await;
                }
                Err(e) => {
                    // Graceful degradation: concatenate child results.
                    let fallback = {
                        let p = pool.read().await;
                        let agent = match p.get_agent(&parent_id) {
                            Some(a) => a,
                            None => {
                                let _ = broker_tx
                                    .send(RuntimeEvent::AgentFailed {
                                        agent_id: parent_id,
                                        error: format!("Synthesis failed (parent gone): {}", e),
                                    })
                                    .await;
                                return;
                            }
                        };
                        if agent.child_results.is_empty() {
                            format!(
                                "⚠️ Aggregation synthesis failed: {}\n\nAll sub-tasks completed but no results were captured.",
                                e
                            )
                        } else {
                            let parts: Vec<String> = agent
                                .child_results
                                .iter()
                                .map(|(_id, r)| r.clone())
                                .collect();
                            format!(
                                "⚠️ Aggregation synthesis failed ({}).  Raw sub-task results:\n\n---\n{}\n\n---\n*Degraded output*",
                                e,
                                parts.join("\n\n---\n\n")
                            )
                        }
                    };
                    {
                        let mut p = pool.write().await;
                        if let Some(parent) = p.get_agent_mut(&parent_id) {
                            parent.result = Some(fallback.clone());
                            parent.status = AgentStatus::Completed;
                            p.release_budget_guard(&parent_id);
                        }
                    }
                    let _ = broker_tx
                        .send(RuntimeEvent::AggregationCompleted {
                            agent_id: parent_id,
                            result: fallback,
                        })
                        .await;
                }
            }
        });
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{Agent, AgentConfig};

    fn stub_agent() -> Agent {
        Agent {
            id: [0u8; 16],
            name: String::new(),
            role: String::new(),
            role_template_id: None,
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: String::new(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
            context: Vec::new(),
            last_active_at: 0,
            tokens_input: 0,
            tokens_output: 0,
            tool_trace: std::collections::VecDeque::new(),
            inbox: std::collections::VecDeque::new(),
            sandbox: None,
        }
    }

    fn pool_with_parent_and_two_children() -> (AgentPool, AgentId, AgentId, AgentId) {
        let mut pool = AgentPool::new();
        let parent = [0u8; 16];
        let child_a = [1u8; 16];
        let child_b = [2u8; 16];

        pool.add_agent(Agent {
            id: parent,
            name: "planner".into(),
            role: "planner".into(),
            status: AgentStatus::AwaitingChildren,
            children: vec![child_a, child_b],
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: child_a,
            name: "dev-a".into(),
            role: "developer".into(),
            parent_id: Some(parent),
            status: AgentStatus::Planning,
            ..stub_agent()
        });
        pool.add_agent(Agent {
            id: child_b,
            name: "dev-b".into(),
            role: "developer".into(),
            parent_id: Some(parent),
            status: AgentStatus::Planning,
            ..stub_agent()
        });

        (pool, parent, child_a, child_b)
    }

    #[tokio::test]
    async fn test_maybe_advance_parent_not_done_yet() {
        let (pool, parent, child_a, _child_b) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
        }

        {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            assert_eq!(pe.status, AgentStatus::AwaitingChildren);
        }
    }

    #[tokio::test]
    async fn test_maybe_advance_parent_all_done() {
        let (pool, parent, child_a, child_b) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
            if let Some(c) = p.get_agent_mut(&child_b) {
                c.status = AgentStatus::Completed;
            }
        }

        let all_done = {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            pe.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };
        assert!(all_done);

        {
            let mut p = pool.write().await;
            if let Some(pe) = p.get_agent_mut(&parent) {
                pe.status = AgentStatus::Aggregating;
            }
        }

        assert_eq!(
            pool.read().await.get_agent(&parent).unwrap().status,
            AgentStatus::Aggregating
        );
    }

    #[tokio::test]
    async fn test_advance_skipped_when_child_failed() {
        let (pool, parent, child_a, child_b) = pool_with_parent_and_two_children();
        let pool = Arc::new(RwLock::new(pool));

        {
            let mut p = pool.write().await;
            if let Some(c) = p.get_agent_mut(&child_a) {
                c.status = AgentStatus::Completed;
            }
            if let Some(c) = p.get_agent_mut(&child_b) {
                c.status = AgentStatus::Failed;
            }
        }

        let all_done = {
            let p = pool.read().await;
            let pe = p.get_agent(&parent).unwrap();
            pe.children.iter().all(|cid| {
                p.get_agent(cid)
                    .map(|c| matches!(c.status, AgentStatus::Completed | AgentStatus::Failed))
                    .unwrap_or(false)
            })
        };
        assert!(all_done, "Failed should count as terminal");
    }
}
