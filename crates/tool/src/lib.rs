use rig::completion::ToolDefinition;
use rig::tool::Tool;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ── SendMessage tool ─────────────────────────────────────────

/// Arguments expected by the `send_message` tool.
#[derive(Deserialize)]
pub struct SendMessageArgs {
    /// Target recipient identifier (e.g. agent name, user ID, channel).
    pub recipient: String,
    /// Message body to deliver.
    pub message: String,
}

/// Structured output returned to the model after a send attempt.
#[derive(Serialize)]
pub struct SendMessageOutput {
    pub success: bool,
    pub detail: String,
}

/// Errors produced by [`SendMessageTool`].
#[derive(Debug, thiserror::Error)]
pub enum SendMessageError {
    #[error("send_message channel closed")]
    ChannelClosed,
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// A tool that sends a message through an unbounded channel.
///
/// The consumer polls the receiver end to forward messages to their
/// final destination (another agent, external API, etc.).
///
/// # Example registration on a ToolServer
/// ```ignore
/// let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
/// let tool_server = ToolServer::new()
///     .tool(SendMessageTool::new(tx))
///     .run();
/// // poll rx in the agent loop to forward messages...
/// ```
pub struct SendMessageTool {
    tx: mpsc::UnboundedSender<(String, String)>,
}

impl SendMessageTool {
    pub fn new(tx: mpsc::UnboundedSender<(String, String)>) -> Self {
        Self { tx }
    }
}

impl Tool for SendMessageTool {
    const NAME: &'static str = "send_message";

    type Error = SendMessageError;
    type Args = SendMessageArgs;
    type Output = SendMessageOutput;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description:
                "Send a text message to a named recipient (another agent, user, or system). \
                 Returns whether the message was dispatched successfully."
                    .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "recipient": {
                        "type": "string",
                        "description": "Identifier of the target recipient"
                    },
                    "message": {
                        "type": "string",
                        "description": "Message content to send"
                    }
                },
                "required": ["recipient", "message"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        self.tx
            .send((args.recipient, args.message))
            .map_err(|_| SendMessageError::ChannelClosed)?;
        Ok(SendMessageOutput {
            success: true,
            detail: "Message dispatched".into(),
        })
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_message_tool_definition() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let tool = SendMessageTool::new(tx);
        let def = tool.definition("".into()).await;
        assert_eq!(def.name, "send_message");
        assert!(def.description.contains("recipient"));
    }

    #[tokio::test]
    async fn test_send_message_tool_call() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let tool = SendMessageTool::new(tx);

        let output = tool
            .call(SendMessageArgs {
                recipient: "agent-42".into(),
                message: "hello from test".into(),
            })
            .await
            .unwrap();

        assert!(output.success);

        let (recipient, message) = rx.recv().await.unwrap();
        assert_eq!(recipient, "agent-42");
        assert_eq!(message, "hello from test");
    }

    #[tokio::test]
    async fn test_send_message_tool_channel_closed() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // close the receiver end

        let tool = SendMessageTool::new(tx);
        let result = tool
            .call(SendMessageArgs {
                recipient: "anyone".into(),
                message: "will fail".into(),
            })
            .await;

        assert!(result.is_err());
    }
}
