use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use tokio::task::JoinSet;
use workflow_agent::{Agent, AgentEvent, AgentId, Message, agent_pool::AgentPool};

use crate::{RoleChecker, ToolError};

#[derive(Debug, Clone, Deserialize)]
pub struct TaskDef {
    pub id: String,
    pub role: String,
    pub task: String,
    #[serde(default)]
    pub depend_on: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct OrchestrateArgs {
    pub tasks: Vec<TaskDef>,
}

#[derive(Debug, Serialize)]
pub struct TaskAssignment {
    pub id: String,
    pub role: String,
    pub task: String,
    pub wave: usize,
    pub agent_id: AgentId,
}

#[derive(Debug, Serialize)]
pub struct ExecutionPlan {
    pub waves: Vec<Vec<TaskAssignment>>,
    pub task_count: usize,
    pub critical_path: Vec<String>,
}

pub type AgentFactory = Arc<dyn Fn(AgentId, String) -> Arc<Agent> + Send + Sync>;

struct PreAllocated {
    agent: Arc<Agent>,
    task_id: String,
    task: String,
    depend_on: Vec<String>,
}

pub struct Orchestrate {
    pool: Arc<AgentPool>,
    next_id: Arc<std::sync::atomic::AtomicU32>,
    factory: AgentFactory,
    roles: Arc<dyn RoleChecker>,
    create_role_flag: Arc<AtomicBool>,
}

impl Orchestrate {
    pub fn new(
        pool: Arc<AgentPool>,
        factory: AgentFactory,
        roles: Arc<dyn RoleChecker>,
        create_role_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            pool,
            next_id: Arc::new(std::sync::atomic::AtomicU32::new(1)),
            factory,
            roles,
            create_role_flag,
        }
    }

    pub fn with_id_allocator(
        pool: Arc<AgentPool>,
        factory: AgentFactory,
        next_id: Arc<std::sync::atomic::AtomicU32>,
        roles: Arc<dyn RoleChecker>,
        create_role_flag: Arc<AtomicBool>,
    ) -> Self {
        Self {
            pool,
            next_id,
            factory,
            roles,
            create_role_flag,
        }
    }

    async fn get_or_create_agent(
        &self,
        role: &str,
        reserved: &HashSet<AgentId>,
    ) -> Result<Arc<Agent>, ToolError> {
        let agents = self.pool.list_agents().await;
        for info in &agents {
            if info.role == role
                && info.current_task.is_none()
                && !reserved.contains(&info.id)
                && let Some(agent) = self.pool.get_agent(&info.id).await
            {
                return Ok(agent);
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let agent = (self.factory)(id, role.to_owned());
        self.pool
            .add_agent(agent.clone())
            .await
            .map_err(|e| ToolError::Orchestrate(format!("failed to create agent: {e}")))?;
        Ok(agent)
    }
}

impl Tool for Orchestrate {
    const NAME: &'static str = "orchestrate";

    type Error = ToolError;
    type Args = OrchestrateArgs;
    type Output = ExecutionPlan;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "orchestrate".to_string(),
            description: "Plan and dispatch multi-agent tasks. Accepts a DAG of \
                tasks with ids, roles, task descriptions, and dependencies. \
                Assigns agents by role (reuses idle agents or creates new ones), \
                executes tasks in wave order (parallel waves where possible), \
                and returns the execution plan with assigned agent IDs. \
                Execution continues in the background; results arrive via events."
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "List of tasks to orchestrate",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Unique task identifier"
                                },
                                "role": {
                                    "type": "string",
                                    "description": "Agent role to execute this task (e.g. 'researcher', 'coder', 'reviewer')"
                                },
                                "task": {
                                    "type": "string",
                                    "description": "Task description / goal"
                                },
                                "depend_on": {
                                    "type": "array",
                                    "description": "Ids of tasks that must complete before this one starts",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["id", "role", "task"]
                        }
                    }
                },
                "required": ["tasks"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let tasks = args.tasks;

        if tasks.is_empty() {
            return Err(ToolError::Orchestrate("task list is empty".into()));
        }

        let available = self.roles.list_roles();
        for task in &tasks {
            if !self.roles.exists(&task.role) {
                self.create_role_flag.store(true, Ordering::Relaxed);
                let suggestions = suggest_roles(&task.role, &available);
                return Err(ToolError::RoleNotFound {
                    requested: task.role.clone(),
                    suggestions,
                    available,
                });
            }
        }

        let mut by_id: HashMap<String, usize> = HashMap::new();
        for (i, t) in tasks.iter().enumerate() {
            if by_id.insert(t.id.clone(), i).is_some() {
                return Err(ToolError::Orchestrate(format!(
                    "duplicate task id: {}",
                    t.id
                )));
            }
        }

        for t in &tasks {
            for dep in &t.depend_on {
                if !by_id.contains_key(dep) {
                    return Err(ToolError::Orchestrate(format!(
                        "task '{}' depends on unknown task '{}'",
                        t.id, dep
                    )));
                }
            }
        }

        // Kahn's algorithm — topological sort + wave assignment
        let mut in_degree: Vec<usize> = vec![0; tasks.len()];
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); tasks.len()];

        for (i, t) in tasks.iter().enumerate() {
            for dep in &t.depend_on {
                let dep_idx = by_id[dep];
                dependents[dep_idx].push(i);
                in_degree[i] += 1;
            }
        }

        let mut queue: VecDeque<usize> = in_degree
            .iter()
            .enumerate()
            .filter(|(_, d)| **d == 0)
            .map(|(i, _)| i)
            .collect();

        let mut waves: Vec<Vec<usize>> = Vec::new();
        let mut id_to_wave: Vec<usize> = vec![0; tasks.len()];

        while !queue.is_empty() {
            let wave: Vec<usize> = queue.drain(..).collect();
            let wave_idx = waves.len();

            for &idx in &wave {
                id_to_wave[idx] = wave_idx;
                for &dep_idx in &dependents[idx] {
                    in_degree[dep_idx] -= 1;
                    if in_degree[dep_idx] == 0 {
                        queue.push_back(dep_idx);
                    }
                }
            }

            waves.push(wave);
        }

        let sorted_count: usize = waves.iter().map(|w| w.len()).sum();
        if sorted_count != tasks.len() {
            return Err(ToolError::Orchestrate(
                "dependency cycle detected — not all tasks could be scheduled".into(),
            ));
        }

        // Critical path
        let mut dist: Vec<usize> = vec![0; tasks.len()];
        let mut prev: Vec<Option<usize>> = vec![None; tasks.len()];

        for wave in &waves {
            for &idx in wave {
                for dep in &tasks[idx].depend_on {
                    let dep_idx = by_id[dep];
                    let d = dist[dep_idx] + 1;
                    if d > dist[idx] {
                        dist[idx] = d;
                        prev[idx] = Some(dep_idx);
                    }
                }
            }
        }

        let mut max_dist = 0;
        let mut max_idx = 0;
        for (i, &d) in dist.iter().enumerate() {
            if d >= max_dist {
                max_dist = d;
                max_idx = i;
            }
        }

        let mut critical_path = Vec::new();
        let mut cur = Some(max_idx);
        while let Some(idx) = cur {
            critical_path.push(tasks[idx].id.clone());
            cur = prev[idx];
        }
        critical_path.reverse();

        // ── Allocate agents synchronously ─────────────────────
        let mut preallocated: Vec<Vec<PreAllocated>> = Vec::new();
        let mut plan_waves: Vec<Vec<TaskAssignment>> = Vec::new();
        let mut all_reserved = HashSet::new();

        for wave in &waves {
            let mut wave_pre = Vec::new();
            let mut assignments = Vec::new();

            for &idx in wave {
                let task = &tasks[idx];
                let agent = self
                    .get_or_create_agent(&task.role, &all_reserved)
                    .await?;
                let agent_id = agent.id();
                all_reserved.insert(agent_id);

                wave_pre.push(PreAllocated {
                    agent,
                    task_id: task.id.clone(),
                    task: task.task.clone(),
                    depend_on: task.depend_on.clone(),
                });

                assignments.push(TaskAssignment {
                    id: task.id.clone(),
                    role: task.role.clone(),
                    task: task.task.clone(),
                    wave: id_to_wave[idx],
                    agent_id,
                });
            }

            preallocated.push(wave_pre);
            plan_waves.push(assignments);
        }

        let task_count = tasks.len();

        // ── Spawn background execution ────────────────────────
        tokio::spawn(async move {
            let mut task_results: HashMap<String, String> = HashMap::new();

            for wave in preallocated {
                let mut completions = JoinSet::new();

                for t in &wave {
                    let prompt = if t.depend_on.is_empty() {
                        t.task.clone()
                    } else {
                        let dependency_context = t
                            .depend_on
                            .iter()
                            .filter_map(|dependency| {
                                task_results
                                    .get(dependency)
                                    .map(|result| format!("[{dependency}]\n{result}"))
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n");
                        format!(
                            "{}\n\nDependency results:\n{}",
                            t.task, dependency_context
                        )
                    };

                    let mut receiver = t.agent.receiver();
                    if let Err(e) = t.agent.send(Message::User(prompt)).await {
                        eprintln!(
                            "orchestrate: failed to dispatch task '{}': {e}",
                            t.task_id
                        );
                        continue;
                    }

                    let task_id = t.task_id.clone();
                    completions.spawn(async move {
                        let mut output = String::new();
                        loop {
                            match receiver.recv().await {
                                Ok(AgentEvent::Text(text)) => output.push_str(&text),
                                Ok(AgentEvent::TurnComplete) => {
                                    return Ok((task_id, output));
                                }
                                Ok(AgentEvent::Error(error)) => {
                                    return Err(format!("task '{task_id}' failed: {error}"));
                                }
                                Ok(_) => {}
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(
                                    skipped,
                                )) => {
                                    return Err(format!(
                                        "task '{task_id}' event stream lagged by {skipped} messages"
                                    ));
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    return Err(format!(
                                        "task '{task_id}' agent stopped unexpectedly"
                                    ));
                                }
                            }
                        }
                    });
                }

                while let Some(result) = completions.join_next().await {
                    match result {
                        Ok(Ok((task_id, output))) => {
                            task_results.insert(task_id, output);
                        }
                        Ok(Err(e)) => {
                            eprintln!("orchestrate: task failed: {e}");
                        }
                        Err(e) => {
                            eprintln!("orchestrate: task join error: {e}");
                        }
                    }
                }
            }
        });

        Ok(ExecutionPlan {
            waves: plan_waves,
            task_count,
            critical_path,
        })
    }
}

fn suggest_roles(requested: &str, available: &[String]) -> Vec<String> {
    let requested_lower = requested.to_lowercase();
    let requested_words: Vec<&str> = requested_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 2)
        .collect();

    if requested_words.is_empty() {
        return available.to_vec();
    }

    let mut scored: Vec<(String, usize)> = available
        .iter()
        .map(|name| {
            let name_lower = name.to_lowercase();
            let name_words: Vec<&str> = name_lower
                .split(|c: char| !c.is_alphanumeric())
                .filter(|w| !w.is_empty())
                .collect();

            let score = requested_words
                .iter()
                .filter(|rw| {
                    name_words
                        .iter()
                        .any(|nw| nw.contains(**rw) || rw.contains(nw))
                })
                .count();

            (name.clone(), score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();

    scored.sort_by_key(|b| std::cmp::Reverse(b.1));
    scored.into_iter().take(3).map(|(name, _)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggest_roles_finds_word_overlap() {
        let available = vec![
            "planner".to_string(),
            "executor".to_string(),
            "code-reviewer".to_string(),
            "coder".to_string(),
            "tester".to_string(),
        ];

        let suggestions = suggest_roles("coder", &available);
        assert!(suggestions.contains(&"coder".to_string()));
        assert!(suggestions.contains(&"code-reviewer".to_string()));
    }

    #[test]
    fn suggest_roles_limits_to_three() {
        let available = vec![
            "code-a".to_string(),
            "code-b".to_string(),
            "code-c".to_string(),
            "code-d".to_string(),
            "other".to_string(),
        ];

        let suggestions = suggest_roles("code", &available);
        assert!(suggestions.len() <= 3);
    }

    #[test]
    fn suggest_roles_returns_all_when_no_words() {
        let available = vec!["planner".to_string(), "executor".to_string()];
        let suggestions = suggest_roles("ab", &available);
        assert_eq!(suggestions, available);
    }
}
