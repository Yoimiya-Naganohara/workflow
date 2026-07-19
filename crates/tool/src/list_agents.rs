use std::sync::Arc;

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use workflow_agent::agent_pool::{AgentInfo, AgentPool};

use crate::ToolError;

#[derive(Deserialize)]
pub struct ListAgentsArgs {}

pub struct ListAgents {
    pool: Arc<AgentPool>,
}

impl ListAgents {
    pub fn new(pool: Arc<AgentPool>) -> Self {
        Self { pool }
    }
}

impl Tool for ListAgents {
    const NAME: &'static str = "list_agents";

    type Error = ToolError;
    type Args = ListAgentsArgs;
    type Output = Vec<AgentInfo>;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "list_agents".to_string(),
            description: "List all agents in the pool with their id, role, current task, and state"
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Ok(self.pool.list_agents().await)
    }
}
