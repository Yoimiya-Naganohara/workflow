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

#[cfg(test)]
mod tests {
    use super::*;

    // ── ConflictType ──

    #[test]
    fn test_conflict_type_variants() {
        assert_eq!(
            format!("{:?}", ConflictType::ResourceLockContention),
            "ResourceLockContention"
        );
        assert_eq!(
            format!("{:?}", ConflictType::ActionContradiction),
            "ActionContradiction"
        );
        assert_eq!(
            format!("{:?}", ConflictType::ValueDivergence),
            "ValueDivergence"
        );
    }

    // ── ConflictManifest ──

    #[test]
    fn test_conflict_manifest_construction() {
        let agents: SmallVec<[AgentId; 2]> = smallvec::smallvec![[1; 16], [2; 16]];
        let ctx: SmallVec<[[f32; EMBEDDING_DIM]; 2]> =
            smallvec::smallvec![[0.1; EMBEDDING_DIM], [0.2; EMBEDDING_DIM]];
        let prio: SmallVec<[f32; 2]> = smallvec::smallvec![0.8, 0.3];
        let manifest = ConflictManifest {
            conflict_id: [0xAB; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: agents.clone(),
            trace_id: [0xCD; 16],
            context_embeddings: ctx,
            dynamic_priority_scores: prio.clone(),
        };
        assert_eq!(manifest.conflict_id, [0xAB; 16]);
        assert_eq!(manifest.conflict_type, ConflictType::ActionContradiction);
        assert_eq!(manifest.contending_agents.len(), 2);
        assert_eq!(manifest.contending_agents[0], [1; 16]);
        assert_eq!(manifest.trace_id, [0xCD; 16]);
        assert_eq!(manifest.context_embeddings.len(), 2);
        assert_eq!(manifest.dynamic_priority_scores, prio);
    }

    #[test]
    fn test_conflict_manifest_empty_contending() {
        let manifest = ConflictManifest {
            conflict_id: [0; 16],
            conflict_type: ConflictType::ResourceLockContention,
            contending_agents: SmallVec::new(),
            trace_id: [0; 16],
            context_embeddings: SmallVec::new(),
            dynamic_priority_scores: SmallVec::new(),
        };
        assert!(manifest.contending_agents.is_empty());
        assert!(manifest.context_embeddings.is_empty());
        assert!(manifest.dynamic_priority_scores.is_empty());
    }

    // ── ArbitrationResult ──

    #[test]
    fn test_arbitration_override() {
        let winner = [0x11; 16];
        let targets = vec![[0x22; 16], [0x33; 16]];
        let result = ArbitrationResult::Override {
            winner,
            slash_targets: targets.clone(),
        };
        match result {
            ArbitrationResult::Override {
                winner: w,
                slash_targets: t,
            } => {
                assert_eq!(w, winner);
                assert_eq!(t, targets);
            }
            _ => panic!("expected Override"),
        }
    }

    #[test]
    fn test_arbitration_prune_empty() {
        let result = ArbitrationResult::Prune(vec![]);
        match result {
            ArbitrationResult::Prune(agents) => assert!(agents.is_empty()),
            _ => panic!("expected Prune"),
        }
    }

    #[test]
    fn test_arbitration_prune_single() {
        let result = ArbitrationResult::Prune(vec![[0x42; 16]]);
        match result {
            ArbitrationResult::Prune(agents) => {
                assert_eq!(agents.len(), 1);
                assert_eq!(agents[0], [0x42; 16]);
            }
            _ => panic!("expected Prune"),
        }
    }

    #[test]
    fn test_arbitration_clone_and_debug() {
        let result = ArbitrationResult::Override {
            winner: [1; 16],
            slash_targets: vec![[2; 16]],
        };
        let cloned = result.clone();
        assert!(format!("{:?}", cloned).contains("Override"));
    }

    // ── OverridePatch ──

    #[test]
    fn test_override_patch_construction() {
        let patch = OverridePatch {
            embedding: [0.5; EMBEDDING_DIM],
            weight: 2.0,
            decay_days: 7,
        };
        assert_eq!(patch.embedding[0], 0.5);
        assert_eq!(patch.weight, 2.0);
        assert_eq!(patch.decay_days, 7);
    }

    #[test]
    fn test_override_patch_zero_weight() {
        let patch = OverridePatch {
            embedding: [0.0; EMBEDDING_DIM],
            weight: 0.0,
            decay_days: 0,
        };
        assert_eq!(patch.weight, 0.0);
        assert_eq!(patch.decay_days, 0);
    }

    // ── L2AuditResult ──

    #[test]
    fn test_l2_audit_result_construction() {
        let result = L2AuditResult {
            decision: ArbitrationResult::Override {
                winner: [0xAA; 16],
                slash_targets: vec![[0xBB; 16]],
            },
            risk_statement: "High risk".to_string(),
            lesson_learned: "Lesson".to_string(),
            override_patch: Some(OverridePatch {
                embedding: [0.1; EMBEDDING_DIM],
                weight: 1.5,
                decay_days: 3,
            }),
            tokens_used: 42,
        };
        assert!(result.risk_statement.contains("High"));
        assert!(result.lesson_learned.contains("Lesson"));
        assert_eq!(result.tokens_used, 42);
        assert!(result.override_patch.is_some());
    }

    #[test]
    fn test_l2_audit_result_no_override() {
        let result = L2AuditResult {
            decision: ArbitrationResult::Prune(vec![]),
            risk_statement: String::new(),
            lesson_learned: String::new(),
            override_patch: None,
            tokens_used: 0,
        };
        assert!(result.override_patch.is_none());
        assert!(result.risk_statement.is_empty());
        assert_eq!(result.tokens_used, 0);
    }
}
