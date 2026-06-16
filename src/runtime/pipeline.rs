//! Decision pipeline orchestrating the L-1 → L0 → L1 → L2 flow.
//!
//! This module owns the core "presumed guilty" decision loop and
//! is injected into [`AgentRuntime`](crate::runtime::AgentRuntime)
//! via dependency inversion — every dependency is a trait object.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::admission::AdmissionControl;
use crate::admission::AdmissionPermit;
use crate::agent::plan::PlanRegistry;
use crate::agent::suspend::SuspendQueue;
use crate::core::types::*;
use crate::l0::BudgetGuard;
use crate::l0::CircuitBreaker;
use crate::l0::L0Permit;
use crate::l1::ExperienceRetrieval;
use crate::l1::L1Assessment;
use crate::l2::AuditEngine;
use crate::l2::llm::{L2LlmAuditEngine, L2LlmConfig};
use crate::llm::EmbeddingService;

// ============================================================================
//  Builder
// ============================================================================

/// Builder for [`DecisionPipeline`] with sensible defaults and
/// full DI support.
#[derive(Default)]
pub struct DecisionPipelineBuilder {
    admission: Option<Box<dyn AdmissionControl>>,
    circuit_breaker: Option<Box<dyn CircuitBreaker>>,
    experience: Option<Box<dyn ExperienceRetrieval>>,
    audit_engine: Option<Box<dyn AuditEngine>>,
    embedding: Option<Arc<dyn EmbeddingService>>,
    suspend: Option<Box<SuspendQueue>>,
    plans: Option<Box<PlanRegistry>>,
}

impl DecisionPipelineBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    builder_method!(admission, Box<dyn AdmissionControl>);
    builder_method!(circuit_breaker, Box<dyn CircuitBreaker>);
    builder_method!(experience, Box<dyn ExperienceRetrieval>);
    builder_method!(audit_engine, Box<dyn AuditEngine>);
    builder_method!(embedding, Arc<dyn EmbeddingService>);
    builder_method!(suspend, Box<SuspendQueue>);
    builder_method!(plans, Box<PlanRegistry>);

    /// Use the LLM-powered audit engine instead of the default rule engine.
    ///
    /// This creates an [`L2LlmAuditEngine`] that uses a language model
    /// judge to review conflicts and screen requests.
    pub fn llm_audit_engine(mut self, provider: Arc<crate::llm::LlmProvider>, config: L2LlmConfig) -> Self {
        self.audit_engine = Some(Box::new(L2LlmAuditEngine::new(provider, config)));
        self
    }

    /// Build the pipeline, using defaults for any unset dependencies.
    pub fn build(self) -> DecisionPipeline {
        DecisionPipeline {
            admission: self.admission.unwrap_or_else(|| {
                Box::new(crate::admission::AdmissionController::new(
                    crate::core::types::DEFAULT_MAX_AGENTS,
                    crate::core::types::DEFAULT_ADMISSION_TIMEOUT_MS,
                ))
            }),
            circuit_breaker: self.circuit_breaker.unwrap_or_else(|| {
                let state = crate::l0::TaskResourceState::new(
                    crate::core::types::DEFAULT_RUNTIME_BUDGET,
                    crate::core::types::DEFAULT_MAX_DEPTH,
                );
                Box::new(crate::l0::L0CircuitBreaker::new(state))
            }),
            experience: Mutex::new(
                self.experience.unwrap_or_else(|| {
                    Box::new(crate::l1::L1Retriever::new(crate::core::types::DEFAULT_L1_CONFIDENCE))
                }),
            ),
            audit_engine: Mutex::new(self.audit_engine.unwrap_or_else(|| {
                Box::new(crate::l2::L2RuleAuditEngine::new(
                    crate::core::types::MAX_CONSECUTIVE_FAILURES,
                ))
            })),
            embedding: self
                .embedding
                .unwrap_or_else(|| panic!("DecisionPipelineBuilder: embedding is required")),
            suspend: Mutex::new(self.suspend.unwrap_or_else(|| {
                Box::new(crate::agent::suspend::SuspendQueue::new(
                    crate::agent::suspend::SuspendConfig {
                        hard_timeout_ms: crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS,
                        dynamic_timeout_ms: crate::core::types::DEFAULT_SUSPEND_TIMEOUT_MS,
                    },
                ))
            })),
            plans: Arc::new(RwLock::new(
                self.plans
                    .unwrap_or_else(|| Box::new(crate::agent::plan::PlanRegistry::new())),
            )),
            pending_guard: Mutex::new(None),
        }
    }
}

// ============================================================================
//  Decision Pipeline
// ============================================================================

/// The core L-1 → L0 → L1 → L2 decision pipeline.
///
/// All dependencies are injected via trait objects — every layer
/// (admission, circuit breaker, experience retrieval, audit
/// engine) can be swapped independently.
pub struct DecisionPipeline {
    admission: Box<dyn AdmissionControl>,
    circuit_breaker: Box<dyn CircuitBreaker>,
    experience: Mutex<Box<dyn ExperienceRetrieval>>,
    audit_engine: Mutex<Box<dyn AuditEngine>>,
    embedding: Arc<dyn EmbeddingService>,
    suspend: Mutex<Box<SuspendQueue>>,
    plans: Arc<RwLock<Box<PlanRegistry>>>,
    /// Budget guard from the last approved request, if any.
    pending_guard: Mutex<Option<BudgetGuard>>,
}

impl DecisionPipeline {
    /// Run a [`SpawnRequest`] through the full pipeline.
    pub async fn process_request(
        &self,
        request: SpawnRequest,
        role_template_id: Option<u32>,
        role_min_experiences: Option<usize>,
    ) -> Result<SpawnDecision> {
        // ── L-1: Admission ──
        let _permit: AdmissionPermit = self
            .admission
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Admission failed: {:?}", e))?;

        // ── L0: Circuit breaker (budget, depth, tools) ──
        let l0_result = self
            .circuit_breaker
            .try_acquire(request.requested_budget, request.current_depth, 0);

        let _l0_permit: L0Permit = match l0_result {
            Ok(permit) => permit,
            Err(rejection) => {
                if matches!(rejection, SpawnRejection::ResourceConflict { .. }) {
                    let priority = self.circuit_breaker.calculate_priority(
                        self.circuit_breaker.remaining_budget(),
                        request.requested_budget,
                        request.current_depth,
                    );
                    self.suspend
                        .lock()
                        .expect("suspend mutex poisoned")
                        .enqueue(request, priority);
                }
                return Ok(SpawnDecision::Rejected(rejection));
            }
        };

        let task_emb = &request.task_description_embedding;
        let role_emb = &request.role_description_embedding;

        // ── L1: Experience retrieval & confidence check ──
        let _l1_assessment: L1Assessment = {
            let exp = self.experience.lock().expect("experience mutex poisoned");
            exp.check_confidence(task_emb, role_emb, role_template_id, role_min_experiences)?
        };

        // ── L2: Screen request before final approval (sync, no .await) ──
        if let Some(rejection) = self
            .audit_engine
            .lock()
            .expect("audit_engine poisoned")
            .screen_request(&request)
        {
            return Ok(SpawnDecision::Rejected(rejection));
        }

        let agent_id: AgentId = rand::random();
        let task_id: TaskId = rand::random();
        let allocated_budget = _l0_permit.budget_amount();

        // Consume the L0 permit into a BudgetGuard (resource ownership
        // transfers to the guard; permit's Drop becomes a no-op).
        let guard = _l0_permit.into_budget_guard(task_id);
        if guard.is_none() {
            // Should never happen: the permit was just acquired.
            return Ok(SpawnDecision::Rejected(SpawnRejection::SystemOverloaded));
        }
        {
            let mut slot = self.pending_guard.lock().expect("pending_guard poisoned");
            *slot = guard;
        }

        // Every agent has complete access to all tools.
        let allowed_tools = !0u64;

        Ok(SpawnDecision::Approved(ChildAgentConfig {
            agent_id,
            task_id,
            allocated_budget,
            allowed_tools,
            role_template_id: None,
        }))
    }

    // ── Accessors ──

    pub fn embedding(&self) -> &Arc<dyn EmbeddingService> {
        &self.embedding
    }

    pub fn plans(&self) -> &Arc<RwLock<Box<PlanRegistry>>> {
        &self.plans
    }

    pub fn add_experience(&self, entry: ExperienceEntry) {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .add_experience(entry);
    }

    pub fn experience_count(&self) -> usize {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .experience_count()
    }

    pub fn flush_experience_pool(&self) -> Result<()> {
        self.experience.lock().expect("experience mutex poisoned").flush()
    }

    pub fn clear_experience_pool(&self) -> Result<()> {
        self.experience.lock().expect("experience mutex poisoned").clear()
    }

    /// Consolidate fluid experiences to bedrock (cluster + promote).
    pub fn consolidate_experience_pool(&self) {
        self.experience.lock().expect("experience mutex poisoned").consolidate();
    }

    pub fn bedrock_count(&self) -> usize {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .bedrock_count()
    }

    pub fn fluid_count(&self) -> usize {
        self.experience.lock().expect("experience mutex poisoned").fluid_count()
    }

    pub fn pending_suspended(&self) -> usize {
        self.suspend.lock().expect("suspend mutex poisoned").len()
    }

    pub fn available_permits(&self) -> usize {
        self.admission.available_permits()
    }

    pub fn remaining_budget(&self) -> i64 {
        self.circuit_breaker.remaining_budget()
    }

    pub fn audit_engine(&self) -> &Mutex<Box<dyn AuditEngine>> {
        &self.audit_engine
    }

    /// Take the budget guard from the last approved request.
    pub fn take_pending_guard(&self) -> Option<BudgetGuard> {
        self.pending_guard.lock().expect("pending_guard poisoned").take()
    }

    /// Search the experience pool by text query.
    pub fn search_experience(&self, query: &[f32; EMBEDDING_DIM], k: usize) -> Vec<(ExperienceEntry, f32)> {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .retrieve(query, k)
    }

    /// Collect all experiences belonging to a specific role.
    pub fn get_experiences_by_role(&self, role_id: u32) -> Vec<ExperienceEntry> {
        self.experience
            .lock()
            .expect("experience mutex poisoned")
            .get_experiences_by_role(role_id)
    }
}

// ============================================================================
//  Macro: builder_method
// ============================================================================

macro_rules! builder_method {
    ($field:ident, $ty:ty) => {
        pub fn $field(mut self, val: $ty) -> Self {
            self.$field = Some(val);
            self
        }
    };
}

pub(crate) use builder_method;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::embedding::EmbeddingService as EmbeddingServiceImpl;
    use std::sync::Arc;

    fn dummy_embedding() -> Arc<dyn EmbeddingService> {
        Arc::new(EmbeddingServiceImpl::new())
    }

    #[tokio::test]
    async fn test_pipeline_approves_valid_request() {
        let pipeline = DecisionPipelineBuilder::new().embedding(dummy_embedding()).build();

        // Add a matching experience so L1 does not reject (empty pool = presumed guilty).
        let mut exp_emb = [0.0f32; EMBEDDING_DIM];
        exp_emb[0] = 1.0;
        pipeline.add_experience(ExperienceEntry {
            embedding: exp_emb,
            applicability_vector: [0.0f32; 128],
            tool_bitmap: 0b101,
            role_template_id: None,
            weight: 1.0,
            domain_version: 0,
            timestamp: 0,
            l2_override_weight: 0.0,
            l2_override_created_at: 0,
        });

        let mut task_emb = [0.0f32; EMBEDDING_DIM];
        task_emb[0] = 1.0;
        let mut role_emb = [0.0f32; EMBEDDING_DIM];
        role_emb[0] = 1.0;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: 1,
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: [0.0f32; EMBEDDING_DIM],
            requested_budget: 100,
            current_depth: 0,
            responsibility_chain: vec![],
            raw_text_ref: None,
        };

        let decision = pipeline.process_request(request, None, None).await.unwrap();
        assert!(matches!(decision, SpawnDecision::Approved(_)));
    }

    #[tokio::test]
    async fn test_pipeline_rejects_budget_exhausted() {
        let state = crate::l0::TaskResourceState::new(50, 10);
        let breaker = Box::new(crate::l0::L0CircuitBreaker::new(state));

        let pipeline = DecisionPipelineBuilder::new()
            .embedding(dummy_embedding())
            .circuit_breaker(breaker)
            .build();

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: 1,
            parent_span_id: 0,
            task_description_embedding: [0.0f32; EMBEDDING_DIM],
            role_description_embedding: [0.0f32; EMBEDDING_DIM],
            value_statement_embedding: [0.0f32; EMBEDDING_DIM],
            requested_budget: 100,
            current_depth: 0,
            responsibility_chain: vec![],
            raw_text_ref: None,
        };

        let decision = pipeline.process_request(request, None, None).await.unwrap();
        assert!(matches!(
            decision,
            SpawnDecision::Rejected(SpawnRejection::BudgetExhausted { .. })
        ));
    }
}
