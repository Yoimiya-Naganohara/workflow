use crate::core::conflict::{ConflictManifest, ConflictType};
use crate::core::simd::cosine_similarity_384;
use crate::core::types::AgentId;
use crate::core::types::EMBEDDING_DIM;
use smallvec::SmallVec;

pub struct L1Arbitrator {
    semantic_threshold: f32,
}

impl L1Arbitrator {
    pub fn new(semantic_threshold: f32) -> Self {
        Self { semantic_threshold }
    }

    pub fn detect_semantic_conflict(
        &self,
        embedding_a: &[f32; EMBEDDING_DIM],
        embedding_b: &[f32; EMBEDDING_DIM],
    ) -> bool {
        let sim = cosine_similarity_384(embedding_a, embedding_b);
        sim < self.semantic_threshold
    }

    pub fn create_conflict_manifest(
        &self,
        agent_a: AgentId,
        agent_b: AgentId,
        embedding_a: [f32; EMBEDDING_DIM],
        embedding_b: [f32; EMBEDDING_DIM],
        trace_id: [u8; 16],
    ) -> ConflictManifest {
        let sim = cosine_similarity_384(&embedding_a, &embedding_b);

        // Use agent_id bytes as deterministic tiebreaker when embeddings are
        // near-identical (sim > 0.99) to avoid artificially favoring one agent.
        let (priority_a, priority_b) = if sim > 0.99 {
            if agent_a <= agent_b { (1.0, 0.0) } else { (0.0, 1.0) }
        } else {
            (1.0 - sim, sim)
        };

        ConflictManifest {
            conflict_id: rand::random(),
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_slice(&[agent_a, agent_b]),
            trace_id,
            context_embeddings: SmallVec::from_slice(&[embedding_a, embedding_b]),
            dynamic_priority_scores: SmallVec::from_slice(&[priority_a, priority_b]),
        }
    }

    pub fn arbitrate_by_priority(&self, manifest: &ConflictManifest) -> L1ArbitrationResult {
        if manifest.dynamic_priority_scores.len() < 2 {
            return L1ArbitrationResult::NoConflict;
        }

        let score_a = manifest.dynamic_priority_scores[0];
        let score_b = manifest.dynamic_priority_scores[1];

        if score_a > score_b {
            L1ArbitrationResult::Override {
                winner: manifest.contending_agents[0],
                loser: manifest.contending_agents[1],
            }
        } else if score_b > score_a {
            L1ArbitrationResult::Override {
                winner: manifest.contending_agents[1],
                loser: manifest.contending_agents[0],
            }
        } else {
            L1ArbitrationResult::RequiresL2
        }
    }
}

pub enum L1ArbitrationResult {
    NoConflict,
    Override { winner: AgentId, loser: AgentId },
    RequiresL2,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_semantic_conflict() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = -1.0;

        assert!(arbitrator.detect_semantic_conflict(&a, &b));
    }

    #[test]
    fn test_no_semantic_conflict() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 1.0;

        assert!(!arbitrator.detect_semantic_conflict(&a, &b));
    }

    #[test]
    fn test_arbitrate_by_priority() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let manifest = ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_slice(&[[1u8; 16], [2u8; 16]]),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::from_slice(&[[0.0f32; EMBEDDING_DIM], [0.0f32; EMBEDDING_DIM]]),
            dynamic_priority_scores: SmallVec::from_slice(&[0.8, 0.3]),
        };

        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, loser } => {
                assert_eq!(winner, [1u8; 16]);
                assert_eq!(loser, [2u8; 16]);
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_identical_embeddings_tiebreaker_lower_id_wins() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 1.0;

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // Tiebreaker: lower agent_id bytes gets higher priority
        // agent_a = [1; 16] < agent_b = [2; 16], so agent_a should win
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                assert_eq!(winner, [1u8; 16], "lower agent_id bytes should win tiebreaker");
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_complement_formula_with_non_collinear_embeddings() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 0.8;
        b[1] = 0.6; // norm = 1.0, different direction from a

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // sim = dot(a,b) / (|a| * |b|) = 0.8 / (1.0 * 1.0) = 0.8 (< 0.99, no tiebreaker)
        // priority_a = 1.0 - 0.8 = 0.2, priority_b = 0.8
        // Agent B wins from the complement formula
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                assert_eq!(winner, [2u8; 16], "agent B has higher priority from complement formula");
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_arbitrate_empty_agents() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let manifest = ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::new(),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::new(),
            dynamic_priority_scores: SmallVec::new(),
        };

        let result = arbitrator.arbitrate_by_priority(&manifest);
        assert!(matches!(result, L1ArbitrationResult::NoConflict));
    }
}
