// ── SendMessage ──────────────────────────────────────────────

use serde::Deserialize;
use workflow_agent::{AgentId, Message, MessageType};

use crate::ToolError;

/// Arguments for the `send_message` tool.
#[derive(Deserialize)]
pub struct SendMessageArgs {
    /// The numeric ID of the sending agent.
    from_id: AgentId,
    /// The numeric ID of the target agent.
    target_id: AgentId,
    /// The message content to send.
    message: String,
}

/// Tool that sends a message from one agent to another via the [`AgentPool`].
///
/// The calling LLM is expected to include its own `from_id` in the arguments
/// (the prompt should instruct it to do so).
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
                    "from_id": {
                        "type": "integer",
                        "description": "Your own numeric agent ID (the sender)"
                    },
                    "target_id": {
                        "type": "integer",
                        "description": "The numeric agent ID of the recipient"
                    },
                    "message": {
                        "type": "string",
                        "description": "The message content to send"
                    }
                },
                "required": ["from_id", "target_id", "message"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let agent = self
            .pool
            .get_agent(&args.target_id)
            .await
            .ok_or(ToolError::AgentNotFound(args.target_id))?;

        let sender = agent.sender().clone();
        let msg = Message::Data(MessageType::AgentMessage(args.from_id, args.message));

        sender
            .send(msg)
            .await
            .map_err(|source| ToolError::SendFailed {
                receiver: args.target_id,
                source,
            })?;

        Ok(format!("Message sent to agent {}", args.target_id))
    }
}
