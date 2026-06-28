use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::pin::Pin;

// ============================================================================
//  ProviderProtocol — maps to a rig provider client type
// ============================================================================

/// Protocol/handler for an LLM provider.
///
/// Each variant corresponds to a specific rig provider implementation,
/// except `OpenAiCompatible` which is used for any OpenAI-compatible API
/// (DeepSeek, Groq, OpenRouter, custom endpoints, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderProtocol {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Cohere,
    Gemini,
    Mistral,
    Ollama,
    Llamafile,
    Azure,
    Copilot,
}

impl ProviderProtocol {
    /// Detect the protocol from a provider ID string.
    ///
    /// Returns `OpenAiCompatible` for unknown providers as a safe default
    /// (most self-hosted and custom APIs follow the OpenAI format).
    pub fn from_id(provider_id: &str) -> Self {
        match provider_id {
            "openai" => Self::OpenAi,
            "anthropic" => Self::Anthropic,
            "cohere" => Self::Cohere,
            "gemini" | "google" => Self::Gemini,
            "mistral" => Self::Mistral,
            "ollama" => Self::Ollama,
            "llamafile" => Self::Llamafile,
            "azure" => Self::Azure,
            "github-copilot" | "copilot" => Self::Copilot,
            _ if provider_id.starts_with("custom-") => Self::OpenAiCompatible,
            _ => Self::OpenAiCompatible,
        }
    }

    /// Human-readable label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::OpenAiCompatible => "OpenAI Compatible",
            Self::Anthropic => "Anthropic",
            Self::Cohere => "Cohere",
            Self::Gemini => "Gemini",
            Self::Mistral => "Mistral",
            Self::Ollama => "Ollama",
            Self::Llamafile => "Llamafile",
            Self::Azure => "Azure",
            Self::Copilot => "GitHub Copilot",
        }
    }

    /// Whether this protocol requires an API key.
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama | Self::Llamafile)
    }

    /// Whether this protocol supports embeddings.
    pub fn supports_embeddings(&self) -> bool {
        matches!(
            self,
            Self::OpenAi | Self::OpenAiCompatible | Self::Cohere | Self::Gemini | Self::Mistral
        )
    }

    /// Whether this protocol supports tool calling.
    pub fn supports_tools(&self) -> bool {
        !matches!(self, Self::Llamafile)
    }
}

impl fmt::Display for ProviderProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ============================================================================
//  Message / Request / Response types
// ============================================================================

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
    /// Request timeout in seconds (default: 60).
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Max retries on transient errors (default: 3).
    #[serde(default)]
    pub max_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: u32,
    /// Input tokens served from provider-managed cache (prompt caching).
    pub cached_input_tokens: u32,
    /// Input tokens written to provider-managed cache.
    pub cache_creation_input_tokens: u32,
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

/// Why a tool-enabled chat stream ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneReason {
    /// Normal stream completion: LLM produced a FinalResponse.
    Normal,
    /// Duplicate tool+args detected (3+ repeats) — forced termination.
    LoopTerminated,
    /// Stream produced an error — forced termination.
    StreamError,
}

/// Event emitted during a tool-enabled chat stream.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    /// Agent starts processing (pi-agent-core: agent_start)
    AgentStart,
    /// Agent ends processing (pi-agent-core: agent_end)
    AgentEnd,
    /// A new turn begins (pi-agent-core: turn_start)
    TurnStart,
    /// A turn completes (pi-agent-core: turn_end)
    TurnEnd,
    /// A message begins streaming (pi-agent-core: message_start)
    MessageStart,
    /// A message completes (pi-agent-core: message_end)
    MessageEnd,
    Text(String),
    /// Reasoning/chain-of-thought content emitted by the model.
    /// Separate from Text so the TUI can render it with distinct styling
    /// (dimmed/italic) and track it independently of the final response.
    Reasoning(String),
    /// The LLM requested a tool call.  The `result` field was removed
    /// because rig's streaming API does not expose the tool execution
    /// result in the `ToolCall` event — it is consumed internally by
    /// the multi-turn framework before our code sees the event.
    ///
    /// Use `ListAgents` or inspect the response text that follows a
    /// `ToolCall` event to infer the tool's effect.
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    /// Per-turn token usage from the LLM provider.
    ///
    /// Emitted after each completion request in a multi‑turn tool chain.
    /// Values come from [`rig::completion::Usage`] reported by the provider.
    TokenUsage {
        input: u32,
        output: u32,
        /// Input tokens served from provider-managed cache.
        cached_input: u32,
        /// Input tokens written to provider-managed cache.
        cache_creation_input: u32,
    },
    Done {
        reason: DoneReason,
    },
}

pub type ToolChatStream = Pin<Box<dyn Stream<Item = ToolEvent> + Send>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_done_reason_variants() {
        assert_eq!(format!("{:?}", DoneReason::Normal), "Normal");
        assert_eq!(
            format!("{:?}", DoneReason::LoopTerminated),
            "LoopTerminated"
        );
        assert_eq!(format!("{:?}", DoneReason::StreamError), "StreamError");
    }

    #[test]
    fn test_tool_event_debug() {
        let event = ToolEvent::Text("hello".to_string());
        assert!(format!("{:?}", event).contains("Text"));
    }

    #[test]
    fn test_tool_event_clone() {
        let event = ToolEvent::Text("clone".to_string());
        let cloned = event.clone();
        assert!(matches!(cloned, ToolEvent::Text(t) if t == "clone"));
    }

    #[test]
    fn test_tool_call_event() {
        let event = ToolEvent::ToolCall {
            name: "read_file".into(),
            args: serde_json::json!({"path": "/tmp/test.txt"}),
        };
        match &event {
            ToolEvent::ToolCall { name, args, .. } => {
                assert_eq!(name, "read_file");
                assert_eq!(args["path"], "/tmp/test.txt");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn test_token_usage_event() {
        let event = ToolEvent::TokenUsage {
            input: 100,
            output: 50,
            cached_input: 10,
            cache_creation_input: 5,
        };
        match event {
            ToolEvent::TokenUsage { input, output, .. } => {
                assert_eq!(input, 100);
                assert_eq!(output, 50);
            }
            _ => panic!("expected TokenUsage"),
        }
    }

    #[test]
    fn test_tool_event_done_variants() {
        for reason in &[
            DoneReason::Normal,
            DoneReason::LoopTerminated,
            DoneReason::StreamError,
        ] {
            let event = ToolEvent::Done { reason: *reason };
            match event {
                ToolEvent::Done { reason: r } => assert_eq!(r, *reason),
                _ => panic!("expected Done"),
            }
        }
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message {
            role: "user".into(),
            content: "hello".into(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.content, "hello");
    }

    #[test]
    fn test_llm_request_serialization() {
        let req = LlmRequest {
            model: "gpt-4".into(),
            messages: vec![Message {
                role: "system".into(),
                content: "be helpful".into(),
            }],
            temperature: 0.7,
            max_tokens: 1000,
            timeout_secs: None,
            max_retries: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: LlmRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "gpt-4");
        assert_eq!(back.messages.len(), 1);
    }

    #[test]
    fn test_llm_response_serialization() {
        let resp = LlmResponse {
            content: "response text".into(),
            tokens_used: 150,
            cached_input_tokens: 10,
            cache_creation_input_tokens: 5,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: LlmResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "response text");
        assert_eq!(back.tokens_used, 150);
    }

    #[test]
    fn test_protocol_formatting() {
        assert_eq!(format!("{}", ProviderProtocol::OpenAi), "OpenAI");
        assert_eq!(format!("{:?}", ProviderProtocol::Gemini), "Gemini");
    }
}
