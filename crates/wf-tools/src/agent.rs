//! Runtime-aware agent tools.

use std::sync::Arc;

use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::Deserialize;
use tokio::sync::RwLock;

use super::builtin::ToolCallError;

/// Register agent-management and inter-agent communication tools.
///
/// Takes extracted dependencies rather than AppState to avoid the
/// `tools → tui` dependency cycle.
pub fn register_tools(
    server: crate::ToolServer,
    agent_pool: Arc<RwLock<wf_agent::AgentPool>>,
    runtime_event_tx: Option<tokio::sync::mpsc::Sender<wf_core::event::RuntimeEvent>>,
    responsible_agent_id: Option<wf_core::AgentId>,
) -> crate::ToolServer {
    server
        .tool(DecomposeTask {
            runtime_event_tx: runtime_event_tx.clone(),
            responsible_agent_id,
        })
        .tool(SendMessage {
            agent_pool: agent_pool.clone(),
            runtime_event_tx: runtime_event_tx.clone(),
            responsible_agent_id,
        })
        .tool(ReadMessages {
            agent_pool: agent_pool.clone(),
            responsible_agent_id,
        })
        .tool(ListAgents { agent_pool })
}

// ── SendMessage ──

#[derive(Clone)]
pub struct SendMessage {
    agent_pool: Arc<RwLock<wf_agent::AgentPool>>,
    runtime_event_tx: Option<tokio::sync::mpsc::Sender<wf_core::event::RuntimeEvent>>,
    responsible_agent_id: Option<wf_core::AgentId>,
}

#[derive(Debug, Deserialize)]
pub struct SendMessageArgs {
    pub recipient: String,
    pub message: String,
}

impl Tool for SendMessage {
    const NAME: &'static str = "send_message";

    type Error = ToolCallError;
    type Args = SendMessageArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Send a message to another agent. Use this to coordinate with siblings."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "Agent name (as shown in agent tree) to send the message to"
                    },
                    "message": {
                        "type": "string",
                        "description": "Message content"
                    }
                },
                "required": ["recipient", "message"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let agent_id = self
            .responsible_agent_id
            .ok_or_else(|| ToolCallError("No active agent to send from".to_string()))?;
        let sender_name = {
            let pool = self.agent_pool.read().await;
            pool.get_agent(&agent_id)
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "unknown".to_string())
        };

        // Find recipient by name
        let recipient = {
            let pool = self.agent_pool.read().await;
            pool.agents()
                .iter()
                .find(|a| a.name == args.recipient)
                .map(|a| a.id)
                .ok_or_else(|| {
                    ToolCallError(format!(
                        "Agent '{}' not found. Use `list_agents` to see active agents.",
                        args.recipient
                    ))
                })?
        };

        // Write via try_write — spin backoff pattern
        let mut retries = 0u32;
        loop {
            if let Ok(mut pool) = self.agent_pool.try_write() {
                pool.send_message(recipient, agent_id, &sender_name, &args.message, None)
                    .map_err(ToolCallError)?;
                break;
            }
            if retries >= 5 {
                return Err(ToolCallError(
                    "Agent pool lock contention — message not sent".to_string(),
                ));
            }
            retries += 1;
            tokio::time::sleep(std::time::Duration::from_micros(50)).await;
        }

        // Emit InboxMessage event so the recipient gets notified.
        let preview: String = args.message.chars().take(200).collect();
        if let Some(tx) = &self.runtime_event_tx {
            let count = {
                if let Ok(pool) = self.agent_pool.try_read() {
                    pool.get_agent(&recipient)
                        .map(|a| a.inbox.len())
                        .unwrap_or(0)
                } else {
                    0
                }
            };
            let _ = tx
                .send(wf_core::event::RuntimeEvent::InboxMessage {
                    agent_id: recipient,
                    from_name: sender_name,
                    preview,
                    unread_count: count,
                })
                .await;
        }

        Ok(format!("Message sent to '{}'.", args.recipient))
    }
}

// ── ReadMessages ──

#[derive(Clone)]
pub struct ReadMessages {
    agent_pool: Arc<RwLock<wf_agent::AgentPool>>,
    responsible_agent_id: Option<wf_core::AgentId>,
}

#[derive(Debug, Deserialize)]
pub struct ReadMessagesArgs {
    pub max: Option<usize>,
}

impl Tool for ReadMessages {
    const NAME: &'static str = "read_messages";

    type Error = ToolCallError;
    type Args = ReadMessagesArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "Read and drain all pending messages from your inbox (FIFO).".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "max": {
                        "type": "integer",
                        "description": "Maximum messages to return (default: all, max: 20)",
                        "minimum": 1,
                        "maximum": 20,
                        "optional": true
                    }
                }
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let agent_id = self
            .responsible_agent_id
            .ok_or_else(|| ToolCallError("No active agent to read messages for".to_string()))?;

        let max = args.max.unwrap_or(20).min(20);
        let messages = {
            let mut pool = self.agent_pool.write().await;
            let all = pool.drain_inbox(&agent_id);
            all.into_iter().take(max).collect::<Vec<_>>()
        };

        if messages.is_empty() {
            return Ok("No messages in inbox.".to_string());
        }

        let mut output = format!("{} message(s) received:\n", messages.len());
        for msg in &messages {
            output.push_str(&format!("  [from {}] {}\n", msg.from_name, msg.content));
        }
        Ok(output)
    }
}

// ── ListAgents ──

#[derive(Clone)]
pub struct ListAgents {
    agent_pool: Arc<RwLock<wf_agent::AgentPool>>,
}

#[derive(Debug, Deserialize)]
pub struct ListAgentsArgs {}

impl Tool for ListAgents {
    const NAME: &'static str = "list_agents";

    type Error = ToolCallError;
    type Args = ListAgentsArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description: "List all active agents and their status.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn call(&self, _: Self::Args) -> Result<Self::Output, Self::Error> {
        let pool = self.agent_pool.read().await;
        let agents = pool.agents();
        if agents.is_empty() {
            return Ok("No agents in pool.".to_string());
        }
        let mut output = format!("Active agents ({} total):\n", agents.len());
        for a in agents {
            let status = match a.status {
                wf_agent::AgentStatus::Idle => "idle",
                wf_agent::AgentStatus::Planning => "planning",
                wf_agent::AgentStatus::AwaitingChildren => "awaiting",
                wf_agent::AgentStatus::Aggregating => "aggregating",
                wf_agent::AgentStatus::Completed => "completed",
                wf_agent::AgentStatus::Failed => "failed",
            };
            let inbox_count = a.inbox.len();
            output.push_str(&format!(
                "  {} [{}] {} — {} message(s)\n",
                a.name, status, a.role, inbox_count
            ));
        }
        Ok(output)
    }
}

// ── DecomposeTask ──

#[derive(Clone)]
pub struct DecomposeTask {
    runtime_event_tx: Option<tokio::sync::mpsc::Sender<wf_core::event::RuntimeEvent>>,
    responsible_agent_id: Option<wf_core::AgentId>,
}

#[derive(Debug, Deserialize)]
pub struct DecomposeTaskArgs {
    pub reasoning: String,
    pub subtasks: Vec<wf_core::SubtaskDef>,
}

impl Tool for DecomposeTask {
    const NAME: &'static str = "decompose_task";

    type Error = ToolCallError;
    type Args = DecomposeTaskArgs;
    type Output = String;

    async fn definition(&self, _: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.into(),
            description:
                "Decompose your current task into subtasks. Call this when the task is complex enough to warrant parallel or sequential execution by sub-agents. Provide a list of subtasks with roles, goals, dependency ordering, and whether each can auto-proceed without pipeline approval.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "reasoning": {
                        "type": "string",
                        "description": "Why this decomposition is needed and how it splits the work"
                    },
                    "subtasks": {
                        "type": "array",
                        "description": "List of subtask definitions",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Unique identifier for this subtask within this decomposition (referenced by depend_on)"
                                },
                                "role": {
                                    "type": "string",
                                    "description": "Role that should execute this subtask (e.g. developer, tester, reviewer)"
                                },
                                "goal": {
                                    "type": "string",
                                    "description": "Concrete goal for this subtask"
                                },
                                "depend_on": {
                                    "type": "array",
                                    "items": {"type": "string"},
                                    "description": "IDs of sibling subtasks that must complete before this one starts"
                                },
                                "auto_confirm": {
                                    "type": "boolean",
                                    "description": "If true, skip pipeline approval (L1/L2) and execute immediately"
                                }
                            },
                            "required": ["id", "role", "goal"]
                        }
                    }
                },
                "required": ["reasoning", "subtasks"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        if args.subtasks.is_empty() {
            return Err(ToolCallError("subtasks list cannot be empty".to_string()));
        }

        let parent_id = self
            .responsible_agent_id
            .ok_or_else(|| ToolCallError("No active agent to decompose from".to_string()))?;
        let runtime_tx = self
            .runtime_event_tx
            .clone()
            .ok_or_else(|| ToolCallError("Runtime event channel not initialized".to_string()))?;

        runtime_tx
            .send(wf_core::event::RuntimeEvent::DecomposeTask {
                parent_agent: parent_id,
                subtasks: args.subtasks,
            })
            .await
            .map_err(|_| ToolCallError("Background runtime loop is dead".to_string()))?;

        Ok("Decomposition submitted. Subtasks will be created and dispatched.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Register tools ──

    #[test]
    fn test_register_tools_returns_server() {
        let pool = Arc::new(RwLock::new(wf_agent::AgentPool::new()));
        let server = crate::ToolServer::new();
        let _ = register_tools(server, pool, None, None);
        // Should not panic
    }
}
