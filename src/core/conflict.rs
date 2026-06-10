use crate::core::types::EMBEDDING_DIM;
use crate::core::types::{AgentId, TraceId};
use smallvec::SmallVec;

/// Unified result returned by any L2 audit engine (rule or LLM).
#[derive(Debug, Clone)]
pub struct L2AuditResult {
    pub decision: ArbitrationResult,
    pub risk_statement: String,
    pub lesson_learned: String,
    pub override_patch: Option<OverridePatch>,
    pub tokens_used: u32,
}

/// Patch injected into L1 experience pool to bias future decisions.
#[derive(Debug, Clone)]
pub struct OverridePatch {
    pub embedding: [f32; EMBEDDING_DIM],
    pub weight: f32,
    pub decay_days: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictType {
    ResourceLockContention,
    ActionContradiction,
    ValueDivergence,
}

#[derive(Debug, Clone)]
pub struct ConflictManifest {
    pub conflict_id: [u8; 16],
    pub conflict_type: ConflictType,
    pub contending_agents: SmallVec<[AgentId; 2]>,
    pub trace_id: TraceId,
    pub context_embeddings: SmallVec<[[f32; EMBEDDING_DIM]; 2]>,
    pub dynamic_priority_scores: SmallVec<[f32; 2]>,
}

#[derive(Debug, Clone)]
pub enum ArbitrationResult {
    Override {
        winner: AgentId,
        slash_targets: Vec<AgentId>,
    },
    Prune(Vec<AgentId>),
}
