use crate::types::{AgentId, ChildAgentConfig, TraceId};
use smallvec::SmallVec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictType {
    ResourceLockContention,
    ActionContradiction,
    ValueDivergence,
}

pub struct ConflictManifest {
    pub conflict_id: [u8; 16],
    pub conflict_type: ConflictType,
    pub contending_agents: SmallVec<[AgentId; 2]>,
    pub trace_id: TraceId,
    pub context_embeddings: SmallVec<[[f32; 768]; 2]>,
    pub dynamic_priority_scores: SmallVec<[f32; 2]>,
}

pub enum ArbitrationResult {
    Override {
        winner: AgentId,
        slash_targets: Vec<AgentId>,
    },
    Merge(ChildAgentConfig),
    Prune(Vec<AgentId>),
}
