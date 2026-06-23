use std::collections::HashMap;
use std::sync::Arc;

use crate::core::types::{AgentId, TaskId, EMBEDDING_DIM};
use crate::runtime::embedding_analyzer::GoalAnalyzer;

#[derive(Debug, Clone)]
pub struct TaskOutcome {
    pub task_id: TaskId,
    pub agent_id: Option<AgentId>,
    pub role: String,
    pub success: bool,
    pub latency_ms: u64,
    pub tokens_input: u32,
    pub tokens_output: u32,
}

pub struct TaskOutcomeStore {
    outcomes: Vec<TaskOutcome>,
    by_role: HashMap<String, Vec<usize>>,
}

impl TaskOutcomeStore {
    pub fn new() -> Self { Self { outcomes: Vec::new(), by_role: HashMap::new() } }
    pub fn record(&mut self, o: TaskOutcome) {
        let idx = self.outcomes.len();
        let role = o.role.clone();
        self.outcomes.push(o);
        self.by_role.entry(role).or_default().push(idx);
    }
    pub fn failure_rate(&self, _keywords: &[&str]) -> f32 {
        if self.outcomes.is_empty() { return 0.0; }
        self.outcomes.iter().filter(|o| !o.success).count() as f32 / self.outcomes.len() as f32
    }
    pub fn failure_rate_by_role(&self, role: &str) -> f32 {
        self.by_role.get(role).map(|indices| {
            if indices.is_empty() { return 0.0; }
            indices.iter().filter(|&&idx| !self.outcomes[idx].success).count() as f32 / indices.len() as f32
        }).unwrap_or(0.0)
    }
    pub fn recent(&self, n: usize) -> &[TaskOutcome] {
        let start = self.outcomes.len().saturating_sub(n);
        &self.outcomes[start..]
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityProfile {
    pub role: String,
    pub success_rate: f32,
    pub avg_latency_ms: u64,
    pub avg_token_cost: u32,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub embedding: Option<[f32; EMBEDDING_DIM]>,
}

pub struct CapabilityRegistry {
    profiles: HashMap<String, CapabilityProfile>,
}

impl CapabilityRegistry {
    pub fn new() -> Self { Self { profiles: HashMap::new() } }
    pub fn get(&self, role: &str) -> Option<&CapabilityProfile> { self.profiles.get(role) }
    pub fn get_mut(&mut self, role: &str) -> Option<&mut CapabilityProfile> { self.profiles.get_mut(role) }
    pub fn all(&self) -> Vec<CapabilityProfile> { self.profiles.values().cloned().collect() }
    pub fn role_prototypes(&self) -> HashMap<String, [f32; EMBEDDING_DIM]> {
        self.profiles.iter().filter_map(|(role, p)| p.embedding.map(|e| (role.clone(), e))).collect()
    }
    pub fn record_outcome(&mut self, outcome: &TaskOutcome) {
        let entry = self.profiles.entry(outcome.role.clone()).or_insert(CapabilityProfile {
            role: outcome.role.clone(), success_rate: 0.0, avg_latency_ms: 0, avg_token_cost: 0,
            completed_tasks: 0, failed_tasks: 0, embedding: None,
        });
        let total = entry.completed_tasks + entry.failed_tasks + 1;
        entry.avg_latency_ms = ((entry.avg_latency_ms as u64 * (total - 1).max(1) as u64) + outcome.latency_ms) / total as u64;
        entry.avg_token_cost = ((entry.avg_token_cost as u32 * (total - 1).max(1) as u32) + outcome.tokens_input + outcome.tokens_output) / total as u32;
        if outcome.success { entry.completed_tasks += 1; } else { entry.failed_tasks += 1; }
        entry.success_rate = entry.completed_tasks as f32 / (entry.completed_tasks + entry.failed_tasks).max(1) as f32;
    }
}

#[derive(Debug, Clone)]
pub struct RoleScore {
    pub role: String,
    pub total_score: f32,
    pub skill_match: f32,
    pub success_score: f32,
    pub latency_score: f32,
    pub cost_score: f32,
}

#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub role: String,
    pub confidence: f32,
    pub capability_score: f32,
    pub skill_match: f32,
}

pub trait RoleSelector: Send + Sync {
    fn score_all(&self, task: &crate::runtime::task_graph::TaskNode, candidates: &[CapabilityProfile]) -> Vec<RoleScore>;
    fn select(&self, task: &crate::runtime::task_graph::TaskNode, candidates: &[CapabilityProfile]) -> RoutingDecision {
        let scored = self.score_all(task, candidates);
        scored.into_iter().max_by(|a, b| a.total_score.partial_cmp(&b.total_score).unwrap())
            .map(|top| RoutingDecision { role: top.role, confidence: top.total_score, capability_score: top.success_score, skill_match: top.skill_match })
            .unwrap_or(RoutingDecision { role: "worker".to_string(), confidence: 0.0, capability_score: 0.0, skill_match: 0.0 })
    }
}

pub struct DefaultRoleSelector {
    analyzer: Arc<dyn GoalAnalyzer>,
}

impl DefaultRoleSelector {
    pub fn new(analyzer: Arc<dyn GoalAnalyzer>) -> Self { Self { analyzer } }
    fn skill_match(goal: &str, role: &str, analyzer: &dyn GoalAnalyzer) -> f32 {
        match analyzer.estimate_role(goal) {
            Some((best_role, conf)) if best_role == role => conf,
            Some(_) => 0.3,
            None => 0.5,
        }
    }
}

impl RoleSelector for DefaultRoleSelector {
    fn score_all(&self, task: &crate::runtime::task_graph::TaskNode, candidates: &[CapabilityProfile]) -> Vec<RoleScore> {
        if candidates.is_empty() {
            let best = self.analyzer.estimate_role(&task.goal).map(|(r, _)| r).unwrap_or_else(|| "developer".to_string());
            return vec![RoleScore { role: best, total_score: 1.0, skill_match: 1.0, success_score: 0.0, latency_score: 0.5, cost_score: 0.5 }];
        }
        candidates.iter().map(|c| {
            let skill = Self::skill_match(&task.goal, &c.role, &*self.analyzer);
            let lat_norm = 1.0 - (c.avg_latency_ms as f32 / 10_000.0).clamp(0.0, 1.0);
            let cost_norm = 1.0 - (c.avg_token_cost as f32 / 10_000.0).clamp(0.0, 1.0);
            let total = 0.40 * skill + 0.30 * c.success_rate + 0.20 * lat_norm + 0.10 * cost_norm;
            RoleScore { role: c.role.clone(), total_score: total, skill_match: skill, success_score: c.success_rate, latency_score: lat_norm, cost_score: cost_norm }
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::embedding_analyzer::MockGoalAnalyzer;

    #[test]
    fn test_capability_registry_prototypes_empty_by_default() {
        let reg = CapabilityRegistry::new();
        assert!(reg.role_prototypes().is_empty());
    }

    #[test]
    fn test_default_role_selector_returns_developer_on_empty() {
        let task = crate::runtime::task_graph::TaskNode::new([0u8; 16], "Build API");
        let selector = DefaultRoleSelector::new(Arc::new(MockGoalAnalyzer {
            domain_count: 1, ambiguity: 0.0, role: Some(("developer".into(), 1.0)),
        }));
        let scores = selector.score_all(&task, &[]);
        assert_eq!(scores.len(), 1);
        assert_eq!(scores[0].role, "developer");
    }
}
