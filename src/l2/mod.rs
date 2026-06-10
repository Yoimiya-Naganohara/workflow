pub mod llm;

use crate::core::conflict::{ArbitrationResult, ConflictManifest};
use crate::core::types::AgentId;

pub struct L2RuleAuditEngine {
    max_consecutive_failures: u32,
    consecutive_failures: u32,
}

impl L2RuleAuditEngine {
    pub fn new(max_consecutive_failures: u32) -> Self {
        Self {
            max_consecutive_failures,
            consecutive_failures: 0,
        }
    }

    pub fn audit(&mut self, manifest: &ConflictManifest) -> L2RuleAuditResult {
        if self.consecutive_failures >= self.max_consecutive_failures {
            self.consecutive_failures += 1;
            return L2RuleAuditResult {
                decision: ArbitrationResult::Prune(manifest.contending_agents.to_vec()),
                risk_statement: "L2 collapsed due to consecutive failures".to_string(),
                lesson_learned: "System needs stabilization".to_string(),
                l1_override_vector_patch: None,
            };
        }

        let risk = self.assess_risk(manifest);
        let decision = if risk.is_high {
            self.consecutive_failures += 1;
            ArbitrationResult::Prune(manifest.contending_agents.to_vec())
        } else {
            self.consecutive_failures = 0;
            let winner = manifest.contending_agents[0];
            let losers: Vec<AgentId> = manifest.contending_agents[1..].to_vec();
            ArbitrationResult::Override {
                winner,
                slash_targets: losers,
            }
        };

        L2RuleAuditResult {
            decision,
            risk_statement: risk.statement,
            lesson_learned: "Resolved via L2 audit".to_string(),
            l1_override_vector_patch: Some(self.generate_override_patch(manifest)),
        }
    }

    fn assess_risk(&self, manifest: &ConflictManifest) -> RiskAssessment {
        let max_priority = manifest
            .dynamic_priority_scores
            .iter()
            .cloned()
            .fold(f32::MIN, f32::max);

        if max_priority > 0.9 {
            RiskAssessment {
                is_high: true,
                statement: "High risk: extreme priority divergence".to_string(),
            }
        } else {
            RiskAssessment {
                is_high: false,
                statement: "Moderate risk".to_string(),
            }
        }
    }

    fn generate_override_patch(&self, manifest: &ConflictManifest) -> crate::core::conflict::OverridePatch {
        let mut embedding = [0.0f32; crate::core::types::EMBEDDING_DIM];
        if !manifest.context_embeddings.is_empty() {
            embedding.copy_from_slice(&manifest.context_embeddings[0]);
        }

        crate::core::conflict::OverridePatch {
            embedding,
            weight: 2.0,
            decay_days: 90,
        }
    }

    pub fn reset_failures(&mut self) {
        self.consecutive_failures = 0;
    }
}

/// L2: High-level audit engine (rule-based or LLM-powered).
#[async_trait::async_trait]
pub trait AuditEngine: Send + Sync {
    async fn audit(
        &mut self,
        manifest: &crate::core::conflict::ConflictManifest,
    ) -> crate::core::conflict::L2AuditResult;
    fn reset(&mut self);

    /// Screen a [`SpawnRequest`] for risk *before* final approval.
    ///
    /// This method is **synchronous** so callers can hold a `std::sync::Mutex`
    /// lock.  Implementations that need async I/O should spawn their own task.
    /// Returns `None` if the request passes, or a rejection reason otherwise.
    fn screen_request(
        &mut self,
        _request: &crate::core::types::SpawnRequest,
    ) -> Option<crate::core::types::SpawnRejection> {
        // Default: pass all requests (no screening).
        None
    }
}

#[async_trait::async_trait]
impl AuditEngine for L2RuleAuditEngine {
    async fn audit(
        &mut self,
        manifest: &crate::core::conflict::ConflictManifest,
    ) -> crate::core::conflict::L2AuditResult {
        let result = self.audit(manifest);
        crate::core::conflict::L2AuditResult {
            decision: result.decision,
            risk_statement: result.risk_statement,
            lesson_learned: result.lesson_learned,
            override_patch: result.l1_override_vector_patch,
            tokens_used: 0,
        }
    }

    fn reset(&mut self) {
        self.reset_failures();
    }

    fn screen_request(
        &mut self,
        request: &crate::core::types::SpawnRequest,
    ) -> Option<crate::core::types::SpawnRejection> {
        // Reject if collapsed.
        if self.consecutive_failures >= self.max_consecutive_failures {
            return Some(crate::core::types::SpawnRejection::L2Collapsed);
        }

        // Reject if depth is too high (already checked in L0, but defense-in-depth).
        let max_depth = crate::core::types::DEFAULT_MAX_DEPTH;
        if request.current_depth > max_depth {
            self.consecutive_failures += 1;
            return Some(crate::core::types::SpawnRejection::L2Rejected {
                reason: "Depth exceeds L2 safety limit".to_string(),
                category: "depth".to_string(),
            });
        }

        None
    }
}

struct RiskAssessment {
    is_high: bool,
    statement: String,
}

pub struct L2RuleAuditResult {
    pub decision: ArbitrationResult,
    pub risk_statement: String,
    pub lesson_learned: String,
    pub l1_override_vector_patch: Option<crate::core::conflict::OverridePatch>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::SmallVec;

    fn make_manifest(agents: Vec<AgentId>, priorities: Vec<f32>) -> ConflictManifest {
        ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: crate::core::conflict::ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_vec(agents),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::from_vec(vec![[0.0f32; crate::core::types::EMBEDDING_DIM]; 2]),
            dynamic_priority_scores: SmallVec::from_vec(priorities),
        }
    }

    #[test]
    fn test_basic_audit() {
        let mut engine = L2RuleAuditEngine::new(3);
        let manifest = make_manifest(vec![[1u8; 16], [2u8; 16]], vec![0.8, 0.3]);

        let result = engine.audit(&manifest);
        assert!(matches!(result.decision, ArbitrationResult::Override { .. }));
    }

    #[test]
    fn test_high_risk_prune() {
        let mut engine = L2RuleAuditEngine::new(3);
        let manifest = make_manifest(vec![[1u8; 16], [2u8; 16]], vec![0.95, 0.1]);

        let result = engine.audit(&manifest);
        assert!(matches!(result.decision, ArbitrationResult::Prune(_)));
    }

    #[test]
    fn test_collapse_after_failures() {
        let mut engine = L2RuleAuditEngine::new(2);
        let manifest = make_manifest(vec![[1u8; 16], [2u8; 16]], vec![0.95, 0.1]);

        engine.audit(&manifest);
        engine.audit(&manifest);
        let result = engine.audit(&manifest);

        assert!(matches!(result.decision, ArbitrationResult::Prune(_)));
        assert!(result.risk_statement.contains("collapsed"));
    }

    #[test]
    fn test_reset_failures() {
        let mut engine = L2RuleAuditEngine::new(2);
        let manifest = make_manifest(vec![[1u8; 16], [2u8; 16]], vec![0.95, 0.1]);

        engine.audit(&manifest);
        engine.audit(&manifest);
        engine.reset_failures();

        let result = engine.audit(&manifest);
        assert!(!result.risk_statement.contains("collapsed"));
    }
}
