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
            .filter(|e| {
                e.plan_name.to_lowercase().contains(&q) || e.goal.to_lowercase().contains(&q)
            })
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
    /// Supports full markdown format:
    /// - Section headers (`## Section`) as task grouping context
    /// - Unordered lists (`-`, `*`, `+`)
    /// - Ordered lists (`1.`, `1)`)
    /// - Task lists with checkboxes (`- [ ] todo`, `- [x] done`)
    /// - `@role "goal description"` format
    /// - Code blocks (fenced with ```)
    /// - Continuation lines (indented content under a task item)
    pub fn parse_from_response(response: &str) -> Option<Self> {
        let mut tasks = Vec::new();
        let mut task_id: usize = 0;
        let lines: Vec<&str> = response.lines().collect();
        let mut i = 0;
        let mut current_section = String::new();

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Skip empty lines
            if trimmed.is_empty() {
                i += 1;
                continue;
            }

            // Detect markdown header as section context
            if let Some(level) = Self::header_level(trimmed) {
                current_section = trimmed[level..].trim().to_string();
                i += 1;
                continue;
            }

            // Try @role "goal" format first
            if let Some((role, goal)) = Self::parse_role_assignment(trimmed) {
                task_id += 1;
                let desc = if current_section.is_empty() {
                    format!("@{} {}", role, goal)
                } else {
                    format!("[{}] @{} {}", current_section, role, goal)
                };
                tasks.push(Task {
                    id: task_id,
                    description: desc,
                    status: TaskStatus::Pending,
                    result: None,
                });
                i += 1;
                continue;
            }

            // Handle code blocks — skip entirely (not task content)
            if trimmed.starts_with("```") {
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with("```") {
                    i += 1;
                }
                i += 1;
                continue;
            }

            // Try checkbox / task list: - [ ] or - [x]
            if let Some((marker, text)) = Self::parse_checkbox(trimmed) {
                task_id += 1;
                let full_text = format!("{} {}", marker, text);
                let desc = if current_section.is_empty() {
                    full_text
                } else {
                    format!("[{}] {}", current_section, full_text)
                };
                tasks.push(Task {
                    id: task_id,
                    description: desc,
                    status: TaskStatus::Pending,
                    result: None,
                });
                i = Self::collect_continuation(&lines, i + 1, &mut tasks);
                continue;
            }

            // Try unordered list: -, *, +
            if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
                .or_else(|| trimmed.strip_prefix("+ "))
                .filter(|s| !s.is_empty())
            {
                task_id += 1;
                let desc = if current_section.is_empty() {
                    rest.to_string()
                } else {
                    format!("[{}] {}", current_section, rest)
                };
                tasks.push(Task {
                    id: task_id,
                    description: desc,
                    status: TaskStatus::Pending,
                    result: None,
                });
                i = Self::collect_continuation(&lines, i + 1, &mut tasks);
                continue;
            }

            // Try ordered list: 1. text, 1) text
            if let Some(rest) = Self::parse_ordered_item(trimmed) {
                task_id += 1;
                let desc = if current_section.is_empty() {
                    rest.to_string()
                } else {
                    format!("[{}] {}", current_section, rest)
                };
                tasks.push(Task {
                    id: task_id,
                    description: desc,
                    status: TaskStatus::Pending,
                    result: None,
                });
                i = Self::collect_continuation(&lines, i + 1, &mut tasks);
                continue;
            }

            i += 1;
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

    /// Detect markdown header level (1-6) from a trimmed line.
    fn header_level(s: &str) -> Option<usize> {
        let mut count = 0;
        for ch in s.chars() {
            if ch == '#' {
                count += 1;
            } else {
                break;
            }
        }
        if (1..=6).contains(&count) && s.chars().nth(count) == Some(' ') {
            Some(count)
        } else {
            None
        }
    }

    /// Parse a checkbox/task list item: `- [ ] text` or `- [x] text`.
    /// Returns `Some((marker, text))` or `None`.
    fn parse_checkbox(s: &str) -> Option<(String, &str)> {
        let s = s.trim_start();
        if s.len() >= 6
            && s.as_bytes()[0] == b'-'
            && s.as_bytes()[1] == b' '
            && s.as_bytes()[2] == b'['
        {
            let checked = s.as_bytes()[3] == b'x' || s.as_bytes()[3] == b'X';
            if s.as_bytes().get(4) == Some(&b']') && s.as_bytes().get(5) == Some(&b' ') {
                let marker = if checked { "☑" } else { "☐" };
                Some((marker.to_string(), &s[6..]))
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Parse an ordered list item: `1. text`, `1) text`, `10. text`, etc.
    fn parse_ordered_item(s: &str) -> Option<&str> {
        let bytes = s.as_bytes();
        let mut digit_end = 0;
        while digit_end < bytes.len() && bytes[digit_end].is_ascii_digit() {
            digit_end += 1;
        }
        if digit_end == 0 {
            return None;
        }
        let rest = &s[digit_end..];
        if let Some(text) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            if !text.is_empty() {
                return Some(text);
            }
        }
        None
    }

    /// Collect continuation lines (indented content) under the last task.
    #[allow(clippy::ptr_arg)]
    fn collect_continuation(lines: &[&str], start: usize, tasks: &mut Vec<Task>) -> usize {
        let mut i = start;
        while i < lines.len() {
            let next = lines[i];
            if next.trim().is_empty() {
                i += 1;
                continue;
            }
            // Stop at another list item or header
            let t = next.trim();
            if t.starts_with('-') || t.starts_with('*') || t.starts_with('+') {
                break;
            }
            if Self::header_level(t).is_some() {
                break;
            }
            // Skip code blocks entirely
            if t.starts_with("```") {
                i += 1;
                while i < lines.len() && !lines[i].trim().starts_with("```") {
                    i += 1;
                }
                i += 1;
                continue;
            }
            if Self::parse_ordered_item(t).is_some() {
                break;
            }
            // Indented continuation
            if next.starts_with(' ') || next.starts_with('\t') {
                if let Some(task) = tasks.last_mut() {
                    task.description.push('\n');
                    task.description.push_str(t.trim());
                }
                i += 1;
            } else {
                break;
            }
        }
        i
    }

    /// Parse a single `@role "goal"` assignment from a line.
    pub fn parse_role_assignment(line: &str) -> Option<(String, String)> {
        for role in Self::ROLES {
            let pattern = format!("@{}", role);
            if let Some(pos) = line.find(&pattern) {
                let after_role = line[pos + pattern.len()..].trim();
                let goal_start = after_role
                    .find('"')
                    .or_else(|| after_role.find('\u{201c}'))?;
                // Use char-based advance — the found quote may be multi-byte UTF-8.
                let quote = after_role[goal_start..].chars().next()?;
                let closing = match quote {
                    '"' => '"',
                    '\u{201c}' => '\u{201d}',
                    _ => return None,
                };
                let after_open = &after_role[goal_start + quote.len_utf8()..];
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
        self.status == PlanStatus::Approved
            && self.tasks.iter().any(|t| t.status == TaskStatus::Pending)
    }

    /// Execute a plan by creating an agent for each task and running it.
    ///
    /// NOTE: This function is preserved for future use but is NOT currently
    /// called from anywhere in the codebase. When wired up, a `model_id`
    /// must be provided — `AgentConfig::default()` has an empty `model_id`.
    pub async fn execute_plan(
        plan: &mut Plan,
        model_id: &str,
        agent_pool: &Arc<RwLock<AgentPool>>,
    ) -> Result<()> {
        plan.status = PlanStatus::Executing;

        while let Some(task) = plan.next_task() {
            let task_clone = task.clone();
            let config = AgentConfig {
                model_id: model_id.to_string(),
                ..Default::default()
            };

            let (agent_id, provider) = {
                let mut pool = agent_pool.write().await;
                let agent = Agent {
                    id: rand::random(),
                    name: format!("Worker-{:04x}", rand::random::<u16>()),
                    role: "worker".to_string(),
                    role_template_id: None,
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: task_clone.description.clone(),
                    config: config.clone(),
                    status: AgentStatus::Planning,
                    result: None,
                    child_results: Vec::new(),
                    context: Vec::new(),
                    last_active_at: crate::agent::now_secs(),
                    tokens_input: 0,
                    tokens_output: 0,
                    tool_trace: std::collections::VecDeque::new(),
                    inbox: std::collections::VecDeque::new(),
                    task_id: None,
                    sandbox: None,
                };
                let agent_id = agent.id;
                pool.add_agent(agent);
                (agent_id, pool.provider.clone())
            };

            plan.mark_task_running(task_clone.id);

            let system_prompt = "You are a worker. Execute the given task.".to_string();
            let result: Result<String> = if let Some(provider) = provider {
                provider
                    .chat(&config.model_id, &system_prompt, &task_clone.description)
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

        // Return the last error if any task failed.
        if plan
            .tasks
            .iter()
            .any(|t| t.status == crate::agent::plan::TaskStatus::Failed)
        {
            Err(anyhow::anyhow!("One or more plan tasks failed"))
        } else {
            Ok(())
        }
    }

    pub fn summary(&self) -> String {
        let completed = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .count();
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

    /// Convert this linear plan into a graph-backed [`GraphPlan`].
    ///
    /// Each task becomes a child of the root.  Dependencies between
    /// tasks must be added separately via [`GraphPlan::add_dependency`].
    pub fn into_graph(self) -> GraphPlan {
        let mut graph = crate::runtime::task_graph::TaskGraph::new();
        let root_id = graph.spawn_root(&self.goal);

        for task in &self.tasks {
            if let Some(child_id) = graph.spawn_child(root_id, &task.description) {
                // Map old status to new status
                match task.status {
                    crate::agent::plan::TaskStatus::Pending => {
                        // stays Created — resolved by ready_tasks()
                    }
                    crate::agent::plan::TaskStatus::Running => {
                        let _ = graph.mark_ready(child_id);
                    }
                    crate::agent::plan::TaskStatus::Completed => {
                        let _ = graph.mark_ready(child_id);
                        let _ = graph.mark_complete(child_id);
                    }
                    crate::agent::plan::TaskStatus::Failed => {
                        let _ = graph.mark_failed(child_id, "");
                    }
                }
                // Store result if any
                if let Some(ref result) = task.result {
                    if let Some(node) = graph.get_mut(&child_id) {
                        node.result = Some(result.clone());
                    }
                }
            }
        }

        GraphPlan {
            graph,
            root_id,
            status: self.status,
            plan_name: self.goal.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        }
    }
}

// ============================================================================
//  GraphPlan — DAG-backed plan using TaskGraph
// ============================================================================

/// A plan backed by a [`TaskGraph`] instead of a flat `Vec<Task>`.
///
/// This is the Phase 1 replacement for the linear `Plan` struct.
/// It provides:
///
/// - Decomposition hierarchy (parent/children)
/// - Dependency tracking (run-after constraints)
/// - Fan-in/fan-out via DAG scheduling
/// - Agent assignment per node
///
/// # Migration path
///
/// 1. New code uses `GraphPlan` directly.
/// 2. Old code continues using `Plan` and `PlanRegistry`.
/// 3. `Plan::into_graph()` provides the bridge.
/// 4. Future: `PlanGraphRegistry` replaces `PlanRegistry`.
#[derive(Debug, Clone)]
pub struct GraphPlan {
    /// The underlying DAG.
    pub graph: crate::runtime::task_graph::TaskGraph,
    /// The root task node ID.
    pub root_id: crate::core::types::TaskId,
    /// Plan-level status (backport from old PlanStatus).
    pub status: PlanStatus,
    /// Plan name / identifier.
    pub plan_name: String,
    /// Creation timestamp.
    pub created_at: u64,
}

impl GraphPlan {
    /// Create a new graph plan with a single root goal.
    pub fn new(goal: &str) -> Self {
        let mut graph = crate::runtime::task_graph::TaskGraph::new();
        let root_id = graph.spawn_root(goal);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        Self {
            graph,
            root_id,
            status: PlanStatus::Draft,
            plan_name: goal.to_string(),
            created_at: now,
        }
    }

    /// Create a new graph plan with a specific root task ID (for replay/persistence).
    pub fn from_graph(
        graph: crate::runtime::task_graph::TaskGraph,
        root_id: crate::core::types::TaskId,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let plan_name = graph
            .get(&root_id)
            .map_or("unknown".to_string(), |n| n.goal.clone());
        Self {
            graph,
            root_id,
            status: PlanStatus::Draft,
            plan_name,
            created_at: now,
        }
    }

    /// Add a sub-task as a child of the root.
    pub fn spawn_subtask(&mut self, goal: &str) -> Option<crate::core::types::TaskId> {
        self.graph.spawn_child(self.root_id, goal)
    }

    /// Add a sub-task as a child of a specific parent.
    pub fn spawn_child_of(
        &mut self,
        parent_id: crate::core::types::TaskId,
        goal: &str,
    ) -> Option<crate::core::types::TaskId> {
        self.graph.spawn_child(parent_id, goal)
    }

    /// Add a dependency edge between two tasks.
    pub fn add_dependency(
        &mut self,
        task_id: crate::core::types::TaskId,
        depends_on: crate::core::types::TaskId,
    ) -> Result<(), String> {
        self.graph.add_dependency(task_id, depends_on)
    }

    /// Returns the list of tasks ready for execution (dependencies satisfied).
    pub fn ready_tasks(&self) -> Vec<crate::core::types::TaskId> {
        self.graph.ready_tasks()
    }

    /// Mark a task as running (assigned to an agent).
    pub fn mark_running(
        &mut self,
        task_id: crate::core::types::TaskId,
        agent_id: AgentId,
    ) -> Result<(), String> {
        self.graph.mark_running(task_id, agent_id)
    }

    /// Mark a task as completed.  May trigger parent completion.
    pub fn mark_complete(
        &mut self,
        task_id: crate::core::types::TaskId,
        result: &str,
    ) -> Result<(), String> {
        if let Some(node) = self.graph.get_mut(&task_id) {
            node.result = Some(result.to_string());
        }
        self.graph.mark_complete(task_id)?;

        // Check plan-level completion
        if self.graph.subgraph_complete(self.root_id) {
            self.status = PlanStatus::Completed;
        }
        Ok(())
    }

    /// Mark a task as failed.
    pub fn mark_failed(
        &mut self,
        task_id: crate::core::types::TaskId,
        error: &str,
    ) -> Result<(), String> {
        self.graph.mark_failed(task_id, error)?;
        self.status = PlanStatus::Failed;
        Ok(())
    }

    /// Mark the root as Decomposed (it has spawned children).
    pub fn mark_decomposed(&mut self) -> Result<(), String> {
        self.graph.mark_decomposed(self.root_id)
    }

    /// Get all leaf tasks (no children) in the decomposition tree.
    /// These are the tasks that need agents assigned.
    pub fn leaf_tasks(&self) -> Vec<crate::core::types::TaskId> {
        self.graph.leaf_tasks()
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        let counts = self.graph.status_counts(Some(self.root_id));
        let total: usize = counts.values().sum();
        let done = counts
            .get(&crate::runtime::task_graph::TaskStatus::Completed)
            .copied()
            .unwrap_or(0);
        format!(
            "[{}/{}] {} | {}",
            done,
            total,
            match self.status {
                PlanStatus::Draft => "Draft",
                PlanStatus::Approved => "Approved",
                PlanStatus::Executing => "Executing",
                PlanStatus::Completed => "Completed",
                PlanStatus::Failed => "Failed",
            },
            self.graph,
        )
    }
}

// ============================================================================
//  PlanGraphRegistry — stores graph-backed plans by ID
// ============================================================================

/// Registry for [`GraphPlan`] entries.
///
/// This is the graph-native replacement for [`PlanRegistry`].
/// Unlike [`PlanRegistry`] which stores flat `PlanEntity` lists,
/// this registry stores full DAGs keyed by plan name.
pub struct PlanGraphRegistry {
    plans: HashMap<String, GraphPlan>,
    by_agent: HashMap<AgentId, Vec<String>>,
}

impl PlanGraphRegistry {
    pub fn new() -> Self {
        Self {
            plans: HashMap::new(),
            by_agent: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: &str, plan: GraphPlan, agent_id: Option<AgentId>) {
        let name = name.to_string();
        self.plans.insert(name.clone(), plan);
        if let Some(aid) = agent_id {
            self.by_agent.entry(aid).or_default().push(name);
        }
    }

    pub fn get(&self, name: &str) -> Option<&GraphPlan> {
        self.plans.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut GraphPlan> {
        self.plans.get_mut(name)
    }

    pub fn get_by_agent(&self, agent_id: AgentId) -> Vec<&GraphPlan> {
        self.by_agent
            .get(&agent_id)
            .map(|names| names.iter().filter_map(|n| self.plans.get(n)).collect())
            .unwrap_or_default()
    }

    pub fn search(&self, query: &str) -> Vec<&GraphPlan> {
        let q = query.to_lowercase();
        self.plans
            .values()
            .filter(|p| p.plan_name.to_lowercase().contains(&q))
            .collect()
    }

    pub fn all(&self) -> Vec<&GraphPlan> {
        self.plans.values().collect()
    }

    pub fn len(&self) -> usize {
        self.plans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }

    /// Delete a plan by name.
    pub fn delete(&mut self, name: &str) -> bool {
        if self.plans.remove(name).is_some() {
            // Clean up by_agent references
            for v in self.by_agent.values_mut() {
                v.retain(|n| n != name);
            }
            true
        } else {
            false
        }
    }

    /// Get all plan names that reference a given task ID.
    pub fn find_by_task(&self, task_id: crate::core::types::TaskId) -> Vec<&str> {
        self.plans
            .iter()
            .filter(|(_, plan)| plan.graph.contains(&task_id))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}

impl Default for PlanGraphRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_plan_dash() {
        let response = "- Task one\n- Task two";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].description, "Task one");
        assert_eq!(plan.tasks[1].description, "Task two");
    }

    #[test]
    fn test_parse_plan_ordered() {
        let response = "Here's the plan:\n1. First task\n2. Second task\n3. Third task";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].description, "First task");
        assert_eq!(plan.tasks[1].description, "Second task");
        assert_eq!(plan.tasks[2].description, "Third task");
    }

    #[test]
    fn test_parse_plan_star_list() {
        let response = "* Task A\n* Task B\n* Task C";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 3);
        assert_eq!(plan.tasks[0].description, "Task A");
    }

    #[test]
    fn test_parse_plan_plus_list() {
        let response = "+ Task X\n+ Task Y";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[1].description, "Task Y");
    }

    #[test]
    fn test_parse_plan_checkboxes() {
        let response = "- [ ] Todo item\n- [x] Done item";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert!(plan.tasks[0].description.starts_with("☐"));
        assert!(plan.tasks[1].description.starts_with("☑"));
    }

    #[test]
    fn test_parse_plan_with_headers() {
        let response =
            "## Phase 1\n- Setup environment\n- Install deps\n\n## Phase 2\n- Run tests\n- Deploy";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 4);
        assert!(plan.tasks[0].description.contains("Phase 1"));
        assert!(plan.tasks[2].description.contains("Phase 2"));
    }

    #[test]
    fn test_parse_plan_with_code_block() {
        let response = "- Write script\n  ```\n  echo hello\n  ```\n  More detail\n- Next task";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].description, "Write script\nMore detail");
    }

    #[test]
    fn test_parse_plan_with_continuation() {
        let response = "- Main task\n  with detail\n  and more\n- Next task";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(
            plan.tasks[0].description,
            "Main task\nwith detail\nand more"
        );
    }

    #[test]
    fn test_parse_plan_with_bold_markdown() {
        let response = "- Implement **login** feature\n- Add `error` handling";
        let plan = Plan::parse_from_response(response).unwrap();
        assert_eq!(plan.tasks.len(), 2);
        assert_eq!(plan.tasks[0].description, "Implement **login** feature");
        assert_eq!(plan.tasks[1].description, "Add `error` handling");
    }

    #[test]
    fn test_parse_plan_empty() {
        assert!(Plan::parse_from_response("Just some text without list items").is_none());
        assert!(Plan::parse_from_response("").is_none());
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
