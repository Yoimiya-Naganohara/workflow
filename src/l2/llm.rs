use crate::core::conflict::{ArbitrationResult, ConflictManifest, ConflictType};
use crate::llm::LlmProvider;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Configuration for the LLM-powered audit engine.
pub struct L2LlmConfig {
    pub model_id: String,
    pub temperature: f64,
    pub max_tokens: u64,
    pub max_consecutive_failures: u32,
}

impl Default for L2LlmConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            temperature: crate::core::types::L2_JUDGE_TEMPERATURE,
            max_tokens: crate::core::types::L2_JUDGE_MAX_TOKENS,
            max_consecutive_failures: crate::core::types::MAX_CONSECUTIVE_FAILURES,
        }
    }
}

pub struct L2LlmAuditEngine {
    provider: Arc<LlmProvider>,
    model_id: String,
    temperature: f64,
    max_tokens: u64,
    max_consecutive_failures: u32,
    consecutive_failures: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct JudgeDecision {
    decision: String,
    winner: Option<String>,
    risk_level: String,
    risk_statement: String,
    lesson_learned: String,
}

impl L2LlmAuditEngine {
    pub fn new(provider: Arc<LlmProvider>, config: L2LlmConfig) -> Self {
        Self {
            provider,
            model_id: config.model_id,
            temperature: config.temperature,
            max_tokens: config.max_tokens,
            max_consecutive_failures: config.max_consecutive_failures,
            consecutive_failures: 0,
        }
    }

    pub async fn audit(&mut self, manifest: &ConflictManifest) -> Result<L2LlmAuditResult> {
        if self.consecutive_failures >= self.max_consecutive_failures {
            self.consecutive_failures += 1;
            return Ok(L2LlmAuditResult {
                decision: ArbitrationResult::Prune(manifest.contending_agents.to_vec()),
                risk_statement: "L2 collapsed due to consecutive failures".to_string(),
                lesson_learned: "System needs stabilization".to_string(),
                l1_override_vector_patch: None,
                tokens_used: 0,
            });
        }

        let prompt = self.build_judge_prompt(manifest);

        let request = crate::llm::LlmRequest {
            model: self.model_id.clone(),
            messages: vec![
                crate::llm::Message {
                    role: "system".to_string(),
                    content: "You are a system architect and ethics reviewer. Analyze the conflict and return a JSON decision.".to_string(),
                },
                crate::llm::Message {
                    role: "user".to_string(),
                    content: prompt,
                },
            ],
            temperature: self.temperature,
            max_tokens: self.max_tokens,
        };

        let response = self.provider.complete(request).await?;
        let judge = self.parse_decision(&response.content)?;

        if judge.risk_level == "high" {
            self.consecutive_failures += 1;
        } else {
            self.consecutive_failures = 0;
        }

        let arbitration = match judge.decision.as_str() {
            "override" => {
                let winner_idx = judge.winner.and_then(|w| w.parse::<usize>().ok()).unwrap_or(0);
                let winner = manifest
                    .contending_agents
                    .get(winner_idx)
                    .copied()
                    .unwrap_or(manifest.contending_agents[0]);
                let losers: Vec<_> = manifest
                    .contending_agents
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != winner_idx)
                    .map(|(_, a)| *a)
                    .collect();
                ArbitrationResult::Override {
                    winner,
                    slash_targets: losers,
                }
            }
            "merge" => ArbitrationResult::Prune(manifest.contending_agents.to_vec()),
            _ => ArbitrationResult::Prune(manifest.contending_agents.to_vec()),
        };

        Ok(L2LlmAuditResult {
            decision: arbitration,
            risk_statement: judge.risk_statement,
            lesson_learned: judge.lesson_learned,
            l1_override_vector_patch: Some(self.generate_override_patch(manifest)),
            tokens_used: response.tokens_used,
        })
    }

    fn build_judge_prompt(&self, manifest: &ConflictManifest) -> String {
        let conflict_type = match manifest.conflict_type {
            ConflictType::ResourceLockContention => "resource lock contention",
            ConflictType::ActionContradiction => "action contradiction",
            ConflictType::ValueDivergence => "value divergence",
        };

        let agents: Vec<String> = manifest.contending_agents.iter().map(|a| format!("{:?}", a)).collect();

        format!(
            r#"Analyze this multi-agent conflict:

Conflict Type: {}
Contending Agents: {}
Priority Scores: {:?}

Context:
- Agent embeddings show semantic divergence
- This conflict was escalated from L1 arbitration

Please respond with JSON:
{{
  "decision": "override|merge|prune",
  "winner": "agent_id or null",
  "risk_level": "low|medium|high",
  "risk_statement": "brief risk assessment",
  "lesson_learned": "what to remember for future"
}}"#,
            conflict_type,
            agents.join(", "),
            manifest.dynamic_priority_scores.as_slice()
        )
    }

    fn parse_decision(&self, content: &str) -> Result<JudgeDecision> {
        let json_start = content.find('{').unwrap_or(0);
        let json_end = content.rfind('}').map(|i| i + 1).unwrap_or(content.len());
        let json_str = &content[json_start..json_end];

        let decision: JudgeDecision = serde_json::from_str(json_str)?;

        if (decision.decision == "override" && decision.winner.is_some())
            || decision.decision == "prune"
            || decision.decision == "merge"
        {
            Ok(decision)
        } else {
            Ok(JudgeDecision {
                decision: "prune".to_string(),
                winner: None,
                risk_level: "high".to_string(),
                risk_statement: "Unable to determine winner, pruning for safety".to_string(),
                lesson_learned: "LLM output was ambiguous".to_string(),
            })
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

#[async_trait::async_trait]
impl super::AuditEngine for L2LlmAuditEngine {
    async fn audit(
        &mut self,
        manifest: &crate::core::conflict::ConflictManifest,
    ) -> crate::core::conflict::L2AuditResult {
        match self.audit(manifest).await {
            Ok(result) => crate::core::conflict::L2AuditResult {
                decision: result.decision,
                risk_statement: result.risk_statement,
                lesson_learned: result.lesson_learned,
                override_patch: result
                    .l1_override_vector_patch
                    .map(|p| crate::core::conflict::OverridePatch {
                        embedding: p.embedding,
                        weight: p.weight,
                        decay_days: p.decay_days,
                    }),
                tokens_used: result.tokens_used,
            },
            Err(e) => crate::core::conflict::L2AuditResult {
                decision: crate::core::conflict::ArbitrationResult::Prune(manifest.contending_agents.to_vec()),
                risk_statement: format!("LLM audit error: {}", e),
                lesson_learned: "LLM audit failed, pruning for safety".to_string(),
                override_patch: None,
                tokens_used: 0,
            },
        }
    }

    fn reset(&mut self) {
        self.reset_failures();
    }
}

pub struct L2LlmAuditResult {
    pub decision: ArbitrationResult,
    pub risk_statement: String,
    pub lesson_learned: String,
    pub l1_override_vector_patch: Option<crate::core::conflict::OverridePatch>,
    pub tokens_used: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::AgentId;
    use smallvec::SmallVec;

    fn make_manifest(agents: Vec<AgentId>, priorities: Vec<f32>) -> ConflictManifest {
        ConflictManifest {
            conflict_id: [0u8; 16],
            conflict_type: ConflictType::ActionContradiction,
            contending_agents: SmallVec::from_vec(agents),
            trace_id: [0u8; 16],
            context_embeddings: SmallVec::from_vec(vec![[0.0f32; EMBEDDING_DIM]; 2]),
            dynamic_priority_scores: SmallVec::from_vec(priorities),
        }
    }

    #[test]
    fn test_build_judge_prompt() {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::CompletionsClient::new("test-key").unwrap(),
        ));
        let engine = L2LlmAuditEngine::new(
            provider,
            L2LlmConfig {
                model_id: "gpt-4".to_string(),
                max_consecutive_failures: 3,
                ..Default::default()
            },
        );
        let manifest = make_manifest(vec![[1u8; 16], [2u8; 16]], vec![0.8, 0.3]);

        let prompt = engine.build_judge_prompt(&manifest);
        assert!(prompt.contains("resource lock contention") || prompt.contains("action contradiction"));
        assert!(prompt.contains("override|merge|prune"));
    }
}
