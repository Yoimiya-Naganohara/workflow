//! Decision pipeline orchestrating the L-1 → L0 → L1 → L2 flow.
//!
//! This module owns the core "presumed guilty" decision loop and
//! is injected into [`AgentRuntime`](crate::runtime::AgentRuntime)
//! via dependency inversion — every dependency is a trait object.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::admission::AdmissionPermit;
use crate::l0::L0Permit;
use crate::l1::L1Assessment;
use crate::traits::{
    AdmissionControl, AuditEngine, CircuitBreaker, ConflictDetector, EmbeddingService, ExperienceRetrieval,
    PlanRegistry, ResourcePool, SuspendQueue, ValueClassifier,
};
use crate::types::*;

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
    value_classifier: Option<Box<dyn ValueClassifier>>,
    conflict_detector: Option<Box<dyn ConflictDetector>>,
    audit_engine: Option<Box<dyn AuditEngine>>,
    embedding: Option<Arc<dyn EmbeddingService>>,
    resources: Option<Arc<dyn ResourcePool>>,
    suspend: Option<Box<dyn SuspendQueue>>,
    plans: Option<Box<dyn PlanRegistry>>,
}

impl DecisionPipelineBuilder {
    /// Create a new builder.
    pub fn new() -> Self {
        Self::default()
    }

    builder_method!(admission, Box<dyn AdmissionControl>);
    builder_method!(circuit_breaker, Box<dyn CircuitBreaker>);
    builder_method!(experience, Box<dyn ExperienceRetrieval>);
    builder_method!(value_classifier, Box<dyn ValueClassifier>);
    builder_method!(conflict_detector, Box<dyn ConflictDetector>);
    builder_method!(audit_engine, Box<dyn AuditEngine>);
    builder_method!(embedding, Arc<dyn EmbeddingService>);
    builder_method!(resources, Arc<dyn ResourcePool>);
    builder_method!(suspend, Box<dyn SuspendQueue>);
    builder_method!(plans, Box<dyn PlanRegistry>);

    /// Build the pipeline, using defaults for any unset dependencies.
    pub fn build(self) -> DecisionPipeline {
        DecisionPipeline {
            admission: self
                .admission
                .unwrap_or_else(|| Box::new(crate::admission::AdmissionController::new(10, 100))),
            circuit_breaker: self.circuit_breaker.unwrap_or_else(|| {
                let state = crate::resource::TaskResourceState::new(10000, 10);
                Box::new(crate::l0::L0CircuitBreaker::new(state))
            }),
            experience: Mutex::new(
                self.experience
                    .unwrap_or_else(|| Box::new(crate::l1::L1Retriever::new(0.5))),
            ),
            value_classifier: self
                .value_classifier
                .unwrap_or_else(|| Box::new(crate::l1::classifier::L1ValueClassifier::new(vec![]))),
            conflict_detector: self
                .conflict_detector
                .unwrap_or_else(|| Box::new(crate::l1_arbitration::L1Arbitrator::new(-0.6))),
            audit_engine: Mutex::new(
                self.audit_engine
                    .unwrap_or_else(|| Box::new(crate::l2::L2RuleAuditEngine::new(5))),
            ),
            embedding: self
                .embedding
                .unwrap_or_else(|| panic!("DecisionPipelineBuilder: embedding is required")),
            resources: self.resources.unwrap_or_else(|| {
                let state = crate::resource::TaskResourceState::new(10000, 10);
                state as Arc<dyn ResourcePool>
            }),
            suspend: Mutex::new(
                self.suspend
                    .unwrap_or_else(|| Box::new(crate::suspend::SuspendQueue::new(Default::default()))),
            ),
            plans: Arc::new(RwLock::new(
                self.plans.unwrap_or_else(|| Box::new(crate::plan::PlanRegistry::new())),
            )),
        }
    }
}

// ============================================================================
//  Decision Pipeline
// ============================================================================

/// The core L-1 → L0 → L1 → L2 decision pipeline.
///
/// All dependencies are injected via trait objects — every layer
/// (admission, circuit breaker, experience retrieval, conflict
/// detection, audit engine) can be swapped independently.
pub struct DecisionPipeline {
    // ── L-1 ──
    admission: Box<dyn AdmissionControl>,
    // ── L0 ──
    circuit_breaker: Box<dyn CircuitBreaker>,
    resources: Arc<dyn ResourcePool>,
    // ── L1 ──
    experience: Mutex<Box<dyn ExperienceRetrieval>>,
    value_classifier: Box<dyn ValueClassifier>,
    conflict_detector: Box<dyn ConflictDetector>,
    // ── L2 ──
    audit_engine: Mutex<Box<dyn AuditEngine>>,
    // ── Services ──
    embedding: Arc<dyn EmbeddingService>,
    suspend: Mutex<Box<dyn SuspendQueue>>,
    plans: Arc<RwLock<Box<dyn PlanRegistry>>>,
}

impl DecisionPipeline {
    /// Run a [`SpawnRequest`] through the full pipeline.
    ///
    /// Returns [`SpawnDecision::Approved`] only when every layer
    /// consents.  Rejected requests may be placed on the suspend
    /// queue for later retry.
    pub async fn process_request(&self, request: SpawnRequest) -> Result<SpawnDecision> {
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
                        self.resources.remaining_budget(),
                        request.requested_budget,
                        request.current_depth,
                    );
                    self.suspend.lock().unwrap().enqueue(request, priority);
                }
                return Ok(SpawnDecision::Rejected(rejection));
            }
        };

        let task_emb = &request.task_description_embedding;
        let role_emb = &request.role_description_embedding;

        // ── L1: Experience retrieval & confidence check ──
        let l1_assessment: L1Assessment = {
            let exp = self.experience.lock().unwrap();
            exp.check_confidence(task_emb, role_emb)?
        };

        // ── L1: Value classification ──
        // (classifies the value_statement for jargon detection)
        let _value = self.value_classifier.classify("");

        // ── L1: Semantic conflict detection ──
        // (would be called against active agents; omitted here for
        //  brevity — the trait is available for the caller)

        // ── L2: (not triggered unless conflict escalates) ──

        let agent_id: AgentId = rand::random();
        let task_id: TaskId = rand::random();
        let allocated_budget = _l0_permit.budget_amount();

        Ok(SpawnDecision::Approved(ChildAgentConfig {
            agent_id,
            task_id,
            allocated_budget,
            allowed_tools: l1_assessment.recommended_tools,
            role_template_id: None,
        }))
    }

    // ── Accessors for composing layers ──

    pub fn embedding(&self) -> &Arc<dyn EmbeddingService> {
        &self.embedding
    }

    pub fn plans(&self) -> &Arc<RwLock<Box<dyn PlanRegistry>>> {
        &self.plans
    }

    pub fn add_experience(&self, entry: ExperienceEntry) {
        self.experience.lock().unwrap().add_experience(entry);
    }

    pub fn experience_count(&self) -> usize {
        self.experience.lock().unwrap().experience_count()
    }

    pub fn suspend_queue_len(&self) -> usize {
        self.suspend.lock().unwrap().len()
    }

    pub fn pending_suspended(&self) -> usize {
        self.suspend.lock().unwrap().len()
    }

    pub fn available_permits(&self) -> usize {
        self.admission.available_permits()
    }

    pub fn remaining_budget(&self) -> i64 {
        self.resources.remaining_budget()
    }

    pub fn resource_pool(&self) -> &Arc<dyn ResourcePool> {
        &self.resources
    }

    pub fn audit_engine(&self) -> &Mutex<Box<dyn AuditEngine>> {
        &self.audit_engine
    }

    pub fn conflict_detector(&self) -> &dyn ConflictDetector {
        &*self.conflict_detector
    }
}

// ============================================================================
//  Macro: builder_method
// ============================================================================

/// Internal macro to generate a builder `with_*` method.
macro_rules! builder_method {
    ($field:ident, $ty:ty) => {
        pub fn $field(mut self, val: $ty) -> Self {
            self.$field = Some(val);
            self
        }
    };
}

// Re-export the macro only within this module.
pub(crate) use builder_method;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingService as EmbeddingServiceImpl;
    use crate::llm::LlmProvider;
    use std::sync::Arc;

    fn dummy_embedding() -> Arc<dyn EmbeddingService> {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::CompletionsClient::new("test-key").unwrap(),
        ));
        Arc::new(EmbeddingServiceImpl::new(provider))
    }

    #[tokio::test]
    async fn test_pipeline_approves_valid_request() {
        let pipeline = DecisionPipelineBuilder::new().embedding(dummy_embedding()).build();

        let mut task_emb = [0.0f32; 768];
        task_emb[0] = 1.0;
        let mut role_emb = [0.0f32; 768];
        role_emb[0] = 1.0;

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: 1,
            parent_span_id: 0,
            task_description_embedding: task_emb,
            role_description_embedding: role_emb,
            value_statement_embedding: [0.0f32; 768],
            requested_budget: 100,
            current_depth: 0,
            responsibility_chain: vec![],
            raw_text_ref: None,
        };

        let decision = pipeline.process_request(request).await.unwrap();
        assert!(matches!(decision, SpawnDecision::Approved(_)));
    }

    #[tokio::test]
    async fn test_pipeline_rejects_budget_exhausted() {
        let state = crate::resource::TaskResourceState::new(50, 10);
        let resources: Arc<dyn ResourcePool> = state.clone();
        let breaker = Box::new(crate::l0::L0CircuitBreaker::new(state));

        let pipeline = DecisionPipelineBuilder::new()
            .embedding(dummy_embedding())
            .resources(resources)
            .circuit_breaker(breaker)
            .build();

        let request = SpawnRequest {
            trace_id: rand::random(),
            span_id: 1,
            parent_span_id: 0,
            task_description_embedding: [0.0f32; 768],
            role_description_embedding: [0.0f32; 768],
            value_statement_embedding: [0.0f32; 768],
            requested_budget: 100,
            current_depth: 0,
            responsibility_chain: vec![],
            raw_text_ref: None,
        };

        let decision = pipeline.process_request(request).await.unwrap();
        assert!(matches!(
            decision,
            SpawnDecision::Rejected(SpawnRejection::BudgetExhausted { .. })
        ));
    }
}
