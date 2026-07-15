// ── SendMessage ──────────────────────────────────────────────

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rig::{completion::ToolDefinition, tool::Tool};
use serde::Deserialize;
use workflow_agent::{
    Message, agent_pool::AgentPool, current_agent_id,
    protocol::{PeerIntent, PeerMessage},
};

use crate::ToolError;

static NEXT_MSG_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize)]
pub struct SendMessageArgs {
    pub target_id: workflow_agent::AgentId,
    pub message: String,
}

pub struct SendMessage {
    pool: Arc<AgentPool>,
}

impl SendMessage {
    pub fn new(pool: Arc<AgentPool>) -> Self {
        Self { pool }
    }
}

impl Tool for SendMessage {
    const NAME: &'static str = "send_message";

    type Error = ToolError;
    type Args = SendMessageArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: "send_message".to_string(),
            description: "Send a message to another agent in the pool by its numeric agent ID"
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "target_id": {
                        "type": "integer",
                        "description": "The numeric agent ID of the recipient"
                    },
                    "message": {
                        "type": "string",
                        "description": "The message content to send"
                    }
                },
                "required": ["target_id", "message"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let from_id = current_agent_id();
        let msg = PeerMessage {
            id: NEXT_MSG_ID.fetch_add(1, Ordering::Relaxed),
            thread_id: 0,
            from: from_id,
            to: args.target_id,
            intent: PeerIntent::Request,
            reply_to: None,
            content: args.message,
        };

        let agent = self
            .pool
            .get_agent(&args.target_id)
            .await
            .ok_or(ToolError::AgentNotFound(args.target_id))?;

        agent
            .send(Message::AgentMessage(msg))
            .await
            .map_err(|source| ToolError::SendFailed {
                receiver: args.target_id,
                source,
            })?;

        Ok(format!("Message sent to agent {}", args.target_id))
    }
}
