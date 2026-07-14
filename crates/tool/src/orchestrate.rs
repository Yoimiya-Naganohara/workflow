use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use workflow_agent::{Agent, AgentId, Message, MessageType, agent_pool::AgentPool};

use crate::ToolError;

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

pub struct Orchestrate {
    pool: Arc<AgentPool>,
    next_id: AtomicU32,
    factory: AgentFactory,
}

impl Orchestrate {
    pub fn new(pool: Arc<AgentPool>, factory: AgentFactory) -> Self {
        Self {
            pool,
            next_id: AtomicU32::new(1),
            factory,
        }
    }
}

impl Orchestrate {
    async fn get_or_create_agent(&self, role: &str) -> Result<Arc<Agent>, ToolError> {
        let agents = self.pool.list_agents().await;
        for info in &agents {
            if info.role == role && info.current_task.is_none() {
                if let Some(agent) = self.pool.get_agent(&info.id).await {
                    return Ok(agent);
                }
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
                Creates agents for each role, assigns tasks in wave order \
                (parallel waves where possible), and begins execution. \
                Returns the execution plan with agent assignments."
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

        // ── Dispatch tasks ────────────────────────────────────
        let mut plan_waves: Vec<Vec<TaskAssignment>> = Vec::new();

        for wave in &waves {
            let mut assigned = Vec::new();

            for &idx in wave {
                let task = &tasks[idx];
                let agent = self.get_or_create_agent(&task.role).await?;
                let agent_id = agent.id();

                agent
                    .sender()
                    .send(Message::Data(MessageType::User(task.task.clone())))
                    .await
                    .map_err(|e| {
                        ToolError::Orchestrate(format!("failed to dispatch task '{}': {e}", task.id))
                    })?;

                assigned.push(TaskAssignment {
                    id: task.id.clone(),
                    role: task.role.clone(),
                    task: task.task.clone(),
                    wave: id_to_wave[idx],
                    agent_id,
                });
            }

            plan_waves.push(assigned);
        }

        let task_count = tasks.len();

        Ok(ExecutionPlan {
            waves: plan_waves,
            task_count,
            critical_path,
        })
    }
}
