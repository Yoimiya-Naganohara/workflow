use crate::core::conflict::{ConflictManifest, ConflictType};
use crate::core::simd::cosine_similarity_768;
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
        let sim = cosine_similarity_768(embedding_a, embedding_b);
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
        let sim = cosine_similarity_768(&embedding_a, &embedding_b);
        let priority_a = 1.0 - sim;
        let priority_b = sim;

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
    fn test_identical_embeddings_agent_b_wins() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 1.0;

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // BUG: when sim=1.0, priority_a = 1.0 - 1.0 = 0.0, priority_b = 1.0
        // Agent B always wins with identical embeddings
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                assert_eq!(
                    winner, [2u8; 16],
                    "BUG: identical embeddings should not deterministically favor B"
                );
            }
            _ => panic!("Expected Override"),
        }
    }

    #[test]
    fn test_priority_inversion_higher_sim_lower_priority() {
        let arbitrator = L1Arbitrator::new(-0.6);

        let mut a = [0.0f32; EMBEDDING_DIM];
        a[0] = 1.0;
        let mut b = [0.0f32; EMBEDDING_DIM];
        b[0] = 0.5; // Less similar to query than a

        let manifest = arbitrator.create_conflict_manifest([1u8; 16], [2u8; 16], a, b, [0u8; 16]);

        // BUG: priority_a = 1.0 - sim, priority_b = sim
        // Higher similarity means LOWER priority_a — inversion!
        // Agent A (more similar) gets lower priority than B (less similar)
        match arbitrator.arbitrate_by_priority(&manifest) {
            L1ArbitrationResult::Override { winner, .. } => {
                // With sim ~= 0.87, priority_a = 0.13, priority_b = 0.87
                // B wins despite being less similar
                assert_eq!(
                    winner, [2u8; 16],
                    "BUG: less similar agent wins due to priority inversion"
                );
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
