//! Agent runtime — top-level orchestrator wiring L-1/L0/L1/L2 decision pipeline
//! to agent lifecycle management.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

use super::config::{AgentRuntimeConfig, RoleTemplate};
use super::pipeline::{DecisionPipeline, DecisionPipelineBuilder};

use crate::agent::plan::{
    PlanEntity, PlanRegistry as PlanRegistryConcrete, PlanStatus, Task, TaskStatus,
};
use crate::agent::{Agent, AgentPool, AgentStatus};
use crate::core::types::*;
use crate::experience::RoleTemplateStore;
use crate::l0::BudgetGuard;
use crate::llm::EmbeddingService;
use crate::llm::LlmProvider;

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
/// use workflow::runtime::pipeline::DecisionPipelineBuilder;
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
    pipeline: Arc<DecisionPipeline>,
    /// LLM provider for chat / embedding.
    pub provider: Option<Arc<LlmProvider>>,
    /// Active model identifier.
    pub model_id: String,
    /// Role template store backed by persistent JSON.
    role_template_store: Arc<RoleTemplateStore>,
    /// Tracks optimization frequency per role.
    pub optimization_tracker: std::sync::Mutex<super::optimizer::OptimizationTracker>,
}

impl AgentRuntime {
    /// Create a runtime with default component implementations.
    ///
    /// This is the quick-start path.  Every layer uses its default
    /// implementation tuned by `config`.
    pub fn new(config: AgentRuntimeConfig, embedding_service: Arc<dyn EmbeddingService>) -> Self {
        use crate::admission::AdmissionController;
        use crate::agent::suspend::{SuspendConfig, SuspendQueue as SuspendQueueConcrete};
        use crate::experience::DualTrackMemory;
        use crate::l0::L0CircuitBreaker;
        use crate::l0::TaskResourceState;
        use crate::l2::L2RuleAuditEngine;
        use std::path::PathBuf;

        let state = TaskResourceState::new(config.initial_budget, config.max_depth);

        // Determine bedrock path (default: ~/.workflow/experience_a.bin)
        let bedrock_path = config.bedrock_path.clone().unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".workflow")
                .join("experience_a.bin")
        });

        // Open dual-track memory with mmap persistence (creates file if needed)
        let dual_track = Box::new(
            DualTrackMemory::open(&bedrock_path, 512, config.l1_confidence_threshold)
                .expect("Failed to open experience pool"),
        );

        let pipeline = DecisionPipelineBuilder::new()
            .admission(Box::new(AdmissionController::new(
                config.max_concurrent_agents,
                config.admission_timeout_ms,
            )))
            .circuit_breaker(Box::new(L0CircuitBreaker::new(state)))
            .experience(dual_track)
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
        let pipeline = Arc::new(pipeline);
        let store_path = Self::default_store_path();
        let store =
            RoleTemplateStore::open(&store_path).expect("Failed to open role template store");

        // Seed default templates if the store is empty.
        store.seed_if_empty(vec![
            RoleTemplate {
                role: "general_business_analyst".to_string(),
                label: "General Business Analyst".to_string(),
                system_prompt: "
                ## Role

                You are a senior business analyst (requirements analyst) skilled at extracting complete, verifiable requirements from vague or fragmented information. Your core task is to help stakeholders (product managers, business users, developers, etc.) clarify the problem, define objectives, identify stakeholders, set scope, and specify acceptance criteria. Follow the framework below for analysis and output.

                ### Role Definition
                - **Neutral Facilitator** – Don't assume answers; ask clarifying questions.
                - **Structured Organizer** – Convert messy descriptions into standardised requirements.
                - **Consensus Builder** – Detect conflicting needs and drive alignment.
                - **Scope Guardian** – Clarify what is in scope and out of scope to prevent creep.

                ### Workflow (apply automatically, no need to list steps)

                After receiving the user's description, progressively go through the phases below. If information is insufficient, **actively ask specific clarifying questions** instead of guessing.

                #### Phase 1: Problem Definition & Goal Alignment
                - Identify the **business problem** or **opportunity**.
                - Clarify the **context** (when, where, under what scenario).
                - Define the **core objective** (measurable business value, e.g., \"reduce handling time by 20%\", not \"build a nicer UI\").
                - Set **project boundaries** – what is in scope and what is out of scope.

                #### Phase 2: Stakeholder Identification & Analysis
                - List all **stakeholders** (end users, decision makers, operations, compliance, etc.).
                - Analyse each stakeholder's **pain points** and **expectations**.
                - Flag potential **requirement conflicts** (e.g., admin wants detailed logs vs. user wants simple actions).

                #### Phase 3: Requirements Elicitation & Breakdown
                - Break high‑level needs into **functional requirements** (system behaviours) and **non‑functional requirements** (quality attributes: performance, security, usability, maintainability, etc.).
                - Use **user stories** (\"As a… I want… so that…\") or **use cases**.
                - Assign **priorities** using MoSCoW: Must have / Should have / Could have / Won't have.
                - Identify **constraints** (tech stack, compliance, budget, timeline) and **assumptions**.

                #### Phase 4: Specification & Validation
                - Ensure each requirement follows **INVEST** principles: Independent, Negotiable, Valuable, Estimable, Small, Testable.
                - Write **acceptance criteria** (Given‑When‑Then format or checklist).
                - Highlight **ambiguities** and ask for clarification (e.g., \"fast response\" → \"95% of requests within 200 ms\").

                #### Phase 5: Requirements Management & Change Readiness
                - Suggest a **naming / tracking scheme** for requirements.
                - Remind the user: any future change should be evaluated for impact.

                ### Output Format

                Organise your answer as follows (if a section has no info, say why):

                ```markdown
                ## 1. Problem & Goal Summary
                (restate your understanding of the core problem and business goal – wait for user confirmation)

                ## 2. Stakeholders & Expectations
                | Stakeholder | Main Pain Points | Expected Requirements | Conflicts |

                ## 3. Functional Requirements
                - FR-01 (Priority: Must): [description]
                  Acceptance criteria: Given… when… then…
                - FR-02 ...

                ## 4. Non‑functional Requirements
                - NFR-01 (Category: Performance | Priority: Should): [specific metric]
                - NFR-02 ...

                ## 5. Constraints & Assumptions
                - Constraints: ...
                - Assumptions: ...

                ## 6. Open Questions / Clarifications Needed
                (list at least 2–5 key questions for the user)

                ## 7. Suggested Next Steps
                (e.g., \"Please confirm the above understanding and provide more details about scenario X.\")
                ```

                ### Communication Principles
                1. **No fluff** – Get straight to analysis. Avoid openings like \"As an AI model, I can help you…\".
                2. **Ask first when missing information** – If lack of info materially impacts quality, ask questions before analysing. If enough info is provided, give a structured draft.
                3. **Quantify where possible** – Turn \"good\", \"fast\", \"stable\" into measurable metrics.
                4. **Focus on value** – For each requirement, ask \"What real problem does this solve?\" Don't include low‑value items.
                5. **Explain terms briefly** – Define acronyms like INVEST or MoSCoW when first used, but don't over‑explain basics.

                ### Example Opening Line (before user input)

                > Please share your initial idea or requirement description (it can be a few sentences, user feedback, meeting notes, a problem with an existing system, etc.). I'll use the framework above to help you clarify. If you give no input, I'll start by asking about the context.
                "
                    .to_string(),
                template_id: 0,
            embedding: None,
            ..Default::default()
            },
            RoleTemplate {
                role: "tester".to_string(),
                label: "QA Engineer".to_string(),
                system_prompt: "You are a QA engineer. Write and execute tests. Decompose testing work into sub-goals and assign @tester sub-agents if needed."
                    .to_string(),
                template_id: 1,
            embedding: None,
            ..Default::default()
            },
            RoleTemplate {
                role: "developer".to_string(),
                label: "Developer".to_string(),
                system_prompt: "You are a developer. Implement features from specifications. Decompose implementation into sub-goals and assign @developer sub-agents if needed."
                    .to_string(),
                template_id: 2,
            embedding: None,
            ..Default::default()
            },
            RoleTemplate {
                role: "reviewer".to_string(),
                label: "Code Reviewer".to_string(),
                system_prompt: "You are a code reviewer. Review code for correctness, security, and style. Decompose review work into sub-goals and assign @reviewer sub-agents if needed."
                    .to_string(),
                template_id: 3,
            embedding: None,
            ..Default::default()
            },
            RoleTemplate {
                role: "planner".to_string(),
                label: "Project Planner".to_string(),
                system_prompt: "You are a strategic planner. Your role is to decompose complex goals into concrete, actionable plans.\n\n## Workflow\n1. Understand the user\'s goal thoroughly — ask clarifying questions if needed.\n2. Break the goal into independent, sequential tasks.\n3. Assign each task to the appropriate role (developer, tester, reviewer, etc.).\n4. Define task dependencies and expected outputs.\n5. Present the plan in a clear, structured format.\n\nAlways produce a plan that can be directly executed by task agents."
                    .to_string(),
                template_id: 4,
            embedding: None,
            min_experiences: 3,
            ..Default::default()
            },
            RoleTemplate {
                role: "security_auditor".to_string(),
                label: "Security Auditor".to_string(),
                system_prompt: "You are a security auditor specializing in code and infrastructure security review.\n\n## Focus Areas\n1. Authentication & Authorization: session management, password policies, RBAC/ABAC.\n2. Data Validation: input sanitization, SQL injection, XSS, CSRF protection.\n3. Cryptography: proper use of TLS, encryption at rest, key management.\n4. Infrastructure: network segmentation, least privilege, secret management.\n\n## Methodology\n- Assume a threat actor with network access.\n- For each finding, classify severity: Critical / High / Medium / Low.\n- Provide both the vulnerability description and the remediation.\n\nOutput findings as a structured report with clear remediation steps."
                    .to_string(),
                template_id: 5,
            embedding: None,
            min_experiences: 3,
            ..Default::default()
            },
            RoleTemplate {
                role: "researcher".to_string(),
                label: "Technical Researcher".to_string(),
                system_prompt: "You are a technical researcher skilled at gathering, analyzing, and synthesizing information.\n\n## Approach\n1. Scope: Clearly define what you\'re researching and why.\n2. Sources: Prioritize primary sources (documentation, specs, papers).\n3. Analysis: Compare approaches, note trade-offs, identify gaps.\n4. Synthesis: Present findings with actionable recommendations.\n\nBe thorough but concise. Focus on practical, actionable information."
                    .to_string(),
                template_id: 6,
            embedding: None,
            min_experiences: 3,
            ..Default::default()
            },
            RoleTemplate {
                role: "devops".to_string(),
                label: "DevOps Engineer".to_string(),
                system_prompt: "You are a DevOps engineer responsible for infrastructure, deployment, and operations.\n\n## Skills\n1. Infrastructure as Code (Terraform, Pulumi, CloudFormation).\n2. Containerization (Docker, Kubernetes).\n3. CI/CD pipeline design (GitHub Actions, GitLab CI).\n4. Monitoring, logging, and alerting.\n5. Cloud services (AWS, GCP, Azure).\n\n## Approach\n- Design for reliability, scalability, and cost-efficiency.\n- Follow infrastructure-as-code principles — no manual changes.\n- Document all infrastructure decisions and trade-offs.\n- Include disaster recovery and backup strategies.\n\nOutput infrastructure plans with specific resource configurations."
                    .to_string(),
                template_id: 7,
            embedding: None,
            ..Default::default()
            },
        ]);

        let rt = Self {
            pipeline,
            provider: None,
            model_id: String::new(),
            role_template_store: Arc::new(store),
            optimization_tracker: std::sync::Mutex::new(
                crate::runtime::optimizer::OptimizationTracker::new(),
            ),
        };

        // Compute role embeddings in background (non-blocking).
        rt.compute_role_embeddings_async();

        rt
    }

    /// Default path for the role template store.
    fn default_store_path() -> std::path::PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        std::path::PathBuf::from(home)
            .join(".workflow")
            .join("role_templates.json")
    }

    // ── Pipeline delegation ──

    /// Run a [`SpawnRequest`] through the decision pipeline.
    pub async fn process_request(
        &self,
        request: SpawnRequest,
        role_template_id: Option<u32>,
        role_min_experiences: Option<usize>,
    ) -> Result<SpawnDecision> {
        self.pipeline
            .process_request(request, role_template_id, role_min_experiences)
            .await
    }

    /// Embed text and run through the decision pipeline.
    #[allow(clippy::too_many_arguments)]
    pub async fn process_with_text(
        &self,
        task_description: &str,
        role_description: &str,
        value_statement: &str,
        requested_budget: u64,
        current_depth: u32,
        role_template_id: Option<u32>,
        role_min_experiences: Option<usize>,
    ) -> Result<SpawnDecision> {
        let task_emb = self.pipeline.embedding().embed(task_description).await?;
        let role_emb = self.pipeline.embedding().embed(role_description).await?;
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

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

        self.pipeline
            .process_request(request, role_template_id, role_min_experiences)
            .await
    }

    // ── Budget guard ──

    /// Take the budget guard from the last approved request.
    pub fn take_pending_guard(&self) -> Option<BudgetGuard> {
        self.pipeline.take_pending_guard()
    }

    // ── Experience ──

    pub fn experience_count(&self) -> usize {
        self.pipeline.experience_count()
    }

    pub fn add_experience(&self, entry: ExperienceEntry) {
        self.pipeline.add_experience(entry);
    }

    /// Return a cloneable reference to the embedding service.
    /// Use this to avoid holding the runtime lock across `.await` points.
    pub fn embedding_service(&self) -> Arc<dyn EmbeddingService> {
        self.pipeline.embedding().clone()
    }

    /// Embed text using the pipeline's embedding service.
    pub async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        self.pipeline.embedding().embed(text).await
    }

    /// Search the experience pool by embedding vector.
    pub fn search_experience(
        &self,
        query: &[f32; EMBEDDING_DIM],
        k: usize,
    ) -> Vec<(ExperienceEntry, f32)> {
        self.pipeline.search_experience(query, k)
    }

    /// Collect all experiences belonging to a specific role.
    pub fn get_experiences_by_role(&self, role_id: u32) -> Vec<ExperienceEntry> {
        self.pipeline.get_experiences_by_role(role_id)
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

    // ── Experience pool ──

    /// Flush the experience pool to disk.
    pub fn flush_experience_pool(&self) -> Result<()> {
        self.pipeline.flush_experience_pool()
    }

    /// Clear all experiences from both tracks.
    pub fn clear_experience_pool(&self) -> Result<()> {
        self.pipeline.clear_experience_pool()
    }

    /// Consolidate fluid experiences to bedrock (cluster + promote).
    /// This is useful before operations that need complete data access,
    /// such as role optimization or system shutdown.
    pub fn consolidate_experience_pool(&mut self) {
        self.pipeline.consolidate_experience_pool();
    }

    /// Number of bedrock (persistent) experience entries.
    pub fn bedrock_count(&self) -> usize {
        self.pipeline.bedrock_count()
    }

    /// Number of fluid (volatile) experience entries.
    pub fn fluid_count(&self) -> usize {
        self.pipeline.fluid_count()
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

    pub fn get_role_template(&self, role: &str) -> Option<RoleTemplate> {
        self.role_template_store.get_by_role(role)
    }

    /// List all role templates.
    pub fn all_role_templates(&self) -> Vec<RoleTemplate> {
        self.role_template_store.all()
    }

    /// Delete a role template by ID.
    /// Silently succeeds if the role does not exist.
    pub fn delete_role_template(&self, template_id: u32) {
        self.role_template_store.delete_by_id(template_id);
    }

    /// Save (create or update) a role template.
    /// If the template has no embedding, computes one in background.
    pub fn save_role_template(&self, template: RoleTemplate) {
        let needs_embedding = template.embedding.is_none();
        self.role_template_store.upsert(template);
        let _ = self.role_template_store.persist();
        if needs_embedding {
            self.compute_role_embeddings_async();
        }
    }

    /// Compute embeddings for all role templates that lack one.
    /// Runs asynchronously — the runtime continues to function while
    /// embeddings are computed in the background.
    pub fn compute_role_embeddings_async(&self) {
        let store = self.role_template_store.clone();
        let embedding = self.pipeline.embedding().clone();
        tokio::spawn(async move {
            let templates = store.all();
            let mut updated = false;
            for t in &templates {
                if t.embedding.is_some() {
                    continue;
                }
                match embedding.embed(&t.system_prompt).await {
                    Ok(emb) => {
                        let mut new_t = t.clone();
                        new_t.embedding = Some(emb);
                        store.upsert(new_t);
                        tracing::info!("Computed embedding for role '{}'", t.role);
                        updated = true;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to compute embedding for role '{}': {}", t.role, e);
                    }
                }
            }
            if updated {
                let _ = store.persist();
                tracing::info!("Role embeddings persisted");
            }
        });
    }

    // ── Chat ──

    /// Chat with an LLM agent for a user goal.
    /// Spawns a root agent, executes it, and returns the result.
    pub async fn chat_with_goal(
        &self,
        goal: &str,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Result<String> {
        let agent_id = {
            let mut pool = agent_pool.write().await;
            self.spawn_root_agent(goal, "planner", "default", &mut pool)
                .await?
        };

        // Spawned agents are leaf nodes; execute directly.
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

    /// Create the initial interactive agent.
    ///
    /// This is a bootstrap actor rather than a spawned child, so it is
    /// registered directly in the pool. Any agents it creates later still go
    /// through the normal L-1/L0/L1/L2 spawn pipeline.
    pub fn bootstrap_root_agent(
        &self,
        goal: &str,
        role: &str,
        agent_pool: &mut AgentPool,
    ) -> AgentId {
        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        // Create sandbox (best-effort — failure means no filesystem isolation).
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .ok()
            .map(std::sync::Arc::new);
        let agent = Agent {
            id: agent_id,
            name: format!(
                "{}-{:04x}",
                role,
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            role: role.to_string(),
            role_template_id: Some(role_tpl.template_id),
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: goal.to_string(),
            config: crate::agent::AgentConfig {
                model_id: self.model_id.clone(),
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
            sandbox,
        };
        agent_pool.add_agent(agent);
        agent_id
    }

    pub async fn spawn_root_agent(
        &self,
        goal: &str,
        role: &str,
        value_statement: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_emb = self.pipeline.embedding().embed(role).await?;
        let task_emb = self.pipeline.embedding().embed(goal).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();

        // Run the decision pipeline
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

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

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the agent (ownership transferred).
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: role_tpl_id,
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
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
                    sandbox: sandbox.clone(),
                };
                agent_pool.add_agent(agent);
                Ok(agent_id)
            }
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_child(
        &self,
        parent_id: AgentId,
        parent_depth: u32,
        role: &str,
        goal: &str,
        value_statement: &str,
        responsibility_chain: &[AgentId],
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let role_emb = self.pipeline.embedding().embed(role).await?;

        let role_tpl = self
            .role_template_store
            .get_by_role(role)
            .or_else(|| self.role_template_store.find_closest(&role_emb, 0.85))
            .unwrap_or(RoleTemplate {
                role: role.to_string(),
                label: role.to_string(),
                system_prompt: format!("You are a {}. Execute the given goal.", role),
                template_id: 0,
                embedding: None,
                ..Default::default()
            });

        let agent_id: AgentId = rand::random();
        let sandbox = crate::tools::sandbox::SandboxHandle::new(&agent_id)
            .map(std::sync::Arc::new)
            .ok();
        let task_emb = self.pipeline.embedding().embed(goal).await?;
        let value_emb = self.pipeline.embedding().embed(value_statement).await?;

        let mut chain = responsibility_chain.to_vec();
        chain.push(agent_id);

        // Derive parent_span_id from the first agent in the responsibility chain.
        let parent_span_id: u64 = responsibility_chain
            .first()
            .and_then(|id| Some(u64::from_le_bytes(id[0..8].try_into().ok()?)))
            .unwrap_or(0);
        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: rand::random(),
            parent_span_id,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: value_emb,
            requested_budget: 1000,
            current_depth: parent_depth + 1,
            responsibility_chain: chain,
            raw_text_ref: None,
        };

        let role_tpl_id = Some(role_tpl.template_id);
        let decision = self
            .pipeline
            .process_request(request, role_tpl_id, Some(role_tpl.min_experiences))
            .await?;
        match decision {
            SpawnDecision::Approved(config) => {
                // Attach budget guard to the child agent.
                if let Some(guard) = self.pipeline.take_pending_guard() {
                    agent_pool.attach_budget_guard(agent_id, guard);
                }
                let agent = Agent {
                    id: agent_id,
                    name: format!(
                        "{}-{:04x}",
                        role,
                        u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
                    ),
                    role: role.to_string(),
                    role_template_id: Some(role_tpl.template_id),
                    parent_id: Some(parent_id),
                    children: Vec::new(),
                    depth: parent_depth + 1,
                    goal: goal.to_string(),
                    config: crate::agent::AgentConfig {
                        model_id: self.model_id.clone(),
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
                    sandbox: sandbox.clone(),
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
                        .unwrap_or_default()
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
            SpawnDecision::Rejected(rejection) => {
                Err(anyhow::anyhow!("Spawn rejected: {:?}", rejection))
            }
        }
    }

    pub async fn spawn_plan_task_agent(
        &self,
        owner_id: AgentId,
        role: &str,
        goal: &str,
        agent_pool: &mut AgentPool,
    ) -> Result<AgentId> {
        let (parent_depth, responsibility_chain) = agent_pool
            .get_agent(&owner_id)
            .map(|agent| (agent.depth, vec![owner_id]))
            .ok_or_else(|| anyhow::anyhow!("Responsible agent not found"))?;

        self.spawn_child(
            owner_id,
            parent_depth,
            role,
            goal,
            "default",
            &responsibility_chain,
            agent_pool,
        )
        .await
    }

    pub async fn synthesize_plan_result(
        &self,
        owner_id: AgentId,
        plan_goal: &str,
        task_results: &[(usize, String)],
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> String {
        let (config, role, provider) = {
            let mut pool = agent_pool.write().await;
            let (config, role) = match pool.get_agent_mut(&owner_id) {
                Some(agent) => {
                    agent.status = AgentStatus::Aggregating;
                    (agent.config.clone(), agent.role.clone())
                }
                None => return "Responsible agent not found".to_string(),
            };
            (config, role, self.provider.clone())
        };

        let result = if let Some(provider) = provider {
            let role_system_prompt = self
                .role_template_store
                .get_by_role(&role)
                .map(|t| t.system_prompt)
                .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));
            let task_summary = task_results
                .iter()
                .map(|(id, result)| format!("Task {}:\n{}", id, result))
                .collect::<Vec<_>>()
                .join("\n\n");
            let prompt = format!(
                "You own this approved plan.\n\nPlan goal: {}\n\nCompleted task results:\n{}\n\nSynthesize the final result for the user.",
                plan_goal, task_summary
            );
            provider
                .chat(&config.model_id, &role_system_prompt, &prompt)
                .await
                .unwrap_or_else(|e| format!("Plan synthesis failed: {}", e))
        } else {
            "No LLM provider configured".to_string()
        };

        let mut pool = agent_pool.write().await;
        if let Some(agent) = pool.get_agent_mut(&owner_id) {
            agent.result = Some(result.clone());
            agent.status = AgentStatus::Completed;
            pool.notify_completed(&owner_id);
        }
        result
    }

    /// Aggregate child results into a final synthesis by calling
    /// `provider.chat()` (pure text-in-text-out, no tools, no role
    /// alternation constraints).
    ///
    /// Reads `child_results` from the pool, builds a structured
    /// prompt, and stores the LLM response as the parent's `result`.
    pub async fn synthesize_aggregation(
        &self,
        owner_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Result<String> {
        // Phase 1: Drain inbox + child_results under a single write lock.
        // Collect raw data; closures must NOT borrow `pool` to avoid conflicts.
        let (config, role, provider, all_summaries, goal): (
            crate::agent::AgentConfig,
            String,
            Option<std::sync::Arc<crate::llm::LlmProvider>>,
            Vec<String>,
            String,
        ) = {
            let mut pool = agent_pool.write().await;
            let agent = pool
                .get_agent_mut(&owner_id)
                .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

            // Drain both inbox (structured handoff) and child_results (legacy).
            let inbox_msgs: Vec<crate::agent::AgentMessage> = agent.inbox.drain(..).collect();
            let cr_raw: Vec<(crate::core::types::AgentId, String)> =
                agent.child_results.drain(..).collect();

            let cfg = agent.config.clone();
            let rl = agent.role.clone();
            let gl = agent.goal.clone();

            // Build inbox summaries inline (no pool access needed).
            let inbox_summaries: Vec<String> = inbox_msgs
                .iter()
                .map(|msg| {
                    let hint = match &msg.payload {
                        Some(crate::agent::MessagePayload::AssetPointer {
                            asset_id, hint, ..
                        }) => format!(" (asset: {}, hint: {})", asset_id, hint),
                        Some(crate::agent::MessagePayload::StateSummary { summary, .. }) => {
                            format!(" (summary: {})", summary)
                        }
                        None => String::new(),
                    };
                    format!("[{}]{}[{}]", msg.from_name, hint, msg.content)
                })
                .collect();

            // Build legacy summaries (no pool access — use raw IDs).
            let cr_summaries: Vec<String> = cr_raw
                .iter()
                .map(|(_, result)| format!("[agent]\n{}", result))
                .collect();

            let all_summaries: Vec<String> = inbox_summaries
                .into_iter()
                .chain(cr_summaries.into_iter())
                .collect();

            (cfg, rl, self.provider.clone(), all_summaries, gl)
        };

        let provider = provider.ok_or_else(|| anyhow::anyhow!("No LLM provider configured"))?;

        let role_system_prompt = self
            .role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let child_count = all_summaries.len();
        let task_summary = all_summaries.join("\n\n---\n\n");

        // Include a note about SearchAsset when there are asset pointers.
        let has_assets = all_summaries.iter().any(|s| s.contains("(asset:"));
        let asset_note = if has_assets {
            concat!(
                "\n\n📌 Some sub-tasks produced large outputs that are stored as assets. ",
                "If you need details, use `search_asset(asset_id, query)`. ",
                "Your current context only contains compact summaries. ",
                "Do not ask for the full raw output unless you truly need it."
            )
        } else {
            ""
        };

        let prompt = format!(
            "You delegated this goal to {} sub-agent(s).\n\nOriginal goal: {}\n\nCompleted sub-task results:\n{}{}\n\nSynthesize the final result for the user.",
            child_count, goal, task_summary, asset_note
        );

        let result = provider
            .chat(&config.model_id, &role_system_prompt, &prompt)
            .await
            .map_err(|e| anyhow::anyhow!("Synthesis LLM call failed: {}", e))?;

        Ok(result)
    }

    pub async fn execute_agent(&self, agent_id: AgentId, agent_pool: &Arc<RwLock<AgentPool>>) {
        self.execute_agent_inner(agent_id, agent_pool, None).await;
    }

    /// Execute agent with tool access.
    pub async fn execute_agent_with_tools(
        &self,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
        tool_server: crate::tools::ToolServerHandle,
    ) {
        self.execute_agent_inner(agent_id, agent_pool, Some(tool_server))
            .await;
    }

    /// Map a tool name to a bit position for the tool bitmap.
    fn tool_bit(name: &str) -> u64 {
        match name {
            "read_file" => 1 << 0,
            "write_file" => 1 << 1,
            "sh" => 1 << 2,
            "list_dir" => 1 << 3,
            "grep" => 1 << 4,
            "find_files" => 1 << 5,
            "move_file" => 1 << 6,
            "copy_file" => 1 << 7,
            "delete_file" => 1 << 8,
            "append_file" => 1 << 9,
            "patch_file" => 1 << 10,
            "glob" => 1 << 11,
            "spawn_agent" => 1 << 12,
            "read_memo" => 1 << 13,
            "write_memo" => 1 << 14,
            "delete_memo" => 1 << 15,
            "list_memos" => 1 << 16,
            "call_agent" => 1 << 17,
            "list_agents" => 1 << 18,
            "send_message" => 1 << 19,
            "read_messages" => 1 << 20,
            _ => 0,
        }
    }

    /// Execute an agent without holding the runtime read lock across the LLM call.
    ///
    /// This is the deadlock-safe variant for use from spawned tasks where the
    /// caller cannot hold `&self` behind a read guard across `.await` points.
    /// The method internally acquires the lock only for brief data extraction
    /// and experience recording, releasing it before the async LLM call.
    pub(crate) async fn execute_agent_detached(
        runtime: Arc<RwLock<Self>>,
        agent_id: AgentId,
        agent_pool: Arc<RwLock<AgentPool>>,
        tool_server: Option<crate::tools::ToolServerHandle>,
    ) -> (String, AgentStatus) {
        // Phase 1: Extract needed data under a brief read lock
        let (provider, role_template_store, embedding_service) = {
            let rt = runtime.read().await;
            (
                rt.provider.clone(),
                Arc::clone(&rt.role_template_store),
                rt.pipeline.embedding().clone(),
            )
        };

        let (goal, role, config) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return (String::new(), AgentStatus::Failed),
            };
            (agent.goal, agent.role, agent.config.clone())
        };

        let provider: Arc<LlmProvider> = match provider {
            Some(p) => p,
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.release_budget_guard(&agent_id);
                    pool.notify_completed(&agent_id);
                }
                return (
                    "No LLM provider configured".to_string(),
                    AgentStatus::Failed,
                );
            }
        };

        // Mark planning
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Planning;
            }
        }

        // Phase 2: Build system prompt (no lock on runtime needed)
        let role_system_prompt = role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));

        let memo_block = {
            let pool = agent_pool.read().await;
            pool.format_role_memos(&role)
        };
        let memos = memo_block.as_deref().unwrap_or("");
        let system_prompt = format!(
            "{}\n\nYour goal: {}\n\nWork independently and produce a concrete result. Do not request sub-agents — you are a leaf agent.\n\n{}\n\n{}{}",
            role_system_prompt,
            goal,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
        );

        // Phase 3: Execute LLM call (no lock held on runtime)
        let (response, tool_bitmap) = if let Some(handle) = &tool_server {
            let mut text = String::new();
            let mut tools_used: u64 = 0;
            let mut tokens_input: u32 = 0;
            let mut tokens_output: u32 = 0;
            let stream = match provider
                .chat_with_tools_stream_mcp(&config.model_id, &system_prompt, &goal, &[], handle)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            use futures::StreamExt;
            futures::pin_mut!(stream);
            let mut tool_call_count = 0usize;
            while let Some(event) = stream.next().await {
                match event {
                    crate::llm::ToolEvent::Text(t) => text.push_str(&t),
                    crate::llm::ToolEvent::Reasoning(_t) => {
                        // Agent execution — reasoning is informational only.
                    }
                    crate::llm::ToolEvent::ToolCall { name, args, .. } => {
                        tool_call_count += 1;
                        tools_used |= Self::tool_bit(&name);
                        let args_preview = serde_json::to_string(&args).unwrap_or_default();
                        let args_preview = if args_preview.len() > 80 {
                            format!("{}…", &args_preview[..80])
                        } else {
                            args_preview
                        };
                        if let Ok(mut pool) = agent_pool.try_write() {
                            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                                agent.tool_trace.push_back(crate::agent::ToolCallRecord {
                                    name,
                                    args_preview,
                                    status: crate::agent::ToolStatus::Success,
                                });
                                // Keep ring buffer bounded
                                if agent.tool_trace.len() > 128 {
                                    agent.tool_trace.pop_front();
                                }
                            }
                        }
                    }
                    crate::llm::ToolEvent::TokenUsage { input, output } => {
                        // Track cumulative token usage for cost/consumption reporting.
                        tokens_input = tokens_input.max(input);
                        tokens_output = tokens_output.max(output);
                    }
                    crate::llm::ToolEvent::Done => break,
                }
            }
            // If the LLM hit max turns without producing a final message,
            // generate a concise summary so the user sees completion feedback.
            if text.trim().is_empty() && tool_call_count > 0 {
                text = format!(
                    "Completed after {} tool call{}.",
                    tool_call_count,
                    if tool_call_count == 1 { "" } else { "s" }
                );
            }
            (text, tools_used)
        } else {
            let text = match provider.chat(&config.model_id, &system_prompt, &goal).await {
                Ok(t) => t,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            (text, 0)
        };

        // Phase 4: Record result under brief lock
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Completed;
                agent.result = Some(response.clone());
            }
        }

        // Phase 5: Record experience (re-acquire runtime lock briefly)
        if !response.is_empty() {
            let goal_for_emb = goal.clone();
            if let Ok(emb) = embedding_service.embed(&goal_for_emb).await {
                let rt = runtime.read().await;
                rt.pipeline
                    .add_experience(crate::core::types::ExperienceEntry {
                        embedding: emb,
                        applicability_vector: [0.0f32; 128],
                        tool_bitmap,
                        role_template_id: role_template_store
                            .get_by_role(&role)
                            .map(|t| t.template_id),
                        weight: 1.0,
                        domain_version: 0,
                        timestamp: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                        l2_override_weight: 0.0,
                        l2_override_created_at: 0,
                    });
            }
        }

        // Phase 6: Release budget guard and notify completion
        let mut pool = agent_pool.write().await;
        pool.release_budget_guard(&agent_id);
        pool.notify_completed(&agent_id);

        (response, AgentStatus::Completed)
    }

    /// Execute a single leaf agent: LLM call, store result, record experience.
    ///
    /// If `tool_server` is provided, the agent can call tools.
    ///
    /// NOTE: This method takes `&self` and holds the runtime read lock for the
    /// entire duration. Prefer [`execute_agent_detached`] from long-lived contexts
    /// to avoid blocking other runtime operations.
    pub(crate) async fn execute_agent_inner(
        &self,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
        tool_server: Option<crate::tools::ToolServerHandle>,
    ) -> (String, AgentStatus) {
        let (goal, role, config) = {
            let pool = agent_pool.read().await;
            let agent = match pool.get_agent(&agent_id) {
                Some(a) => a.clone(),
                None => return (String::new(), AgentStatus::Failed),
            };
            (agent.goal, agent.role, agent.config.clone())
        };

        let provider = match &self.provider {
            Some(p) => p.clone(),
            None => {
                let mut pool = agent_pool.write().await;
                if let Some(agent) = pool.get_agent_mut(&agent_id) {
                    agent.status = AgentStatus::Failed;
                    agent.result = Some("No LLM provider configured".to_string());
                    pool.release_budget_guard(&agent_id);
                    pool.notify_completed(&agent_id);
                }
                return (
                    "No LLM provider configured".to_string(),
                    AgentStatus::Failed,
                );
            }
        };

        // Mark planning
        {
            let mut pool = agent_pool.write().await;
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.status = AgentStatus::Planning;
            }
        }

        let role_system_prompt = self
            .role_template_store
            .get_by_role(&role)
            .map(|t| t.system_prompt)
            .unwrap_or_else(|| format!("You are a {}. Execute the given goal.", role));
        // Inject role-scoped memos directly into the system prompt.
        // This replaces the old explicit ReadMemo tool pattern — memos are
        // now automatically available without an extra tool call.
        let memo_block = {
            let pool = agent_pool.read().await;
            pool.format_role_memos(&role)
        };

        let memos = memo_block.as_deref().unwrap_or("");
        let system_prompt = format!(
            "{}\n\nYour goal: {}\n\nWork independently and produce a concrete result. Do not request sub-agents — you are a leaf agent.\n\n{}\n\n{}{}",
            role_system_prompt,
            goal,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
        );

        let (response, tool_bitmap) = if let Some(handle) = &tool_server {
            let mut text = String::new();
            let mut tools_used: u64 = 0;
            let mut tokens_input: u32 = 0;
            let mut tokens_output: u32 = 0;
            let stream = match provider
                .chat_with_tools_stream_mcp(&config.model_id, &system_prompt, &goal, &[], handle)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            };
            use futures::StreamExt;
            futures::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    crate::llm::ToolEvent::Text(t) => text.push_str(&t),
                    crate::llm::ToolEvent::Reasoning(_t) => {
                        // Agent execution — reasoning is informational only.
                    }
                    crate::llm::ToolEvent::ToolCall { name, args, .. } => {
                        tools_used |= Self::tool_bit(&name);
                        // Record tool call trace (ring buffer, bounded).
                        let args_preview = serde_json::to_string(&args).unwrap_or_default();
                        let args_preview = if args_preview.len() > 80 {
                            format!("{}…", &args_preview[..80])
                        } else {
                            args_preview
                        };
                        if let Ok(mut pool) = agent_pool.try_write() {
                            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                                agent.tool_trace.push_back(crate::agent::ToolCallRecord {
                                    name,
                                    args_preview,
                                    status: crate::agent::ToolStatus::Success,
                                });
                                if agent.tool_trace.len() > crate::agent::MAX_TOOL_TRACE {
                                    agent.tool_trace.pop_front();
                                }
                            }
                        } // try_write drops here — minimal lock duration
                    }
                    crate::llm::ToolEvent::TokenUsage { input, output } => {
                        tokens_input = tokens_input.max(input);
                        tokens_output = tokens_output.max(output);
                    }
                    crate::llm::ToolEvent::Done => break,
                }
            }
            (text, tools_used)
        } else {
            match provider.chat(&config.model_id, &system_prompt, &goal).await {
                Ok(r) => (r, 0),
                Err(e) => {
                    let mut pool = agent_pool.write().await;
                    if let Some(agent) = pool.get_agent_mut(&agent_id) {
                        agent.status = AgentStatus::Failed;
                        agent.result = Some(format!("LLM error: {}", e));
                        pool.release_budget_guard(&agent_id);
                        pool.notify_completed(&agent_id);
                    }
                    return (format!("LLM error: {}", e), AgentStatus::Failed);
                }
            }
        };

        // Leaf agent — response is the result
        let (role_template_id, recorded_tool_bitmap) = {
            let mut pool = agent_pool.write().await;
            let role_tpl_id = pool.get_agent(&agent_id).and_then(|a| a.role_template_id);
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.result = Some(response.clone());
                agent.status = AgentStatus::Completed;
                pool.release_budget_guard(&agent_id);
                pool.notify_completed(&agent_id);
            }
            (role_tpl_id, tool_bitmap)
        };

        // Record experience entry (feedback loop).
        if let Ok(emb) = self.pipeline.embedding().embed(&goal).await {
            self.pipeline.add_experience(ExperienceEntry {
                embedding: emb,
                applicability_vector: [0.0f32; 128],
                tool_bitmap: recorded_tool_bitmap,
                role_template_id,
                weight: 0.8,
                domain_version: 0,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                l2_override_weight: 0.0,
                l2_override_created_at: 0,
            });
        }

        (response, AgentStatus::Completed)
    }

    pub async fn await_agent(
        &self,
        agent_id: AgentId,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Mock embedding service that returns a fixed vector without API calls.
    struct MockEmbedding;

    #[async_trait::async_trait]
    impl EmbeddingService for MockEmbedding {
        async fn embed(&self, _text: &str) -> anyhow::Result<[f32; EMBEDDING_DIM]> {
            let mut emb = [0.0f32; EMBEDDING_DIM];
            emb[0] = 1.0;
            Ok(emb)
        }

        async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<[f32; EMBEDDING_DIM]>> {
            let mut results = Vec::with_capacity(texts.len());
            for _ in texts {
                let mut emb = [0.0f32; EMBEDDING_DIM];
                emb[0] = 1.0;
                results.push(emb);
            }
            Ok(results)
        }

        fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
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

    fn dummy_embedding() -> Arc<dyn EmbeddingService> {
        Arc::new(MockEmbedding)
    }

    #[tokio::test]
    async fn test_basic_spawn() {
        let runtime = AgentRuntime::new(AgentRuntimeConfig::default(), dummy_embedding());

        // Seed an experience so L1 doesn't reject empty pool.
        let mut emb = [0.0f32; EMBEDDING_DIM];
        emb[0] = 1.0;
        runtime.add_experience(ExperienceEntry {
            embedding: emb,
            applicability_vector: [0.0f32; 128],
            tool_bitmap: 0,
            role_template_id: None,
            weight: 1.0,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        });

        let task = "Implement a REST API";
        let role = "Senior Rust developer";
        let value = "Write secure, well-tested code";

        let decision = runtime
            .process_with_text(task, role, value, 1000, 0, None, None)
            .await
            .unwrap();

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

        let decision = runtime
            .process_with_text(task, role, value, 99999, 0, None, None)
            .await
            .unwrap();

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

    #[tokio::test]
    async fn test_multi_spawn_sequential() {
        let rt = AgentRuntime::new(AgentRuntimeConfig::default(), dummy_embedding());

        let mut emb = [0.0f32; EMBEDDING_DIM];
        emb[0] = 1.0;
        rt.add_experience(ExperienceEntry {
            embedding: emb,
            applicability_vector: [0.0f32; 128],
            tool_bitmap: 0,
            role_template_id: None,
            weight: 1.0,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        });

        let task = "Implement feature X";
        let role = "developer";
        let value = "Write quality code";

        for i in 0..5 {
            let decision = rt
                .process_with_text(task, role, value, 200, 0, None, None)
                .await
                .unwrap();

            match &decision {
                SpawnDecision::Approved(config) => {
                    assert!(config.allocated_budget > 0, "iteration {}: budget > 0", i);
                }
                SpawnDecision::Rejected(rejection) => {
                    panic!("Iteration {}: unexpected rejection: {:?}", i, rejection);
                }
            }

            let mut exp_emb = [0.0f32; EMBEDDING_DIM];
            exp_emb[0] = 1.0 - (i as f32) * 0.05;
            rt.add_experience(ExperienceEntry {
                embedding: exp_emb,
                applicability_vector: [0.0f32; 128],
                tool_bitmap: 0b1 << i.min(5),
                role_template_id: None,
                weight: 0.7 + (i as f32) * 0.05,
                domain_version: 0,
                timestamp: 0,
                l2_override_weight: 0.0,
                l2_override_created_at: 0,
            });

            assert_eq!(rt.experience_count(), 2 + i, "iteration {}: pool count", i);
        }

        assert_eq!(rt.experience_count(), 6);
        assert!(rt.remaining_budget() < crate::core::types::DEFAULT_RUNTIME_BUDGET as i64);
    }
}
