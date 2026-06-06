//! Interface definitions for the layered decision pipeline.
//!
//! Each architectural layer defines its contract via a trait,
//! enabling dependency injection, testability, and runtime
//! polymorphism.
//!
//! Only layers with genuine multiple-implementation potential
//! are abstracted — trivial 1:1 wrappers are kept as concrete
//! structs to avoid unnecessary indirection.

use crate::admission::AdmissionPermit;
use crate::core::conflict::{ConflictManifest, L2AuditResult};
use crate::l0::L0Permit;
use crate::l1::L1Assessment;
use crate::agent::plan::PlanEntity;
use crate::core::types::*;
use anyhow::Result;
use async_trait::async_trait;

// ============================================================================
//  L-1: Admission Control
// ============================================================================

/// Concurrency admission for agent spawns.
///
/// Limits the number of concurrently processing agents.
/// The canonical implementation uses a `tokio::Semaphore`;
/// alternative implementations could use a rate-limiter,
/// priority queue, or external admission webhook.
#[async_trait]
pub trait AdmissionControl: Send + Sync {
    /// Acquire an admission permit. Returns an error if the
    /// semaphore is exhausted or the timeout elapses.
    async fn acquire(&self) -> Result<AdmissionPermit, SpawnRejection>;

    /// Number of permits still available (advisory).
    fn available_permits(&self) -> usize;
}

// ============================================================================
//  L0: Circuit Breaker
// ============================================================================

/// Physical-resource circuit breaker.
///
/// Guards budget, depth, and tool-lock resources.
/// Returns an [`L0Permit`] whose `Drop` auto-rolls back resources.
pub trait CircuitBreaker: Send + Sync {
    /// Reserve budget, check depth, and lock tools.
    fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection>;

    /// Priority score for the suspend queue.
    fn calculate_priority(&self, budget_remaining: i64, budget_requested: u64, depth: u32) -> f32;

    /// Remaining budget (advisory, for diagnostics and priority).
    fn remaining_budget(&self) -> i64;
}

// ============================================================================
//  L1: Experience Retrieval
// ============================================================================

/// Experience-driven confidence assessment.
///
/// Maintains a pool of prior [`ExperienceEntry`] values and
/// scores new requests by cosine-similarity with weighting.
pub trait ExperienceRetrieval: Send + Sync {
    /// Retrieve the top-*k* most similar experiences (owned).
    fn retrieve(&self, query: &[f32; 768], k: usize) -> Vec<(ExperienceEntry, f32)>;

    /// Evaluate confidence that the task/role pair can be handled.
    fn check_confidence(
        &self,
        task_embedding: &[f32; 768],
        role_embedding: &[f32; 768],
    ) -> Result<L1Assessment, SpawnRejection>;

    /// Add a new experience entry.
    fn add_experience(&mut self, entry: ExperienceEntry);

    /// Number of stored experiences.
    fn experience_count(&self) -> usize;
}

// ============================================================================
//  L2: Audit Engine
// ============================================================================

/// High-level audit engine (rule-based or LLM-powered).
///
/// Receives escalated conflicts from L1 and produces a final
/// arbitration decision. The two canonical implementations are
/// [`L2RuleAuditEngine`](crate::l2::L2RuleAuditEngine) and
/// [`L2LlmAuditEngine`](crate::l2::llm::L2LlmAuditEngine).
#[async_trait]
pub trait AuditEngine: Send + Sync {
    /// Audit a conflict manifest and return a result.
    async fn audit(&mut self, manifest: &ConflictManifest) -> L2AuditResult;

    /// Reset consecutive-failure counter.
    fn reset(&mut self);
}

// ============================================================================
//  Service: Embedding
// ============================================================================

/// Text-to-vector embedding service with caching.
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    async fn embed(&self, text: &str) -> Result<[f32; 768]>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; 768]>>;
    fn similarity(&self, a: &[f32; 768], b: &[f32; 768]) -> f32;
    fn cache_size(&self) -> usize;
    fn clear_cache(&self);
}

// ============================================================================
//  Service: Suspend Queue
// ============================================================================

/// Priority-ordered queue for deferred spawn requests.
pub trait SuspendQueue: Send + Sync {
    fn enqueue(&mut self, request: SpawnRequest, priority: f32);
    fn dequeue(&mut self) -> Option<crate::agent::suspend::SuspendedRequest>;
    fn prune_expired(&mut self) -> Vec<SpawnRequest>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

// ============================================================================
//  Service: Plan Registry
// ============================================================================

/// Registry mapping agents to their plans and tasks.
pub trait PlanRegistry: Send + Sync {
    fn insert(&mut self, entity: PlanEntity);
    fn get_by_name(&self, name: &str) -> Option<PlanEntity>;
    fn get_by_agent(&self, agent_id: AgentId) -> Vec<PlanEntity>;
    fn search(&self, query: &str) -> Vec<PlanEntity>;
    fn all(&self) -> Vec<PlanEntity>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}
