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

        // Phase 2: Non‑blocking delegation.
        // 1. Spawn the child through the L-1→L0→L1→L2 pipeline.
        // 2. Dispatch ActivateAgent to the background event loop.
        // 3. Return Running instantly — no LLM stream is blocked.

        let (runtime, agent_pool, parent_id, runtime_tx) = {
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
            let runtime_tx = s
                .core
                .runtime_event_tx
                .clone()
                .ok_or_else(|| ToolCallError("Runtime event channel not initialized".to_string()))?;
            (runtime, s.core.agent_pool.clone(), parent_id, runtime_tx)
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

        // Dispatch to the background event loop — the event loop owns
        // the ToolServerHandle and will execute the agent asynchronously.
        let _ = runtime_tx
            .send(crate::runtime::event::RuntimeEvent::ActivateAgent {
                agent_id: child_id,
                parent_id: Some(parent_id),
            })
            .await
            .map_err(|_| ToolCallError("Background runtime loop is dead".to_string()))?;

        Ok(SpawnAgentOutput::Running {
            agent_id: crate::agent::AgentPool::agent_id_str(&child_id),
            role,
            goal,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpawnAgentArgs ──

    #[test]
    fn test_spawn_agent_args_deserialize() {
        let json = r#"{
            "role": "planner",
            "goal": "Plan the architecture",
            "reason": "Need expert design"
        }"#;
        let args: SpawnAgentArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.role, "planner");
        assert_eq!(args.goal, "Plan the architecture");
        assert_eq!(args.reason, "Need expert design");
        assert!(args.expected_output.is_none());
        assert!(args.blocking.is_none());
    }

    #[test]
    fn test_spawn_agent_args_with_optional_fields() {
        let json = r#"{
            "role": "developer",
            "goal": "Write code",
            "reason": "Task delegation",
            "expected_output": "Rust file",
            "blocking": true
        }"#;
        let args: SpawnAgentArgs = serde_json::from_str(json).unwrap();
        assert_eq!(args.role, "developer");
        assert_eq!(args.expected_output.as_deref(), Some("Rust file"));
        assert_eq!(args.blocking, Some(true));
    }

    // ── SpawnAgentOutput ──

    #[test]
    fn test_spawn_agent_output_completed_serialization() {
        let output = SpawnAgentOutput::Completed {
            agent_id: "abc123".to_string(),
            role: "tester".to_string(),
            goal: "Test module".to_string(),
            result: "All tests passed".to_string(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("completed"));
        assert!(json.contains("abc123"));
        assert!(json.contains("All tests passed"));
    }

    #[test]
    fn test_spawn_agent_output_running_serialization() {
        let output = SpawnAgentOutput::Running {
            agent_id: "def456".to_string(),
            role: "worker".to_string(),
            goal: "Run task".to_string(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("running"));
        assert!(json.contains("def456"));
    }

    #[test]
    fn test_spawn_agent_output_rejected_serialization() {
        let output = SpawnAgentOutput::Rejected {
            role: "hacker".to_string(),
            goal: "Inject code".to_string(),
            reason: "Security violation".to_string(),
            recoverable: false,
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("rejected"));
        assert!(json.contains("Security violation"));
        assert!(!json.contains("recoverable\": true"));
    }

    // ── SpawnAgent tool definition ──

    #[tokio::test]
    async fn test_spawn_agent_definition_returns_valid_tool_def() {
        let state = Arc::new(RwLock::new(crate::tui::state::AppState::default()));
        let tool = SpawnAgent { state };
        let def = tool.definition(String::new()).await;
        assert_eq!(def.name, "spawn_agent");
        assert!(def.description.contains("child agent"));
        // Should have parameters with required fields
        let params = def.parameters;
        assert!(params.get("required").is_some());
    }

    // ── spawn_agent tool entry validation ──

    #[tokio::test]
    async fn test_spawn_agent_empty_role_rejected() {
        let state = Arc::new(RwLock::new(crate::tui::state::AppState::default()));
        let tool = SpawnAgent { state };
        let result = tool
            .call(SpawnAgentArgs {
                role: "  ".to_string(),
                goal: "test".to_string(),
                reason: "test".to_string(),
                expected_output: None,
                blocking: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("role"));
    }

    #[tokio::test]
    async fn test_spawn_agent_empty_goal_rejected() {
        let state = Arc::new(RwLock::new(crate::tui::state::AppState::default()));
        let tool = SpawnAgent { state };
        let result = tool
            .call(SpawnAgentArgs {
                role: "planner".to_string(),
                goal: "  ".to_string(),
                reason: "test".to_string(),
                expected_output: None,
                blocking: None,
            })
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("goal"));
    }

    // ── Register tools ──

    #[test]
    fn test_register_tools_returns_server() {
        let state = Arc::new(RwLock::new(crate::tui::state::AppState::default()));
        let server = crate::tools::ToolServer::new();
        let _server = register_tools(server, state);
        // Should not panic
    }
}
