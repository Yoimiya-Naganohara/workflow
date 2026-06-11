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
    Text(String),
    ToolCall {
        name: String,
        args: serde_json::Value,
        result: String,
    },
    Done,
}

pub type ToolChatStream = Pin<Box<dyn Stream<Item = ToolEvent> + Send>>;
