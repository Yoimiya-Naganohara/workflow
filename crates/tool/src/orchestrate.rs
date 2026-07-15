use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, task::JoinSet};
use workflow_agent::{Agent, AgentEvent, AgentId, Message, agent_pool::AgentPool};

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
    pub result: String,
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
    next_id: Arc<AtomicU32>,
    factory: AgentFactory,
    execution_lock: Mutex<()>,
}

impl Orchestrate {
    pub fn new(pool: Arc<AgentPool>, factory: AgentFactory) -> Self {
        Self {
            pool,
            next_id: Arc::new(AtomicU32::new(1)),
            factory,
            execution_lock: Mutex::new(()),
        }
    }

    pub fn with_id_allocator(
        pool: Arc<AgentPool>,
        factory: AgentFactory,
        next_id: Arc<AtomicU32>,
    ) -> Self {
        Self {
            pool,
            next_id,
            factory,
            execution_lock: Mutex::new(()),
        }
    }
}

impl Orchestrate {
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
        // Agent availability and assignment are check-then-act operations. Keep
        // one orchestration run at a time so concurrent tool calls cannot
        // reserve the same idle agent.
        let _execution_guard = self.execution_lock.lock().await;
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
        let mut task_results: HashMap<String, String> = HashMap::new();

        for wave in &waves {
            let mut assigned = Vec::new();
            let mut reserved = HashSet::new();
            let mut completions = JoinSet::new();

            for &idx in wave {
                let task = &tasks[idx];
                let agent = self.get_or_create_agent(&task.role, &reserved).await?;
                let agent_id = agent.id();
                reserved.insert(agent_id);

                let prompt = if task.depend_on.is_empty() {
                    task.task.clone()
                } else {
                    let dependency_context = task
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
                        task.task, dependency_context
                    )
                };

                // Subscribe before dispatch so a fast completion cannot be
                // emitted before the scheduler starts listening.
                let mut receiver = agent.receiver();
                agent.send(Message::User(prompt)).await.map_err(|e| {
                    ToolError::Orchestrate(format!("failed to dispatch task '{}': {e}", task.id))
                })?;

                let task_id = task.id.clone();
                completions.spawn(async move {
                    let mut output = String::new();
                    loop {
                        match receiver.recv().await {
                            Ok(AgentEvent::Text(text)) => output.push_str(&text),
                            Ok(AgentEvent::TurnComplete) => return Ok((task_id, output)),
                            Ok(AgentEvent::Error(error)) => {
                                return Err(format!("task '{task_id}' failed: {error}"));
                            }
                            Ok(_) => {}
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                return Err(format!(
                                    "task '{task_id}' event stream lagged by {skipped} messages"
                                ));
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                return Err(format!("task '{task_id}' agent stopped unexpectedly"));
                            }
                        }
                    }
                });

                assigned.push(TaskAssignment {
                    id: task.id.clone(),
                    role: task.role.clone(),
                    task: task.task.clone(),
                    wave: id_to_wave[idx],
                    agent_id,
                    result: String::new(),
                });
            }

            while let Some(result) = completions.join_next().await {
                let (task_id, output) = result
                    .map_err(|error| {
                        ToolError::Orchestrate(format!("task waiter failed: {error}"))
                    })?
                    .map_err(ToolError::Orchestrate)?;
                task_results.insert(task_id, output);
            }

            for assignment in &mut assigned {
                assignment.result = task_results
                    .get(&assignment.id)
                    .cloned()
                    .unwrap_or_default();
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
