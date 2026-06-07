use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f64,
    pub max_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: u32,
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

/// Event emitted during a tool-enabled chat stream.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    /// A text chunk from the assistant.
    Text(String),
    /// A tool call that was executed, with its result.
    ToolCall {
        name: String,
        args: serde_json::Value,
        result: String,
    },
    /// The stream has completed.
    Done,
}

/// Streaming type for tool-enabled chat, yielding tool events.
pub type ToolChatStream = Pin<Box<dyn Stream<Item = ToolEvent> + Send>>;
