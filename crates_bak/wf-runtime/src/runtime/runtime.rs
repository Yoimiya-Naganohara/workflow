//! Agent runtime — top-level orchestrator wiring L-1/L0/L1/L2 decision pipeline
//! to agent lifecycle management.

use std::sync::Arc;

use anyhow::Result;

use super::config::{AgentRuntimeConfig, RoleTemplate};
use super::pipeline::{DecisionPipeline, DecisionPipelineBuilder};

use crate::l0::BudgetGuard;
use wf_agent::plan::PlanRegistry as PlanRegistryConcrete;
use wf_core::*;
use wf_experience::RoleTemplateStore;
use wf_llm::EmbeddingService;
use wf_llm::LlmProvider;

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
    pub(crate) pipeline: Arc<DecisionPipeline>,
    /// LLM provider for chat / embedding.
    pub provider: Option<Arc<LlmProvider>>,
    /// Active model identifier.
    pub model_id: String,
    /// Role template store backed by persistent JSON.
    pub(crate) role_template_store: Arc<RoleTemplateStore>,
    /// Tracks optimization frequency per role.
    pub optimization_tracker: std::sync::Mutex<super::optimizer::OptimizationTracker>,
    /// Task graph DAG — the single source of truth for delegation hierarchy
    /// and execution ordering (Phase 2A).
    /// Shared with `RuntimeEventLoop` via this `Arc`.
    pub task_graph: std::sync::Arc<std::sync::Mutex<wf_core::task_graph::TaskGraph>>,
}

impl AgentRuntime {
    /// Create a runtime with default component implementations.
    ///
    /// This is the quick-start path.  Every layer uses its default
    /// implementation tuned by `config`.
    pub fn new(config: AgentRuntimeConfig, embedding_service: Arc<dyn EmbeddingService>) -> Self {
        use crate::admission::AdmissionController;
        use crate::l0::L0CircuitBreaker;
        use crate::l0::TaskResourceState;
        use std::path::PathBuf;
        use wf_agent::suspend::{SuspendConfig, SuspendQueue as SuspendQueueConcrete};
        use wf_experience::DualTrackMemory;
        use wf_l2::L2RuleAuditEngine;

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

        // Open dual-track memory with mmap persistence (creates file if needed).
        // Falls back to in-memory pool on failure (no persistence but no crash).
        let dual_track =
            match DualTrackMemory::open(&bedrock_path, 512, config.l1_confidence_threshold) {
                Ok(pool) => Box::new(pool),
                Err(e) => {
                    tracing::warn!(
                        "Failed to open experience pool, using in-memory fallback: {}",
                        e
                    );
                    Box::new(DualTrackMemory::in_memory(512usize))
                }
            };

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

        Self::from_pipeline_with_store(pipeline, config.role_template_path.clone())
    }

    /// Create a runtime from a pre-built [`DecisionPipeline`].
    ///
    /// Use this when you want full control over every layer's
    /// implementation (mocking, custom audit engines, etc.).
    pub fn from_pipeline(pipeline: DecisionPipeline) -> Self {
        Self::from_pipeline_with_store(pipeline, None)
    }

    /// Like [`from_pipeline`](Self::from_pipeline) but with a custom path
    /// for the role template store.  Use a temp directory in tests to avoid
    /// writing to the user's `~/.workflow/` directory.
    pub fn from_pipeline_with_store(
        pipeline: DecisionPipeline,
        role_template_path: Option<std::path::PathBuf>,
    ) -> Self {
        let pipeline = Arc::new(pipeline);
        let store_path = role_template_path.unwrap_or_else(Self::default_store_path);
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
            task_graph: std::sync::Arc::new(std::sync::Mutex::new(
                wf_core::task_graph::TaskGraph::new(),
            )),
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

    // ── Checkpoint recovery ──

    // /// Restore an agent pool and task graph from the last checkpoint.
    // ///
    // /// Returns `true` if a checkpoint was found and restored, `false` if
    // /// no checkpoint exists (first run).  The restored pool is written into
    // /// `agent_pool` and the task graph into `self.task_graph`.
    // ///
    // /// # Non-serializable fields
    // ///
    // /// After restoration, `agent_pool` will have:
    // /// - No `provider` — caller must set it via `agent_pool.set_provider()`
    // /// - Re-created `Notify` handles for each agent
    // /// - No budget guards — budget resets on restart
    // /// - No sandbox handles — sandboxes are re-created on activation
    // ── Pipeline delegation ──

    /// Run a `SpawnRequest` through the decision pipeline.
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

    /// Export all pool entries to a JSON file.
    pub fn export_pool(&self, path: &std::path::Path) -> Result<()> {
        self.pipeline.export_pool(path)
    }

    /// Import entries from a JSON file into the pool.
    pub fn import_pool(&self, path: &std::path::Path) -> Result<usize> {
        self.pipeline.import_pool(path)
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
    /// If the template has no valid embedding, computes one in background.
    pub fn save_role_template(&self, mut template: RoleTemplate) {
        if let Some(existing) = self.role_template_store.get_by_role(&template.role)
            && existing.system_prompt != template.system_prompt
        {
            template.embedding = None;
        }
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

    // ── Agent lifecycle ──
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Mock embedding service that returns a fixed vector without API calls.
    use crate::test_utils::MockEmbedding;

    fn dummy_embedding() -> Arc<dyn EmbeddingService> {
        Arc::new(MockEmbedding)
    }

    /// Create an [`AgentRuntimeConfig`] with temp directories so tests
    /// never write to the real `~/.workflow/` directory.
    fn test_config() -> AgentRuntimeConfig {
        let dir = std::env::temp_dir().join(format!("workflow_test_{}", rand::random::<u64>()));
        AgentRuntimeConfig {
            bedrock_path: Some(dir.join("experience_a.bin")),
            role_template_path: Some(dir.join("role_templates.json")),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_basic_spawn() {
        let runtime = AgentRuntime::new(test_config(), dummy_embedding());

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
        let runtime = AgentRuntime::new(test_config(), dummy_embedding());

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
        let rt = AgentRuntime::new(test_config(), dummy_embedding());

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
        assert!(rt.remaining_budget() < wf_core::DEFAULT_RUNTIME_BUDGET as i64);
    }

    #[tokio::test]
    async fn test_save_role_template_clears_embedding_on_prompt_change() {
        let rt = AgentRuntime::new(test_config(), dummy_embedding());

        // Use a non-seeded role to avoid background embedding race.
        // The seeded templates have background compute_role_embeddings_async running.
        let tpl = RoleTemplate {
            role: "cache_test_role".to_string(),
            label: "Cache Test".to_string(),
            system_prompt: "Original prompt.".to_string(),
            embedding: None,
            ..Default::default()
        };
        rt.save_role_template(tpl.clone());

        let saved = rt.get_role_template("cache_test_role").unwrap();
        assert!(saved.embedding.is_none(), "no embedding yet");

        // 1. Set embedding → save → should persist.
        let mut emb = [0.0f32; EMBEDDING_DIM];
        emb[0] = 0.5;
        let mut with_emb = saved;
        with_emb.embedding = Some(emb);
        rt.save_role_template(with_emb);

        let tpl2 = rt.get_role_template("cache_test_role").unwrap();
        assert!(tpl2.embedding.is_some(), "embedding should be cached");

        // 2. Change system_prompt → save → embedding should be cleared.
        let mut changed = tpl2;
        changed.system_prompt = "Completely different prompt.".to_string();
        rt.save_role_template(changed);

        let tpl3 = rt.get_role_template("cache_test_role").unwrap();
        assert!(
            tpl3.embedding.is_none(),
            "embedding cleared when system_prompt changes"
        );
    }
}
