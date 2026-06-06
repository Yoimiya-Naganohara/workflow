//! Interface definitions for the layered decision pipeline.
//!
//! Each architectural layer defines its contract via a trait,
//! enabling dependency injection, testability, and runtime
//! polymorphism.

use crate::admission::AdmissionPermit;
use crate::conflict::{ArbitrationResult, ConflictManifest};
use crate::l0::L0Permit;
use crate::l1::{L1Assessment, ValueAssessment};
use crate::l1_arbitration::L1ArbitrationResult;
use crate::plan::PlanEntity;
use crate::suspend::SuspendedRequest;
use crate::types::*;
use anyhow::Result;
use async_trait::async_trait;

// ============================================================================
//  Layer 1: Admission Control (L-1)
// ============================================================================

/// Concurrency admission for agent spawns.
///
/// Uses a semaphore to limit the number of concurrent agents.
/// Implementations must be `Send + Sync` so they can be shared
/// across async tasks.
#[async_trait]
pub trait AdmissionControl: Send + Sync {
    /// Try to acquire an admission permit. Returns an error if the
    /// semaphore is exhausted or the timeout elapses.
    async fn acquire(&self) -> Result<AdmissionPermit, SpawnRejection>;

    /// Number of permits still available (advisory).
    fn available_permits(&self) -> usize;
}

// ============================================================================
//  Layer 2: Circuit Breaker (L0)
// ============================================================================

/// Physical resource circuit breaker.
///
/// Guards budget, depth, and tool-lock resources using CAS
/// atomics. Every successful acquisition returns an [`L0Permit`]
/// whose `Drop` implementation automatically rolls back resources.
pub trait CircuitBreaker: Send + Sync {
    /// Attempt to reserve budget, check depth, and lock tools.
    ///
    /// Returns an [`L0Permit`] on success, or a [`SpawnRejection`]
    /// describing which guardrail was tripped.
    fn try_acquire(
        &self,
        requested_budget: u64,
        current_depth: u32,
        requested_tools: u64,
    ) -> Result<L0Permit, SpawnRejection>;

    /// Compute a priority score for a suspended request so that
    /// the queue can reorder by importance.
    fn calculate_priority(&self, budget_remaining: i64, budget_requested: u64, depth: u32) -> f32;
}

// ============================================================================
//  Layer 3: Experience Retrieval (L1)
// ============================================================================

/// Experience-driven confidence assessment.
///
/// Maintains a pool of prior [`ExperienceEntry`] values and
/// scores new requests by cosine-similarity with weighting.
pub trait ExperienceRetrieval: Send + Sync {
    /// Retrieve the top-*k* most similar experiences.
    ///
    /// Returns owned entries so the trait can be used as a trait
    /// object (no lifetime parameter needed).
    fn retrieve(&self, query: &[f32; 768], k: usize) -> Vec<(ExperienceEntry, f32)>;

    /// Evaluate confidence that the given task/role pair can be
    /// handled based on past experience.
    fn check_confidence(
        &self,
        task_embedding: &[f32; 768],
        role_embedding: &[f32; 768],
    ) -> Result<L1Assessment, SpawnRejection>;

    /// Add a new experience entry (e.g. after an agent completes
    /// successfully).
    fn add_experience(&mut self, entry: ExperienceEntry);

    /// Number of stored experiences.
    fn experience_count(&self) -> usize;
}

// ============================================================================
//  Layer 4: Value Classifier (L1)
// ============================================================================

/// Lightweight value / jargon classifier.
///
/// Used during L1 to detect high-risk language (urgency,
/// criticality, etc.) and adjust spawn probability.
pub trait ValueClassifier: Send + Sync {
    /// Classify the value-statement text.
    fn classify(&self, text: &str) -> ValueAssessment;
}

// ============================================================================
//  Layer 5: Conflict Detector (L1 Arbitration)
// ============================================================================

/// Semantic conflict detection and priority-based arbitration.
pub trait ConflictDetector: Send + Sync {
    /// Return `true` if the two embeddings represent a semantic
    /// conflict (cosine similarity below threshold).
    fn detect(&self, a: &[f32; 768], b: &[f32; 768]) -> bool;

    /// Build a full [`ConflictManifest`] for escalation to L2.
    fn create_manifest(
        &self,
        agent_a: AgentId,
        agent_b: AgentId,
        embedding_a: [f32; 768],
        embedding_b: [f32; 768],
        trace_id: [u8; 16],
    ) -> ConflictManifest;

    /// Resolve a conflict by comparing dynamic priority scores.
    /// Falls through to [`L1ArbitrationResult::RequiresL2`] when
    /// scores are tied.
    fn arbitrate(&self, manifest: &ConflictManifest) -> L1ArbitrationResult;
}

// ============================================================================
//  Layer 6: Audit Engine (L2)
// ============================================================================

/// Shared outcome type for all L2 audit engines.
#[derive(Debug, Clone)]
pub struct AuditOutcome {
    pub decision: ArbitrationResult,
    pub risk_statement: String,
    pub lesson_learned: String,
    pub override_patch: Option<UnifiedOverridePatch>,
}

/// Patch that L2 can inject into L1's experience pool to bias
/// future decisions.
#[derive(Debug, Clone)]
pub struct UnifiedOverridePatch {
    pub embedding: [f32; 768],
    pub weight: f32,
    pub decay_days: u32,
}

/// High-level audit engine (rule-based or LLM-powered).
///
/// Receives escalated conflicts from L1 and produces a final
/// arbitration decision.
#[async_trait]
pub trait AuditEngine: Send + Sync {
    /// Audit a conflict manifest and return an outcome.
    async fn audit(&mut self, manifest: &ConflictManifest) -> AuditOutcome;

    /// Reset consecutive-failure counter (e.g. after successful
    /// recovery).
    fn reset(&mut self);
}

// ============================================================================
//  Service: Embedding
// ============================================================================

/// Text-to-vector embedding service with caching.
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// Embed a single text string into a 768-d vector.
    async fn embed(&self, text: &str) -> Result<[f32; 768]>;

    /// Embed a batch of texts.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; 768]>>;

    /// Cosine similarity between two embeddings.
    fn similarity(&self, a: &[f32; 768], b: &[f32; 768]) -> f32;

    /// Number of cached embeddings.
    fn cache_size(&self) -> usize;

    /// Clear the embedding cache.
    fn clear_cache(&self);
}

// ============================================================================
//  Service: Resource Pool
// ============================================================================

/// Atomic resource pool for budget, tools, and depth bookkeeping.
pub trait ResourcePool: Send + Sync {
    fn try_acquire_budget(&self, requested: u64) -> Option<u64>;
    fn release_budget(&self, amount: u64);
    fn try_lock_tools(&self, bitmap: u64) -> Result<(), u64>;
    fn release_tools(&self, bitmap: u64);
    fn try_increment_depth(&self) -> Result<u32, u32>;
    fn decrement_depth(&self);
    fn increment_spawned(&self) -> u32;
    fn remaining_budget(&self) -> i64;
    fn current_depth(&self) -> u32;
    fn max_depth(&self) -> u32;
}

// ============================================================================
//  Service: Suspend Queue
// ============================================================================

/// Priority-ordered queue for requests that were temporarily
/// rejected (e.g. resource conflict) and may be retried later.
pub trait SuspendQueue: Send + Sync {
    fn enqueue(&mut self, request: SpawnRequest, priority: f32);
    fn dequeue(&mut self) -> Option<SuspendedRequest>;
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
