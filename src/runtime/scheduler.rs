//! Task Scheduler — the pure dispatcher between TaskGraph and agents.

use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

use crate::agent::{Agent, AgentConfig, AgentPool, AgentStatus};
use crate::core::types::AgentId;
use crate::runtime::AgentRuntime;
use crate::runtime::event::RuntimeEvent;
use crate::runtime::graph_analytics::TemplateEvolution;
use crate::runtime::orchestration::{
    CapabilityRegistry, DecompositionEngine, DispatchDecider, DispatchDecision, EscalationPolicy,
    RoleSelector, TaskOutcome, TaskOutcomeStore,
};
use crate::runtime::strategy_graph::{StrategyGraph, StrategyId, StrategyType, TaskSignature};

pub struct TaskScheduler {
    runtime: Arc<RwLock<AgentRuntime>>,
    pool: Arc<RwLock<AgentPool>>,
    broker_tx: mpsc::Sender<RuntimeEvent>,
    decider: Box<dyn DispatchDecider>,
    decomposition: Option<Box<dyn DecompositionEngine>>,
    role_selector: Option<Box<dyn RoleSelector>>,
    capability_registry: Option<Arc<RwLock<CapabilityRegistry>>>,
    escalation: Option<Box<dyn EscalationPolicy>>,
    outcome_store: Option<Arc<RwLock<TaskOutcomeStore>>>,
    /// Phase 5: strategy graph for per-task strategy selection.
    strategy_graph: Option<Arc<std::sync::Mutex<StrategyGraph>>>,
    /// Phase 4: graph analytics for template evolution.
    template_evolution: Option<Arc<std::sync::Mutex<TemplateEvolution>>>,
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
            role_selector: None,
            capability_registry: None,
            escalation: None,
            outcome_store: None,
            strategy_graph: None,
            template_evolution: None,
        }
    }
    pub fn with_decomposition(mut self, engine: Box<dyn DecompositionEngine>) -> Self {
        self.decomposition = Some(engine);
        self
    }
    pub fn with_routing(
        mut self,
        sel: Box<dyn RoleSelector>,
        reg: Arc<RwLock<CapabilityRegistry>>,
    ) -> Self {
        self.role_selector = Some(sel);
        self.capability_registry = Some(reg);
        self
    }
    pub fn with_escalation(
        mut self,
        policy: Box<dyn EscalationPolicy>,
        store: Arc<RwLock<TaskOutcomeStore>>,
    ) -> Self {
        self.escalation = Some(policy);
        self.outcome_store = Some(store);
        self
    }
    pub fn with_strategy_graph(mut self, sg: Arc<std::sync::Mutex<StrategyGraph>>) -> Self {
        self.strategy_graph = Some(sg);
        self
    }
    pub fn with_graph_analytics(
        mut self,
        evolution: Arc<std::sync::Mutex<TemplateEvolution>>,
    ) -> Self {
        self.template_evolution = Some(evolution);
        self
    }

    pub async fn dispatch(&self) {
        let ready: Vec<(AgentId, String, Option<String>)> = {
            let rt = self.runtime.read().await;
            let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            g.ready_tasks()
                .iter()
                .filter_map(|tid| {
                    let n = g.get(tid)?;
                    Some((*tid, n.goal.clone(), n.role.clone()))
                })
                .collect()
        };
        if ready.is_empty() {
            return;
        }
        tracing::info!("TaskScheduler::dispatch: {} ready task(s)", ready.len());
        for (task_id, task_goal, task_role) in ready {
            let role = task_role.clone().unwrap_or_else(|| "worker".to_string());
            // ── Phase 5: StrategyGraph selection + trace recording ──
            let sig = TaskSignature {
                goal_length_chars: task_goal.len(),
                domain_count: task_role.as_ref().map(|_| 2u32).unwrap_or(1),
                estimated_complexity: 0.5,
                role_count: 1,
            };
            let selected_strategy: Option<(Option<StrategyId>, u64)> =
                self.strategy_graph.as_ref().map(|sg| {
                    let mut g = sg.lock().unwrap_or_else(|e| e.into_inner());
                    let sid = g.select_strategy(StrategyType::Estimator, 0);
                    let epoch = g.exploration.epoch;
                    (sid, epoch)
                });
            let strategy_id_for_trace = selected_strategy.and_then(|(id, _)| id);
            if let Some(sid) = strategy_id_for_trace {
                tracing::debug!("scheduler: task {:02x}.. strategy={:?}", task_id[0], sid);
            }
            // ── End Phase 5 ──
            {
                // mark_dispatching
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
            if let Some(ref engine) = self.decomposition {
                let should = {
                    let rt = self.runtime.read().await;
                    let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                    engine.should_decompose(task_id, &g)
                };
                if should {
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
                    continue;
                }
            }
            let decision_is_approved = self.decider.decide(task_id, &task_goal, &role).await;
            let decision_success =
                matches!(decision_is_approved, DispatchDecision::Approved { .. });
            match decision_is_approved {
                DispatchDecision::Approved { config } => {
                    self.apply_approved(task_id, &task_goal, &role, config)
                        .await
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
                    if let Some(ref store) = self.outcome_store {
                        if let Ok(mut s) = store.try_write() {
                            s.record(TaskOutcome {
                                task_id,
                                agent_id: None,
                                role: role.clone(),
                                success: false,
                                latency_ms: 0,
                                tokens_input: 0,
                                tokens_output: 0,
                            });
                        }
                    }
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
                        node.metadata.insert("escalated_to".into(), target_role);
                        node.metadata.insert("escalated_reason".into(), reason);
                    }
                    g.mark_blocked(task_id).ok();
                }
            }
            // ── Phase 4: Record graph metrics for template evolution ──
            if let Some(ref _evolution) = self.template_evolution {
                let rt = self.runtime.read().await;
                let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                let metrics = crate::runtime::graph_analytics::GraphMetrics::from_graph(&g);
                tracing::debug!(
                    "GraphAnalytics: {} nodes, {} roots, {} leaves",
                    metrics.node_count,
                    metrics.root_count,
                    metrics.leaf_count,
                );
            }
            // ── Phase 5: Record trace in StrategyGraph ──
            if let (Some(sg), Some(sid)) = (&self.strategy_graph, strategy_id_for_trace) {
                if let Ok(mut g) = sg.lock() {
                    g.record_trace(crate::runtime::strategy_graph::StrategyExecutionTrace {
                        trace_id: rand::random(),
                        strategy_id: sid,
                        cluster_id: Some(0),
                        task_signature: sig.clone(),
                        output_decision: serde_json::json!({"approved": decision_success}),
                        success: decision_success,
                        latency_ms: 0,
                        epoch: 0,
                    });
                }
            }
        }
    }

    async fn apply_approved(
        &self,
        task_id: AgentId,
        task_goal: &str,
        role: &str,
        config: crate::core::types::ChildAgentConfig,
    ) {
        let effective_role = if let Some(ref selector) = self.role_selector {
            let candidates = self
                .capability_registry
                .as_ref()
                .map(|reg| reg.try_read().map(|r| r.all()).unwrap_or_default())
                .unwrap_or_default();
            let routing = {
                let rt = self.runtime.read().await;
                let g = rt.task_graph.lock().unwrap_or_else(|e| e.into_inner());
                match g.get(&task_id) {
                    Some(node) => selector.select(node, &candidates),
                    None => crate::runtime::orchestration::RoutingDecision {
                        role: role.to_string(),
                        confidence: 0.5,
                        capability_score: 0.0,
                        skill_match: 0.5,
                    },
                }
            };
            if routing.confidence < 0.3 {
                tracing::warn!(
                    "routing: task {:02x}.. low confidence ({:.2})",
                    task_id[0],
                    routing.confidence
                );
            }
            routing.role
        } else {
            role.to_string()
        };

        if let Some(ref reg) = self.capability_registry {
            if let Ok(mut r) = reg.try_write() {
                r.record_outcome(&TaskOutcome {
                    task_id,
                    agent_id: None,
                    role: effective_role.clone(),
                    success: true,
                    latency_ms: 0,
                    tokens_input: 0,
                    tokens_output: 0,
                });
            }
        }

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(Arc::new)
            .ok();
        let agent = Agent {
            id: agent_id,
            name: format!(
                "{}-{:04x}",
                effective_role,
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            role: effective_role.clone(),
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
            retry_count: 0,
            reasoning: String::new(),
        };
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
        {
            let mut p = self.pool.write().await;
            p.add_agent(agent);
            if let Some(g) = {
                let rt = self.runtime.read().await;
                rt.take_pending_guard()
            } {
                p.attach_budget_guard(agent_id, g);
            }
        }
        let _ = self
            .broker_tx
            .send(RuntimeEvent::ActivateAgent {
                agent_id,
                parent_id: None,
            })
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::AgentRuntime;
    use crate::runtime::AgentRuntimeConfig;
    use crate::runtime::orchestration::{DispatchDecider, PipelineDispatchDecider};
    use std::sync::Arc;
    use tokio::sync::{RwLock, mpsc};

    /// Create a scheduler with default components for testing.
    fn test_decider() -> Box<dyn DispatchDecider> {
        Box::new(PipelineDispatchDecider::new(Arc::new(RwLock::new(
            AgentRuntime::new(AgentRuntimeConfig::default(), Arc::new(MockEmbed)),
        ))))
    }

    struct MockEmbed;
    #[async_trait::async_trait]
    impl crate::llm::EmbeddingService for MockEmbed {
        async fn embed(&self, _text: &str) -> anyhow::Result<[f32; 384]> {
            let mut e = [0.0f32; 384];
            e[0] = 1.0;
            Ok(e)
        }
        async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<[f32; 384]>> {
            Ok(texts
                .iter()
                .map(|_| {
                    let mut e = [0.0f32; 384];
                    e[0] = 1.0;
                    e
                })
                .collect())
        }
        fn similarity(&self, a: &[f32; 384], b: &[f32; 384]) -> f32 {
            crate::core::simd::cosine_similarity_384(a, b)
        }
        fn cache_size(&self) -> usize {
            0
        }
        fn clear_cache(&self) {}
        fn cache_hits(&self) -> u64 {
            0
        }
        fn cache_misses(&self) -> u64 {
            0
        }
    }

    #[tokio::test]
    async fn test_scheduler_new() {
        let rt = Arc::new(RwLock::new(AgentRuntime::new(
            AgentRuntimeConfig::default(),
            Arc::new(MockEmbed),
        )));
        let pool = Arc::new(RwLock::new(crate::agent::AgentPool::new()));
        let (_tx, _rx) = mpsc::channel(16);
        let decider = test_decider();
        let scheduler = TaskScheduler::new(rt, pool, _tx, decider);
        // Builder methods should chain
        let _ = scheduler.with_decomposition(Box::new(
            crate::runtime::orchestration::NoopDecompositionEngine,
        ));
    }

    #[tokio::test]
    async fn test_dispatch_with_no_ready_tasks() {
        let rt = Arc::new(RwLock::new(AgentRuntime::new(
            AgentRuntimeConfig::default(),
            Arc::new(MockEmbed),
        )));
        let pool = Arc::new(RwLock::new(crate::agent::AgentPool::new()));
        let (_tx, _rx) = mpsc::channel(16);
        let decider = test_decider();
        let scheduler = TaskScheduler::new(rt, pool, _tx, decider);
        // With no tasks, dispatch should be a no-op
        scheduler.dispatch().await;
        // No panic = success
    }

    #[tokio::test]
    async fn test_dispatch_with_ready_task() {
        let rt = Arc::new(RwLock::new(AgentRuntime::new(
            AgentRuntimeConfig::default(),
            Arc::new(MockEmbed),
        )));
        let pool = Arc::new(RwLock::new(crate::agent::AgentPool::new()));
        let (_tx, mut _rx) = mpsc::channel(16);
        let decider = test_decider();
        let scheduler = TaskScheduler::new(rt.clone(), pool, _tx, decider);

        // Add a ready task to the graph
        {
            let r = rt.read().await;
            let mut g = r.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            g.spawn_root("test goal");
        }

        // Dispatch should process the task
        scheduler.dispatch().await;
        // No panic = dispatch handled the task
    }

    #[tokio::test]
    async fn test_scheduler_full_chain() {
        let rt = Arc::new(RwLock::new(AgentRuntime::new(
            AgentRuntimeConfig::default(),
            Arc::new(MockEmbed),
        )));
        let pool = Arc::new(RwLock::new(crate::agent::AgentPool::new()));
        let (_tx, _rx) = mpsc::channel(16);
        let decider = test_decider();

        let scheduler = TaskScheduler::new(rt.clone(), pool.clone(), _tx, decider)
            .with_decomposition(Box::new(
                crate::runtime::orchestration::NoopDecompositionEngine,
            ))
            .with_escalation(
                Box::new(crate::runtime::orchestration::DefaultEscalationPolicy::default()),
                Arc::new(RwLock::new(
                    crate::runtime::orchestration::TaskOutcomeStore::new(),
                )),
            );

        // Add a task
        {
            let r = rt.read().await;
            let mut g = r.task_graph.lock().unwrap_or_else(|e| e.into_inner());
            g.spawn_root("complex task");
        }

        scheduler.dispatch().await;
        // After dispatch, the task should be marked running or rejected
        let r = rt.read().await;
        let g = r.task_graph.lock().unwrap_or_else(|e| e.into_inner());
        let tasks = g.ready_tasks();
        // No ready tasks remain (all have been dispatched)
        assert!(tasks.is_empty(), "all tasks should be dispatched");
    }
}
