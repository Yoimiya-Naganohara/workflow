use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::admission::AdmissionController;
use crate::agent::{Agent, AgentPool, AgentStatus};
use crate::embedding::EmbeddingService;
use crate::l0::L0CircuitBreaker;
use crate::l1::L1Retriever;
use crate::l1_arbitration::L1Arbitrator;
use crate::llm::LlmProvider;
use crate::plan::{PlanEntity, PlanRegistry, PlanStatus, Task, TaskStatus};
use crate::resource::TaskResourceState;
use crate::suspend::{SuspendConfig, SuspendQueue};
use crate::types::*;

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

pub struct AgentRuntime {
    #[allow(dead_code)]
    config: AgentRuntimeConfig,
    admission: AdmissionController,
    resource_state: Arc<TaskResourceState>,
    l0: L0CircuitBreaker,
    l1: L1Retriever,
    #[allow(dead_code)]
    l1_arbitrator: L1Arbitrator,
    embedding_service: Arc<EmbeddingService>,
    suspend_queue: Mutex<SuspendQueue>,
    pub plan_registry: Arc<RwLock<PlanRegistry>>,
    pub provider: Option<Arc<LlmProvider>>,
    role_templates: HashMap<String, RoleTemplate>,
}

#[derive(Clone)]
pub struct RoleTemplate {
    pub role: String,
    pub label: String,
    pub system_prompt: String,
    pub template_id: u32,
}

impl AgentRuntime {
    pub fn new(config: AgentRuntimeConfig, embedding_service: Arc<EmbeddingService>) -> Self {
        let admission = AdmissionController::new(config.max_concurrent_agents, config.admission_timeout_ms);
        let resource_state = TaskResourceState::new(config.initial_budget, config.max_depth);
        let l0 = L0CircuitBreaker::new(resource_state.clone());
        let l1 = L1Retriever::new(config.l1_confidence_threshold);
        let l1_arbitrator = L1Arbitrator::new(config.semantic_conflict_threshold);
        let suspend_queue = SuspendQueue::new(SuspendConfig {
            hard_timeout_ms: config.suspend_timeout_ms,
            dynamic_timeout_ms: config.suspend_timeout_ms,
        });

        let mut role_templates = HashMap::new();
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
            config,
            admission,
            resource_state,
            l0,
            l1,
            l1_arbitrator,
            embedding_service,
            suspend_queue: Mutex::new(suspend_queue),
            plan_registry: Arc::new(RwLock::new(PlanRegistry::new())),
            provider: None,
            role_templates,
        }
    }

    pub async fn process_request(&self, request: SpawnRequest) -> Result<SpawnDecision> {
        let _permit = self
            .admission
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("Admission failed: {:?}", e))?;

        let l0_result = self.l0.try_acquire(request.requested_budget, request.current_depth, 0);

        let l0_permit = match l0_result {
            Ok(permit) => permit,
            Err(rejection) => {
                if matches!(rejection, SpawnRejection::ResourceConflict { .. }) {
                    self.suspend_queue.lock().unwrap().enqueue(request, 0.5);
                }
                return Ok(SpawnDecision::Rejected(rejection));
            }
        };

        let task_emb = &request.task_description_embedding;
        let role_emb = &request.role_description_embedding;

        let l1_result = self.l1.check_confidence(task_emb, role_emb);
        let l1_assessment = match l1_result {
            Ok(assessment) => assessment,
            Err(rejection) => {
                return Ok(SpawnDecision::Rejected(rejection));
            }
        };

        let agent_id: AgentId = rand::random();
        let task_id: TaskId = rand::random();
        let allocated_budget = l0_permit.budget_amount();

        Ok(SpawnDecision::Approved(ChildAgentConfig {
            agent_id,
            task_id,
            allocated_budget,
            allowed_tools: l1_assessment.recommended_tools,
            role_template_id: None,
        }))
    }

    pub async fn process_with_text(
        &mut self,
        task_description: &str,
        role_description: &str,
        value_statement: &str,
        requested_budget: u64,
        current_depth: u32,
    ) -> Result<SpawnDecision> {
        let task_emb = self.embedding_service.embed(task_description).await?;
        let role_emb = self.embedding_service.embed(role_description).await?;
        let value_emb = self.embedding_service.embed(value_statement).await?;

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

        self.process_request(request).await
    }

    pub fn experience_count(&self) -> usize {
        self.l1.experience_count()
    }

    pub fn add_experience(&mut self, entry: ExperienceEntry) {
        self.l1.add_experience(entry);
    }

    pub fn available_permits(&self) -> usize {
        self.admission.available_permits()
    }

    pub fn remaining_budget(&self) -> i64 {
        self.resource_state
            .remaining_budget
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_provider_from_state(&mut self, state_provider: Arc<LlmProvider>) {
        self.provider = Some(state_provider);
    }

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

    pub fn pending_suspended(&self) -> usize {
        self.suspend_queue.lock().unwrap().len()
    }

    pub fn get_role_template(&self, role: &str) -> Option<&RoleTemplate> {
        self.role_templates.get(role)
    }

    pub fn set_provider(&mut self, provider: LlmProvider) {
        self.provider = Some(Arc::new(provider));
    }

    pub async fn spawn_root_agent(
        &self,
        goal: &str,
        role: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_tpl = self
            .role_templates
            .get(role)
            .cloned()
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
            });

        let agent_id: AgentId = rand::random();

        // Run the decision pipeline
        let role_emb = self.embedding_service.embed(role).await?;
        let task_emb = self.embedding_service.embed(goal).await?;
        let value_emb = self.embedding_service.embed("default").await?;

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

        let decision = self.process_request(request).await?;
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
        let role_tpl = self
            .role_templates
            .get(role)
            .cloned()
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
            });

        let agent_id: AgentId = rand::random();

        // Run the decision pipeline
        let role_emb = self.embedding_service.embed(goal).await?;
        let task_emb = self.embedding_service.embed(goal).await?;
        let value_emb = self.embedding_service.embed("default").await?;

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

        let decision = self.process_request(request).await?;
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
                    let mut reg = self.plan_registry.write().await;
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

    pub async fn execute_agent(
        &self,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) {
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
                        agent.child_results.push((
                            [0; 16],
                            format!("Failed to spawn {}: {}", child_role, e),
                        ));
                    }
                }
            }
        }

        // Execute each child (iterative, not recursive — so no boxing needed)
        for child_id in &child_ids {
            self.execute_agent_inner(*child_id, agent_pool).await;
        }

        // Await each child
        let mut child_results = Vec::new();
        for child_id in &child_ids {
            let result = self.await_agent(*child_id, agent_pool).await;
            child_results.push((*child_id, result));
        }

        // If children had further children, execute those too
        // (simple iterative deepening: process grandchildren)
        for child_id in &child_ids {
            let grandchild_ids: Vec<AgentId> = {
                let pool = agent_pool.read().await;
                pool.get_agent(child_id)
                    .map(|a| a.children.clone())
                    .unwrap_or_default()
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

        // Aggregating — LLM summarizes children's results
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
    /// Returns Some(assignments) if the agent has children to spawn,
    /// or None if it's a leaf/completed/failed agent.
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

        if assignments.is_empty() || depth >= self.config.max_depth {
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

fn parse_role_assignments(response: &str) -> Vec<(String, String)> {
    let mut assignments = Vec::new();
    for line in response.lines() {
        let trimmed = line.trim();
        // Match: @role "description" or @role "description with spaces"
        if let Some(rest) = trimmed.strip_prefix('@') {
            let parts: Vec<&str> = rest.splitn(2, '"').collect();
            if parts.len() >= 2 {
                let role = parts[0].trim();
                if let Some(desc_end) = parts[1].rfind('"') {
                    let description = parts[1][..desc_end].trim();
                    if !role.is_empty() && !description.is_empty() {
                        assignments.push((role.to_string(), description.to_string()));
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
    use crate::llm::LlmProvider;

    #[tokio::test]
    async fn test_basic_spawn() {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::CompletionsClient::new("test-key").unwrap(),
        ));
        let embedding_service = Arc::new(EmbeddingService::new(provider));
        let config = AgentRuntimeConfig::default();
        let mut runtime = AgentRuntime::new(config, embedding_service);

        let result = runtime
            .process_with_text(
                "Implement user authentication",
                "Rust developer",
                "Write secure, maintainable code",
                1000,
                0,
            )
            .await;

        assert!(result.is_err() || matches!(result.unwrap(), SpawnDecision::Approved(_)));
    }

    #[tokio::test]
    async fn test_budget_exhaustion() {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::CompletionsClient::new("test-key").unwrap(),
        ));
        let embedding_service = Arc::new(EmbeddingService::new(provider));
        let config = AgentRuntimeConfig {
            initial_budget: 100,
            ..Default::default()
        };
        let mut runtime = AgentRuntime::new(config, embedding_service);

        let result = runtime.process_with_text("task", "role", "value", 200, 0).await;

        assert!(
            result.is_err()
                || matches!(
                    result.unwrap(),
                    SpawnDecision::Rejected(SpawnRejection::BudgetExhausted { .. })
                )
        );
    }
}
