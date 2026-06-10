use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::core::types::AgentId;
use crate::l0::resource::BudgetGuard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub model_id: String,
    pub provider_id: String,
    pub system_prompt: String,
    pub max_tokens: u64,
    pub temperature: f64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model_id: String::new(),
            provider_id: String::new(),
            system_prompt: String::new(),
            max_tokens: 4000,
            temperature: 0.7,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub name: String,
    pub role: String,
    pub parent_id: Option<AgentId>,
    pub children: Vec<AgentId>,
    pub depth: u32,
    pub goal: String,
    pub config: AgentConfig,
    pub status: AgentStatus,
    pub result: Option<String>,
    pub child_results: Vec<(AgentId, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AgentStatus {
    Idle,
    Planning,
    AwaitingChildren,
    Aggregating,
    Completed,
    Failed,
}

pub struct AgentPool {
    agents: Vec<Agent>,
    pub(crate) provider: Option<Arc<crate::llm::LlmProvider>>,
    completions: HashMap<AgentId, Arc<Notify>>,
    /// Active budget guards keyed by agent ID.
    /// Guards are dropped (releasing budget) when agents complete or fail.
    budget_guards: HashMap<AgentId, BudgetGuard>,
}

impl Default for AgentPool {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentPool {
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            provider: None,
            completions: HashMap::new(),
            budget_guards: HashMap::new(),
        }
    }

    pub fn set_provider(&mut self, provider: crate::llm::LlmProvider) {
        self.provider = Some(Arc::new(provider));
    }

    pub fn add_agent(&mut self, agent: Agent) -> AgentId {
        let id = agent.id;
        self.completions.insert(id, Arc::new(Notify::new()));
        self.agents.push(agent);
        id
    }

    pub fn get_agent(&self, id: &AgentId) -> Option<&Agent> {
        self.agents.iter().find(|a| &a.id == id)
    }

    pub fn get_agent_mut(&mut self, id: &AgentId) -> Option<&mut Agent> {
        self.agents.iter_mut().find(|a| &a.id == id)
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn get_completion_notify(&self, id: &AgentId) -> Option<Arc<Notify>> {
        self.completions.get(id).cloned()
    }

    /// Attach a budget guard to an agent (called after spawn approval).
    ///
    /// The guard will be released (budget returned to pool) when the
    /// agent completes or fails.
    pub fn attach_budget_guard(&mut self, agent_id: AgentId, guard: BudgetGuard) {
        self.budget_guards.insert(agent_id, guard);
    }

    /// Release an agent's budget guard (typically when agent completes/fails).
    /// Drops the guard, which returns any unspent budget to the pool.
    pub fn release_budget_guard(&mut self, agent_id: &AgentId) {
        self.budget_guards.remove(agent_id);
    }

    pub fn notify_completed(&mut self, id: &AgentId) {
        if let Some(notify) = self.completions.get(id) {
            notify.notify_one();
        }
    }

    pub fn summary(&self) -> String {
        let total = self.agents.len();
        let running = self
            .agents
            .iter()
            .filter(|a| {
                matches!(
                    a.status,
                    AgentStatus::Planning | AgentStatus::AwaitingChildren | AgentStatus::Aggregating
                )
            })
            .count();
        let completed = self
            .agents
            .iter()
            .filter(|a| a.status == AgentStatus::Completed)
            .count();
        let failed = self.agents.iter().filter(|a| a.status == AgentStatus::Failed).count();
        format!(
            "Agents: {} total, {} running, {} completed, {} failed",
            total, running, completed, failed
        )
    }

    pub fn agent_id_str(id: &AgentId) -> String {
        format!("{:02x}{:02x}{:02x}{:02x}", id[0], id[1], id[2], id[3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_pool() {
        let mut pool = AgentPool::new();
        let agent = Agent {
            id: [1; 16],
            name: "test".to_string(),
            role: "tester".to_string(),
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "test goal".to_string(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
        };
        let id = pool.add_agent(agent);
        assert_eq!(pool.agents().len(), 1);
        assert!(pool.get_agent(&id).is_some());
    }

    #[test]
    fn test_agent_summary() {
        let mut pool = AgentPool::new();
        let agent = Agent {
            id: [1; 16],
            name: "worker".to_string(),
            role: "dev".to_string(),
            parent_id: None,
            children: Vec::new(),
            depth: 0,
            goal: "goal".to_string(),
            config: AgentConfig::default(),
            status: AgentStatus::Idle,
            result: None,
            child_results: Vec::new(),
        };
        pool.add_agent(agent);
        let summary = pool.summary();
        assert!(summary.contains("1 total"));
    }
}
