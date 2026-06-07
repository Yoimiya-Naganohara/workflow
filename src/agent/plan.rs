use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agent::{Agent, AgentConfig, AgentPool, AgentStatus};
use crate::core::types::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub goal: String,
    pub tasks: Vec<Task>,
    pub status: PlanStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntity {
    pub plan_name: String,
    pub agent_id: AgentId,
    pub parent_plan: Option<String>,
    pub goal: String,
    pub tasks: Vec<Task>,
    pub status: PlanStatus,
    pub created_at: u64,
}

pub struct PlanRegistry {
    plans: HashMap<String, PlanEntity>,
    by_agent: HashMap<AgentId, Vec<String>>,
}

impl PlanRegistry {
    pub fn new() -> Self {
        Self {
            plans: HashMap::new(),
            by_agent: HashMap::new(),
        }
    }

    pub fn insert(&mut self, entity: PlanEntity) {
        let name = entity.plan_name.clone();
        let agent_id = entity.agent_id;
        self.plans.insert(name.clone(), entity);
        self.by_agent.entry(agent_id).or_default().push(name);
    }

    pub fn get_by_name(&self, name: &str) -> Option<&PlanEntity> {
        self.plans.get(name)
    }

    pub fn get_by_agent(&self, agent_id: AgentId) -> Vec<&PlanEntity> {
        self.by_agent
            .get(&agent_id)
            .map(|names| names.iter().filter_map(|n| self.plans.get(n)).collect())
            .unwrap_or_default()
    }

    pub fn search(&self, query: &str) -> Vec<&PlanEntity> {
        let q = query.to_lowercase();
        self.plans
            .values()
            .filter(|e| e.plan_name.to_lowercase().contains(&q) || e.goal.to_lowercase().contains(&q))
            .collect()
    }

    pub fn all(&self) -> Vec<&PlanEntity> {
        self.plans.values().collect()
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }
}

/// Registry mapping agents to their plans and tasks.
pub trait PlanRegistryOps: Send + Sync {
    fn insert(&mut self, entity: PlanEntity);
    fn get_by_name(&self, name: &str) -> Option<PlanEntity>;
    fn get_by_agent(&self, agent_id: AgentId) -> Vec<PlanEntity>;
    fn search(&self, query: &str) -> Vec<PlanEntity>;
    fn all(&self) -> Vec<PlanEntity>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool;
}

impl PlanRegistryOps for PlanRegistry {
    fn insert(&mut self, entity: PlanEntity) {
        self.insert(entity)
    }

    fn get_by_name(&self, name: &str) -> Option<PlanEntity> {
        self.get_by_name(name).cloned()
    }

    fn get_by_agent(&self, agent_id: AgentId) -> Vec<PlanEntity> {
        self.get_by_agent(agent_id).into_iter().cloned().collect()
    }

    fn search(&self, query: &str) -> Vec<PlanEntity> {
        self.search(query).into_iter().cloned().collect()
    }

    fn all(&self) -> Vec<PlanEntity> {
        self.all().into_iter().cloned().collect()
    }

    fn len(&self) -> usize {
        self.len()
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }
}

impl Default for PlanRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: usize,
    pub description: String,
    pub status: TaskStatus,
    pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PlanStatus {
    Draft,
    Approved,
    Executing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

impl Plan {
    pub fn new(goal: &str) -> Self {
        Self {
            goal: goal.to_string(),
            tasks: Vec::new(),
            status: PlanStatus::Draft,
        }
    }

    /// Known roles that can appear in @role syntax.
    const ROLES: &[&str] = &["planner", "developer", "tester", "reviewer", "worker"];

    /// Parse a plan from an LLM response.
    ///
    /// Supports three formats:
    /// - `- task description`
    /// - `1. task description`
    /// - `@role "goal description"`
    pub fn parse_from_response(response: &str) -> Option<Self> {
        let mut tasks = Vec::new();
        let mut task_id = 0;

        for line in response.lines() {
            let trimmed = line.trim();

            // Try @role "goal" format first
            if let Some((role, goal)) = Self::parse_role_assignment(trimmed) {
                task_id += 1;
                tasks.push(Task {
                    id: task_id,
                    description: format!("@{} {}", role, goal),
                    status: TaskStatus::Pending,
                    result: None,
                });
                continue;
            }

            // Match lines like "1. Task description" or "- Task description"
            if let Some(task_desc) = trimmed
                .strip_prefix("- ")
                .or_else(|| {
                    trimmed
                        .strip_prefix(|c: char| c.is_ascii_digit())
                        .map(|s| s.trim_start_matches('.').trim_start())
                })
                .filter(|s| !s.is_empty())
            {
                task_id += 1;
                tasks.push(Task {
                    id: task_id,
                    description: task_desc.to_string(),
                    status: TaskStatus::Pending,
                    result: None,
                });
            }
        }

        if tasks.is_empty() {
            None
        } else {
            Some(Plan {
                goal: String::new(),
                tasks,
                status: PlanStatus::Draft,
            })
        }
    }

    /// Parse a single `@role "goal"` assignment from a line.
    fn parse_role_assignment(line: &str) -> Option<(String, String)> {
        for role in Self::ROLES {
            let pattern = format!("@{}", role);
            if let Some(pos) = line.find(&pattern) {
                let after_role = line[pos + pattern.len()..].trim();
                let goal_start = after_role.find('"').or_else(|| after_role.find('\u{201c}'))?;
                let quote_char = after_role.as_bytes()[goal_start];
                let closing = if quote_char == b'"' { '"' } else { '\u{201d}' };
                let after_open = &after_role[goal_start + 1..];
                let goal_end = after_open.find(closing)?;
                let goal = after_open[..goal_end].to_string();
                if !goal.is_empty() {
                    return Some((role.to_string(), goal));
                }
            }
        }
        None
    }

    pub fn approve(&mut self) {
        self.status = PlanStatus::Approved;
    }

    pub fn next_task(&self) -> Option<&Task> {
        self.tasks.iter().find(|t| t.status == TaskStatus::Pending)
    }

    pub fn mark_task_running(&mut self, task_id: usize) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Running;
        }
    }

    pub fn mark_task_completed(&mut self, task_id: usize, result: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Completed;
            task.result = Some(result);
        }
        self.check_completion();
    }

    pub fn mark_task_failed(&mut self, task_id: usize, error: String) {
        if let Some(task) = self.tasks.iter_mut().find(|t| t.id == task_id) {
            task.status = TaskStatus::Failed;
            task.result = Some(error);
        }
        self.status = PlanStatus::Failed;
    }

    fn check_completion(&mut self) {
        if self.tasks.iter().all(|t| t.status == TaskStatus::Completed) {
            self.status = PlanStatus::Completed;
        }
    }

    pub fn is_executable(&self) -> bool {
        self.status == PlanStatus::Approved && self.tasks.iter().any(|t| t.status == TaskStatus::Pending)
    }

    pub async fn execute_plan(plan: &mut Plan, agent_pool: &Arc<RwLock<AgentPool>>) -> Result<()> {
        plan.status = PlanStatus::Executing;

        while let Some(task) = plan.next_task() {
            let task_clone = task.clone();
            let config = AgentConfig::default();

            let (agent_id, provider) = {
                let mut pool = agent_pool.write().await;
                let agent = Agent {
                    id: rand::random(),
                    name: format!("Worker {}", pool.agents().len() + 1),
                    role: "worker".to_string(),
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: task_clone.description.clone(),
                    config: config.clone(),
                    status: AgentStatus::Planning,
                    result: None,
                    child_results: Vec::new(),
                };
                let agent_id = agent.id;
                pool.add_agent(agent);
                (agent_id, pool.provider.clone())
            };

            plan.mark_task_running(task_clone.id);

            let result: Result<String> = if let Some(provider) = provider {
                provider
                    .chat(&config.model_id, &config.system_prompt, &task_clone.description)
                    .await
            } else {
                Err(anyhow::anyhow!("No provider configured"))
            };

            {
                let mut pool = agent_pool.write().await;
                match &result {
                    Ok(response) => {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.status = AgentStatus::Completed;
                            agent.result = Some(response.clone());
                        }
                    }
                    Err(e) => {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.status = AgentStatus::Failed;
                            agent.result = Some(e.to_string());
                        }
                    }
                }
            }

            match result {
                Ok(response) => {
                    plan.mark_task_completed(task_clone.id, response);
                }
                Err(e) => {
                    plan.mark_task_failed(task_clone.id, e.to_string());
                    break;
                }
            }
        }

        Ok(())
    }

    pub fn summary(&self) -> String {
        let completed = self.tasks.iter().filter(|t| t.status == TaskStatus::Completed).count();
        let total = self.tasks.len();
        format!(
            "[{}/{}] {}",
            completed,
            total,
            match self.status {
                PlanStatus::Draft => "Draft",
                PlanStatus::Approved => "Approved",
                PlanStatus::Executing => "Executing",
                PlanStatus::Completed => "Completed",
                PlanStatus::Failed => "Failed",
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan() {
        let response = "Here's the plan:\n1. First task\n2. Second task\n3. Third task";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].description, "First task");
        assert_eq!(plan.tasks[1].description, "Second task");
        assert_eq!(plan.tasks[2].description, "Third task");
    }

    #[test]
    fn test_parse_plan_with_dashes() {
        let response = "- Task one\n- Task two";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
    }

    #[test]
    fn test_plan_lifecycle() {
        let mut plan = Plan::new("test goal");
        plan.tasks.push(Task {
            id: 1,
            description: "task 1".to_string(),
            status: TaskStatus::Pending,
            result: None,
        });
        plan.tasks.push(Task {
            id: 2,
            description: "task 2".to_string(),
            status: TaskStatus::Pending,
            result: None,
        });

        plan.approve();
        assert_eq!(plan.status, PlanStatus::Approved);
        assert!(plan.is_executable());

        plan.mark_task_running(1);
        assert_eq!(plan.tasks[0].status, TaskStatus::Running);

        plan.mark_task_completed(1, "done".to_string());
        assert_eq!(plan.tasks[0].status, TaskStatus::Completed);
        assert_eq!(plan.status, PlanStatus::Approved); // Not completed yet

        plan.mark_task_completed(2, "done".to_string());
        assert_eq!(plan.status, PlanStatus::Completed);
        assert!(!plan.is_executable());
    }
}
