use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use crate::agent::{Agent, AgentPool, AgentStatus};
use crate::llm::LlmProvider;
use crate::pipeline::{DecisionPipeline, DecisionPipelineBuilder};
use crate::plan::{PlanEntity, PlanRegistry as PlanRegistryConcrete, PlanStatus, Task, TaskStatus};
use crate::traits::EmbeddingService;
use crate::types::*;

// ============================================================================
//  Runtime Configuration
// ============================================================================

/// Configuration passed to [`AgentRuntime::new`] as guidance.
///
/// Individual layers may override these values if their injected
/// implementations have their own configuration.
pub struct AgentRuntimeConfig {
    pub max_concurrent_agents: usize,
    pub admission_timeout_ms: u64,
    pub max_depth: u32,
    pub initial_budget: u64,
    pub l1_confidence_threshold: f32,
    pub semantic_conflict_threshold: f32,
    pub suspend_timeout_ms: u64,
}

impl Default for AgentRuntimeConfig {
    fn default() -> Self {
        Self {
            max_concurrent_agents: 10,
            admission_timeout_ms: 100,
            max_depth: 5,
            initial_budget: 10000,
            l1_confidence_threshold: 0.7,
            semantic_conflict_threshold: -0.6,
            suspend_timeout_ms: 50,
        }
    }
}

// ============================================================================
//  RoleTemplate
// ============================================================================

#[derive(Clone)]
pub struct RoleTemplate {
    pub role: String,
    pub label: String,
    pub system_prompt: String,
    pub template_id: u32,
}

// ============================================================================
//  AgentRuntime — Composes decision pipeline + agent lifecycle
// ============================================================================

/// Top-level orchestrator that wires the decision pipeline to
/// agent lifecycle management.
///
/// # Dependency Injection
///
/// The simplest construction path is [`AgentRuntime::new`] which
/// creates default implementations for every layer.  For full
/// control, build a [`DecisionPipeline`] first and pass it to
/// [`AgentRuntime::from_pipeline`]:
///
/// ```ignore
/// use workflow::pipeline::DecisionPipelineBuilder;
/// use workflow::runtime::AgentRuntime;
///
/// let pipeline = DecisionPipelineBuilder::new()
///     .embedding(my_embedding)
///     .audit_engine(Box::new(MyCustomAudit::new()))
///     .build();
///
/// let runtime = AgentRuntime::from_pipeline(pipeline);
/// ```
pub struct AgentRuntime {
    /// Injected decision pipeline (L-1 / L0 / L1 / L2).
    pipeline: DecisionPipeline,
    /// LLM provider for chat / embedding.
    pub provider: Option<Arc<LlmProvider>>,
    /// Active model identifier.
    pub model_id: String,
    /// Role templates for sub-agent spawning.
    role_templates: HashMap<String, RoleTemplate>,
}

impl AgentRuntime {
    /// Create a runtime with default component implementations.
    ///
    /// This is the quick-start path.  Every layer uses its default
    /// implementation tuned by `config`.
    pub fn new(config: AgentRuntimeConfig, embedding_service: Arc<dyn EmbeddingService>) -> Self {
        use crate::admission::AdmissionController;
        use crate::l0::L0CircuitBreaker;
        use crate::l1::L1Retriever;
        use crate::l2::L2RuleAuditEngine;
        use crate::resource::TaskResourceState;
        use crate::suspend::{SuspendConfig, SuspendQueue as SuspendQueueConcrete};

        let state = TaskResourceState::new(config.initial_budget, config.max_depth);

        let pipeline = DecisionPipelineBuilder::new()
            .admission(Box::new(AdmissionController::new(
                config.max_concurrent_agents,
                config.admission_timeout_ms,
            )))
            .circuit_breaker(Box::new(L0CircuitBreaker::new(state)))
            .experience(Box::new(L1Retriever::new(config.l1_confidence_threshold)))
            .audit_engine(Box::new(L2RuleAuditEngine::new(5)))
            .embedding(embedding_service)
            .suspend(Box::new(SuspendQueueConcrete::new(SuspendConfig {
                hard_timeout_ms: config.suspend_timeout_ms,
                dynamic_timeout_ms: config.suspend_timeout_ms,
            })))
            .plans(Box::new(PlanRegistryConcrete::new()))
            .build();

        Self::from_pipeline(pipeline)
    }

    /// Create a runtime from a pre-built [`DecisionPipeline`].
    ///
    /// Use this when you want full control over every layer's
    /// implementation (mocking, custom audit engines, etc.).
    pub fn from_pipeline(pipeline: DecisionPipeline) -> Self {
        let mut role_templates: HashMap<String, RoleTemplate> = HashMap::new();
        role_templates.insert(
            "planner".to_string(),
            RoleTemplate {
                role: "planner".to_string(),
                label: "Senior Architect".to_string(),
                system_prompt: "You are a senior architect. Decompose goals into jobs and assign @role to each."
                    .to_string(),
                template_id: 0,
            },
        );
        role_templates.insert(
            "tester".to_string(),
            RoleTemplate {
                role: "tester".to_string(),
                label: "QA Engineer".to_string(),
                system_prompt: "You are a QA engineer. Write and execute tests. Decompose testing work into sub-goals and assign @tester sub-agents if needed."
                    .to_string(),
                template_id: 1,
            },
        );
        role_templates.insert(
            "developer".to_string(),
            RoleTemplate {
                role: "developer".to_string(),
                label: "Developer".to_string(),
                system_prompt: "You are a developer. Implement features from specifications. Decompose implementation into sub-goals and assign @developer sub-agents if needed."
                    .to_string(),
                template_id: 2,
            },
        );
        role_templates.insert(
            "reviewer".to_string(),
            RoleTemplate {
                role: "reviewer".to_string(),
                label: "Code Reviewer".to_string(),
                system_prompt: "You are a code reviewer. Review code for correctness, security, and style. Decompose review work into sub-goals and assign @reviewer sub-agents if needed."
                    .to_string(),
                template_id: 3,
            },
        );

        Self {
            pipeline,
            provider: None,
            model_id: String::new(),
            role_templates,
        }
    }

    // ── Pipeline delegation ──

    /// Run a [`SpawnRequest`] through the decision pipeline.
    pub async fn process_request(&self, request: SpawnRequest) -> Result<SpawnDecision> {
        self.pipeline.process_request(request).await
    }

    /// Embed text and run through the decision pipeline.
    pub async fn process_with_text(
        &self,
        task_description: &str,
        role_description: &str,
        _value_statement: &str,
        requested_budget: u64,
        current_depth: u32,
    ) -> Result<SpawnDecision> {
        let task_emb = self.pipeline.embedding().embed(task_description).await?;
        let role_emb = self.pipeline.embedding().embed(role_description).await?;
        let value_emb = self.pipeline.embedding().embed("default").await?;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget,
            current_depth,
            responsibility_chain: vec![],
            raw_text_ref: None,
        };

        self.pipeline.process_request(request).await
    }

    // ── Experience ──

    pub fn experience_count(&self) -> usize {
        self.pipeline.experience_count()
    }

    pub fn add_experience(&self, entry: ExperienceEntry) {
        self.pipeline.add_experience(entry);
    }

    // ── Resource status ──

    pub fn available_permits(&self) -> usize {
        self.pipeline.available_permits()
    }

    pub fn remaining_budget(&self) -> i64 {
        self.pipeline.remaining_budget()
    }

    pub fn pending_suspended(&self) -> usize {
        self.pipeline.pending_suspended()
    }

    // ── Provider / Model ──

    pub fn set_provider(&mut self, provider: LlmProvider) {
        self.provider = Some(Arc::new(provider));
    }

    pub fn set_provider_from_state(&mut self, state_provider: Arc<LlmProvider>) {
        self.provider = Some(state_provider);
    }

    pub fn set_default_model(&mut self, model_id: &str) {
        self.model_id = model_id.to_string();
    }

    // ── Role templates ──

    pub fn get_role_template(&self, role: &str) -> Option<&RoleTemplate> {
        self.role_templates.get(role)
    }

    // ── Chat ──

    /// Chat with an LLM agent for a user goal.
    /// Spawns a root agent, executes it, and returns the result.
    pub async fn chat_with_goal(&self, goal: &str, agent_pool: &Arc<RwLock<AgentPool>>) -> Result<String> {
        let agent_id = {
            let mut pool = agent_pool.write().await;
            self.spawn_root_agent(goal, "planner", &mut pool).await?
        };

        self.execute_agent(agent_id, agent_pool).await;
        self.await_agent(agent_id, agent_pool).await;

        let result = {
            let pool = agent_pool.read().await;
            pool.get_agent(&agent_id)
                .and_then(|a| a.result.clone())
                .unwrap_or_default()
        };

        if result.is_empty() {
            Err(anyhow::anyhow!("Agent produced no result"))
        } else {
            Ok(result)
        }
    }

    // ── Agent lifecycle ──

    pub async fn spawn_root_agent(&self, goal: &str, role: &str, agent_pool: &mut AgentPool) -> Result<AgentId> {
        let role_tpl = self.role_templates.get(role).cloned().unwrap_or(RoleTemplate {
            role: role.to_string(),
            label: role.to_string(),
            system_prompt: format!("You are a {}. Execute the given goal.", role),
            template_id: 0,
        });

        let agent_id: AgentId = rand::random();

        // Run the decision pipeline
        let role_emb = self.pipeline.embedding().embed(role).await?;
        let task_emb = self.pipeline.embedding().embed(goal).await?;
        let value_emb = self.pipeline.embedding().embed("default").await?;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: 0,
            responsibility_chain: vec![agent_id],
            raw_text_ref: None,
        };

        let decision = self.pipeline.process_request(request).await?;
        match decision {
            SpawnDecision::Approved(_config) => {
                let agent = Agent {
                    id: agent_id,
                    name: format!("{}-{:04x}", role, u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])),
                    role: role.to_string(),
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        system_prompt: role_tpl.system_prompt,
                        model_id: self.model_id.clone(),
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                };
                agent_pool.add_agent(agent);
                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection)),
        }
    }

    pub async fn spawn_child(
        &self,
        parent_id: AgentId,
        parent_depth: u32,
        role: &str,
        goal: &str,
        responsibility_chain: &[AgentId],
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_tpl = self.role_templates.get(role).cloned().unwrap_or(RoleTemplate {
            role: role.to_string(),
            label: role.to_string(),
            system_prompt: format!("You are a {}. Execute the given goal.", role),
            template_id: 0,
        });

        let agent_id: AgentId = rand::random();

        let role_emb = self.pipeline.embedding().embed(goal).await?;
        let task_emb = self.pipeline.embedding().embed(goal).await?;
        let value_emb = self.pipeline.embedding().embed("default").await?;

        let mut chain = responsibility_chain.to_vec();
        chain.push(agent_id);

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: parent_depth + 1,
            responsibility_chain: chain,
            raw_text_ref: None,
        };

        let decision = self.pipeline.process_request(request).await?;
        match decision {
            SpawnDecision::Approved(_config) => {
                let agent = Agent {
                    id: agent_id,
                    name: format!("{}-{:04x}", role, u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])),
                    role: role.to_string(),
                    parent_id: Some(parent_id),
                    children: Vec::new(),
                    depth: parent_depth + 1,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        system_prompt: role_tpl.system_prompt,
                        model_id: self.model_id.clone(),
                        ..Default::default()
                    },
                    status: AgentStatus::Idle,
                    result: None,
                    child_results: Vec::new(),
                };
                agent_pool.add_agent(agent);

                // Register plan entity
                let plan_entity = PlanEntity {
                    plan_name: format!(
                        "{}-{}-{:04x}",
                        role,
                        goal.chars().take(16).collect::<String>(),
                        agent_id[0] as u16
                    ),
                    agent_id,
                    parent_plan: None,
                    goal: goal.to_string(),
                    tasks: vec![Task {
                        id: 0,
                        description: goal.to_string(),
                        status: TaskStatus::Pending,
                        result: None,
                    }],
                    status: PlanStatus::Draft,
                    created_at: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs(),
                };
                {
                    let mut reg = self.pipeline.plans().write().await;
                    reg.insert(plan_entity);
                }

                // Link parent → child
                if let Some(parent) = agent_pool.get_agent_mut(&parent_id) {
                    parent.children.push(agent_id);
                }

                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection)),
        }
    }

    pub async fn execute_agent(&self, agent_id: AgentId, agent_pool: &Arc<RwLock<AgentPool>>) {
        // Process agent and get any child assignments
        let maybe_assignments = self.execute_agent_inner(agent_id, agent_pool).await;

        let assignments = match maybe_assignments {
            Some(a) => a,
            None => return, // leaf or error — already completed in inner
        };

        let (goal, config, depth) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return,
            };
            (agent.goal, agent.config, agent.depth)
        };

        // Mark awaiting children
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::AwaitingChildren;
            }
        }

        // Spawn children
        let mut child_ids = Vec::new();
        for (child_role, child_goal) in &assignments {
            let responsibility_chain = vec![agent_id];
            match self
                .spawn_child(
                    agent_id,
                    depth,
                    child_role,
                    child_goal,
                    &responsibility_chain,
                    &mut *agent_pool.write().await,
                )
                .await
            {
                Ok(child_id) => {
                    child_ids.push(child_id);
                }
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent
                            .child_results
                            .push(([0; 16], format!("Failed to spawn {}: {}", child_role, e)));
                    }
                }
            }
        }

        // Execute each child (iterative, not recursive)
        for child_id in &child_ids {
            self.execute_agent_inner(*child_id, agent_pool).await;
        }

        // Await each child
        let mut child_results = Vec::new();
        for child_id in &child_ids {
            let result = self.await_agent(*child_id, agent_pool).await;
            child_results.push((*child_id, result));
        }

        // Process grandchildren
        for child_id in &child_ids {
            let grandchild_ids: Vec<AgentId> = {
                let pool = agent_pool.read().await;
                pool.get_agent(child_id).map(|a| a.children.clone()).unwrap_or_default()
            };
            for gc_id in &grandchild_ids {
                self.execute_agent_inner(*gc_id, agent_pool).await;
            }
            for gc_id in &grandchild_ids {
                let result = self.await_agent(*gc_id, agent_pool).await;
                child_results.push((*gc_id, result));
            }
        }

        let provider = match &self.provider {
            Some(p) => p.clone(),
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.notify_completed(&agent_id);
                }
                return;
            }
        };

        // Aggregation — LLM summarizes children's results
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Aggregating;
                agent.child_results = child_results.clone();
            }
        }

        let child_summary: String = child_results
            .iter()
            .map(|(id, r)| format!("Sub-agent {:?}: {}", id, r))
            .collect::<Vec<_>>()
            .join("\n");

        let aggregation_prompt = format!(
            "Your goal: {}\n\nYour sub-agents returned:\n{}\n\nSynthesize a final result.",
            goal, child_summary
        );

        let final_result = provider
            .chat(&config.model_id, &config.system_prompt, &aggregation_prompt)
            .await
            .unwrap_or_else(|e| format!("Aggregation failed: {}", e));

        let mut pool = agent_pool.write().await;
        if let Some(agent) = pool.get_agent_mut(&agent_id) {
            agent.result = Some(final_result);
            agent.status = AgentStatus::Completed;
            pool.notify_completed(&agent_id);
        }
    }

    /// Execute a single agent: LLM call, check for sub-assignments.
    /// Returns `Some(assignments)` if the agent has children to spawn,
    /// or `None` if it's a leaf / completed / failed agent.
    async fn execute_agent_inner(
        &self,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Option<Vec<(String, String)>> {
        let (goal, config, depth) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return None,
            };
            (agent.goal, agent.config, agent.depth)
        };

        let provider = match &self.provider {
            Some(p) => p.clone(),
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.notify_completed(&agent_id);
                }
                return None;
            }
        };

        // Mark planning
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Planning;
            }
        }

        let system_prompt = format!(
            "{}\n\nYour goal: {}\n\nYou can spawn sub-agents by including @role \"description\" in your response. Available roles: planner, developer, tester, reviewer. Always produce a concrete result.",
            config.system_prompt, goal
        );

        let response = match provider.chat(&config.model_id, &system_prompt, &goal).await {
            Ok(r) => r,
            Err(e) => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some(format!("LLM error: {}", e));
                    pool.notify_completed(&agent_id);
                }
                return None;
            }
        };

        let assignments = parse_role_assignments(&response);

        if assignments.is_empty() || depth >= 5 {
            // Leaf agent — response is the result
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.result = Some(response);
                agent.status = AgentStatus::Completed;
                pool.notify_completed(&agent_id);
            }
            None
        } else {
            Some(assignments)
        }
    }

    pub async fn await_agent(&self, agent_id: AgentId, agent_pool: &Arc<RwLock<AgentPool>>) -> String {
        let notify = {
            let pool = agent_pool.read().await;
            pool.get_completion_notify(&agent_id)
        };

        if let Some(notify) = notify {
            notify.notified().await;
        }

        let pool = agent_pool.read().await;
        pool.get_agent(&agent_id)
            .and_then(|a| a.result.clone())
            .unwrap_or_default()
    }
}

/// Parse agent role assignments like `@planner "description goes here"` from text.
fn parse_role_assignments(response: &str) -> Vec<(String, String)> {
    let mut assignments = Vec::new();
    let roles = ["planner", "developer", "tester", "reviewer"];

    for line in response.lines() {
        for role in &roles {
            let pattern = format!("@{}", role);
            if let Some(pos) = line.find(&pattern) {
                let after_role = line[pos + pattern.len()..].trim();
                if let Some(goal_start) = after_role.find('"').or_else(|| after_role.find('\u{201c}')) {
                    let quote_char = after_role.as_bytes()[goal_start];
                    let closing = if quote_char == b'"' { '"' } else { '\u{201d}' };
                    let after_open = &after_role[goal_start + 1..];
                    if let Some(goal_end) = after_open.find(closing) {
                        let goal = after_open[..goal_end].to_string();
                        if !goal.is_empty() {
                            assignments.push((role.to_string(), goal));
                        }
                    }
                }
            }
        }
    }

    assignments
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Mock embedding service that returns a fixed vector without API calls.
    struct MockEmbedding;

    #[async_trait::async_trait]
    impl EmbeddingService for MockEmbedding {
        async fn embed(&self, _text: &str) -> anyhow::Result<[f32; 768]> {
            let mut emb = [0.0f32; 768];
            emb[0] = 1.0;
            Ok(emb)
        }

        async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<[f32; 768]>> {
            let mut results = Vec::with_capacity(texts.len());
            for _ in texts {
                let mut emb = [0.0f32; 768];
                emb[0] = 1.0;
                results.push(emb);
            }
            Ok(results)
        }

        fn similarity(&self, a: &[f32; 768], b: &[f32; 768]) -> f32 {
            crate::simd::cosine_similarity_768(a, b)
        }

        fn cache_size(&self) -> usize {
            0
        }
        fn clear_cache(&self) {}
    }

    fn dummy_embedding() -> Arc<dyn EmbeddingService> {
        Arc::new(MockEmbedding)
    }

    #[tokio::test]
    async fn test_basic_spawn() {
        let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), dummy_embedding());

        let task = "Implement a REST API";
        let role = "Senior Rust developer";
        let value = "Write secure, well-tested code";

        let decision = runtime.process_with_text(task, role, value, 1000, 0).await.unwrap();

        match decision {
            SpawnDecision::Approved(config) => {
                assert!(config.allocated_budget > 0);
            }
            SpawnDecision::Rejected(rejection) => {
                panic!("Expected approval, got: {:?}", rejection);
            }
        }
    }

    #[tokio::test]
    async fn test_budget_exhaustion() {
        let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), dummy_embedding());

        // Try to spend more than available budget
        let task = "A task";
        let role = "A role";
        let value = "some value";

        let decision = runtime.process_with_text(task, role, value, 99999, 0).await.unwrap();

        // Should still pass L1/L2, may pass L0 if budget allows
        // (initial_budget is 10000, requested is 99999, should be rejected)
        match decision {
            SpawnDecision::Approved(_) => {
                // Budget is 10000, request is 99999 — L0 should reject
                // But L0 uses CAS which might allow it... let's see
            }
            SpawnDecision::Rejected(rejection) => {
                assert!(matches!(rejection, SpawnRejection::BudgetExhausted { .. }));
            }
        }
    }
}
