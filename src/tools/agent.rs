//! Runtime-aware agent tools.

use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::tui::state::AppState;

use super::builtin::ToolCallError;

/// Register agent-management tools.
pub fn register_tools(server: crate::tools::ToolServer, state: Arc<RwLock<AppState>>) -> crate::tools::ToolServer {
    server.tool(SpawnAgent { state })
}

#[derive(Clone)]
pub struct SpawnAgent {
    state: Arc<RwLock<AppState>>,
}

#[derive(Debug, Deserialize)]
pub struct SpawnAgentArgs {
    pub role: String,
    pub goal: String,
    pub reason: String,
    pub expected_output: Option<String>,
    pub blocking: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SpawnAgentOutput {
    Completed {
        agent_id: String,
        role: String,
        goal: String,
        result: String,
    },
    Running {
        agent_id: String,
        role: String,
        goal: String,
    },
    Rejected {
        role: String,
        goal: String,
        reason: String,
        recoverable: bool,
    },
}

impl Tool for SpawnAgent {
    const NAME: &'static str = "spawn_agent";

    type Error = ToolCallError;
    type Args = SpawnAgentArgs;
    type Output = SpawnAgentOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: concat!(
                "Spawn a child agent through the workflow runtime. ",
                "Use this only when delegation is useful. The human does not approve or manage child agents; ",
                "the calling agent remains responsible for integrating the result."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "role": {
                        "type": "string",
                        "description": "Child role, e.g. planner, developer, tester, reviewer, worker"
                    },
                    "goal": {
                        "type": "string",
                        "description": "Concrete goal for the child agent"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why this work should be delegated"
                    },
                    "expected_output": {
                        "type": "string",
                        "description": "What the child should return"
                    },
                    "blocking": {
                        "type": "boolean",
                        "description": "If true, wait for the child result before returning"
                    }
                },
                "required": ["role", "goal", "reason"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let role = args.role.trim().to_string();
        let goal = args.goal.trim().to_string();
        if role.is_empty() {
            return Err(ToolCallError("role is required".to_string()));
        }
        if goal.is_empty() {
            return Err(ToolCallError("goal is required".to_string()));
        }

        let child_goal = match args.expected_output.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(expected) => format!(
                "{}\n\nDelegation reason: {}\n\nExpected output: {}",
                goal,
                args.reason.trim(),
                expected
            ),
            None => format!("{}\n\nDelegation reason: {}", goal, args.reason.trim()),
        };

        let (runtime, agent_pool, parent_id) = {
            let s = self.state.read().await;
            let runtime = s
                .core
                .runtime
                .clone()
                .ok_or_else(|| ToolCallError("Runtime not initialized".to_string()))?;
            let parent_id = s
                .core
                .responsible_agent_id
                .ok_or_else(|| ToolCallError("No responsible parent agent is active".to_string()))?;
            (runtime, s.core.agent_pool.clone(), parent_id)
        };

        let child_id = match runtime
            .read()
            .await
            .spawn_plan_task_agent(parent_id, &role, &child_goal, &mut *agent_pool.write().await)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                return Ok(SpawnAgentOutput::Rejected {
                    role,
                    goal,
                    reason: e.to_string(),
                    recoverable: true,
                });
            }
        };

        let child_id_str = crate::agent::AgentPool::agent_id_str(&child_id);
        if args.blocking.unwrap_or(true) {
            runtime.read().await.execute_agent(child_id, &agent_pool).await;
            let result = runtime.read().await.await_agent(child_id, &agent_pool).await;
            Ok(SpawnAgentOutput::Completed {
                agent_id: child_id_str,
                role,
                goal,
                result,
            })
        } else {
            let runtime_clone = runtime.clone();
            let pool_clone = agent_pool.clone();
            tokio::spawn(async move {
                runtime_clone.read().await.execute_agent(child_id, &pool_clone).await;
            });
            Ok(SpawnAgentOutput::Running {
                agent_id: child_id_str,
                role,
                goal,
            })
        }
    }
}
