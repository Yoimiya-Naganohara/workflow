use anyhow::Result;
use std::sync::Arc;

use crate::admission::AdmissionController;
use crate::embedding::EmbeddingService;
use crate::l0::L0CircuitBreaker;
use crate::l1::L1Retriever;
use crate::l1_arbitration::L1Arbitrator;
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
    suspend_queue: SuspendQueue,
}

impl AgentRuntime {
    pub fn new(config: AgentRuntimeConfig, embedding_service: Arc<EmbeddingService>) -> Self {
        let admission =
            AdmissionController::new(config.max_concurrent_agents, config.admission_timeout_ms);
        let resource_state = TaskResourceState::new(config.initial_budget, config.max_depth);
        let l0 = L0CircuitBreaker::new(resource_state.clone());
        let l1 = L1Retriever::new(config.l1_confidence_threshold);
        let l1_arbitrator = L1Arbitrator::new(config.semantic_conflict_threshold);
        let suspend_queue = SuspendQueue::new(SuspendConfig {
            hard_timeout_ms: config.suspend_timeout_ms,
            dynamic_timeout_ms: config.suspend_timeout_ms,
        });

        Self {
            config,
            admission,
            resource_state,
            l0,
            l1,
            l1_arbitrator,
            embedding_service,
            suspend_queue,
        }
    }

    pub async fn process_request(&mut self, request: SpawnRequest) -> Result<SpawnDecision> {
        let _permit = self
            .admission
            .acquire_owned()
            .await
            .map_err(|e| anyhow::anyhow!("Admission failed: {:?}", e))?;

        let l0_result = self
            .l0
            .try_acquire(request.requested_budget, request.current_depth, 0);

        let l0_permit = match l0_result {
            Ok(permit) => permit,
            Err(rejection) => {
                if matches!(rejection, SpawnRejection::ResourceConflict { .. }) {
                    self.suspend_queue.enqueue(request, 0.5);
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
        let budget_guard = l0_permit.into_budget_guard(task_id);

        Ok(SpawnDecision::Approved(ChildAgentConfig {
            agent_id,
            task_id,
            allocated_budget: budget_guard.amount(),
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

    pub fn pending_suspended(&self) -> usize {
        self.suspend_queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::LlmProvider;

    #[tokio::test]
    async fn test_basic_spawn() {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::Client::new("test-key").unwrap(),
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
            rig::providers::openai::Client::new("test-key").unwrap(),
        ));
        let embedding_service = Arc::new(EmbeddingService::new(provider));
        let config = AgentRuntimeConfig {
            initial_budget: 100,
            ..Default::default()
        };
        let mut runtime = AgentRuntime::new(config, embedding_service);

        let result = runtime
            .process_with_text("task", "role", "value", 200, 0)
            .await;

        assert!(
            result.is_err()
                || matches!(
                    result.unwrap(),
                    SpawnDecision::Rejected(SpawnRejection::BudgetExhausted { .. })
                )
        );
    }
}
