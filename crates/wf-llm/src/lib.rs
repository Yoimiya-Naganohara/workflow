//! wf-llm — LLM provider abstraction, embedding, and chat.
#![allow(clippy::module_inception)]

pub mod chat;
pub mod embedding;
pub mod types;

use wf_core::EMBEDDING_DIM;
pub use types::*;

use anyhow::Result;
use async_trait::async_trait;
use std::time::Duration;

// Default timeout/retry configuration for LLM requests.
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_MAX_RETRIES: u32 = 3;

// Re-export ProviderProtocol at the crate level for convenience.
pub use types::ProviderProtocol;

/// Text-to-vector embedding service with caching.
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>>;
    fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32;
    fn cache_size(&self) -> usize;
    fn cache_hits(&self) -> u64;
    fn cache_misses(&self) -> u64;
    fn clear_cache(&self);
}

use rig::client::{CompletionClient, EmbeddingsClient, Nothing, ProviderClient};
use rig::completion::Prompt;
use rig::embeddings::EmbeddingsBuilder;
use rig::providers::anthropic;
use rig::providers::azure;
use rig::providers::cohere;
use rig::providers::copilot;
use rig::providers::gemini;
use rig::providers::llamafile;
use rig::providers::mistral;
use rig::providers::ollama;
use rig::providers::openai;

#[derive(Debug, Clone)]
pub enum LlmProvider {
    OpenAi(openai::CompletionsClient),
    Anthropic(anthropic::Client),
    Cohere(cohere::Client),
    Gemini(gemini::Client),
    Mistral(mistral::Client),
    Ollama(ollama::Client),
    Llamafile(llamafile::Client),
    Azure(azure::Client),
    Copilot(copilot::Client),
}
impl LlmProvider {
    /// Classify an error as retryable (transient) vs permanent.
    fn is_retryable(err: &anyhow::Error) -> bool {
        // Try to extract a reqwest error — this is the most common source
        // of network errors from rig's HTTP-based providers.
        if let Some(req_err) = err.downcast_ref::<reqwest::Error>() {
            // Timeouts, DNS failures, connection resets — always retryable
            if req_err.is_timeout() || req_err.is_connect() || req_err.is_request() {
                return true;
            }
            // Status-based classification
            if let Some(status) = req_err.status() {
                let code = status.as_u16();
                // 5xx (server errors) + 429 (rate limit) → retryable
                if code >= 500 || code == 429 {
                    return true;
                }
                // 4xx (client errors except 429) → not retryable
                return false;
            }
            // No status code and not a recognized transport error → retry
            return true;
        }

        // For non-reqwest errors (rig-native, local), fall back to string matching.
        let msg = err.to_string().to_lowercase();
        // Transient patterns
        if msg.contains("timeout")
            || msg.contains("timed out")
            || msg.contains("temporarily unavailable")
            || msg.contains("too many requests")
        {
            return true;
        }
        // Permanent patterns (auth, not found, invalid request)
        if msg.contains("unauthorized")
            || msg.contains("forbidden")
            || msg.contains("not found")
            || msg.contains("invalid")
        {
            return false;
        }
        // Default: retry (defense in depth)
        true
    }

    /// Execute the API call with timeout and retry logic.
    ///
    /// # Message handling
    ///
    /// This is a **single-turn** completion — it uses `rig::Prompt` which
    /// sends exactly one system prompt + one user message.  Multi-turn
    /// conversation should use `chat_with_tools_stream_mcp` instead.
    ///
    /// From `request.messages`, the **first system message** is used as
    /// the system prompt and the **last message** (any role) as the user
    /// prompt.  Intermediate messages are not included — use the streaming
    /// API for multi-turn conversations.
    async fn do_complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let timeout_secs = request.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let max_retries = request.max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
        let timeout = Duration::from_secs(timeout_secs);

        if request.messages.is_empty() {
            anyhow::bail!("LlmRequest has empty messages — nothing to send to LLM");
        }

        let system_prompt = request
            .messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let prompt = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("");

        // Use extended_details().prompt() to capture token usage from the provider.
        macro_rules! complete_ext {
            ($client:expr) => {{
                let resp = $client
                    .agent(&request.model)
                    .preamble(system_prompt)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .extended_details()
                    .await?;
                let total = resp.usage.total_tokens as u32;
                let cached = resp.usage.cached_input_tokens as u32;
                let cache_create = resp.usage.cache_creation_input_tokens as u32;
                (resp.output, total, cached, cache_create)
            }};
        }

        let mut last_error = None;
        for attempt in 0..=max_retries {
            let result = tokio::time::timeout(timeout, async {
                let (content, tokens_used, cached_input_tokens, cache_creation_input_tokens): (
                    String,
                    u32,
                    u32,
                    u32,
                ) = match self {
                    Self::OpenAi(c) => complete_ext!(c),
                    Self::Anthropic(c) => complete_ext!(c),
                    Self::Cohere(c) => complete_ext!(c),
                    Self::Gemini(c) => complete_ext!(c),
                    Self::Mistral(c) => complete_ext!(c),
                    Self::Ollama(c) => complete_ext!(c),
                    Self::Llamafile(c) => complete_ext!(c),
                    Self::Azure(c) => complete_ext!(c),
                    Self::Copilot(c) => complete_ext!(c),
                };
                Ok(LlmResponse {
                    content,
                    tokens_used,
                    cached_input_tokens,
                    cache_creation_input_tokens,
                })
            })
            .await;

            match result {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(e)) => {
                    if !Self::is_retryable(&e) || attempt >= max_retries {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
                Err(_) => {
                    let timeout_err = anyhow::anyhow!(
                        "LLM request timed out after {}s (attempt {}/{})",
                        timeout_secs,
                        attempt + 1,
                        max_retries + 1
                    );
                    if attempt >= max_retries {
                        return Err(timeout_err);
                    }
                    last_error = Some(timeout_err);
                }
            }

            // Exponential backoff: 1s, 2s, 4s, ...
            let backoff = Duration::from_secs(1 << attempt);
            if let Some(ref err) = last_error {
                tracing::warn!(
                    "LLM request failed (attempt {}/{}), retrying in {}s: {}",
                    attempt + 1,
                    max_retries + 1,
                    backoff.as_secs(),
                    err
                );
            }
            tokio::time::sleep(backoff).await;
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM request failed after all retries")))
    }

    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.do_complete(request).await
    }

    /// Build reasoning parameters from an effort level and the model's
    /// `reasoning_options` (parsed from `api.json`).
    ///
    /// For OpenAI-compatible providers: `{"reasoning_effort": effort}`
    /// For Anthropic: `{"thinking": {"type": "enabled", "effort": effort, "budget_tokens": N}}`
    /// For others: returns `None`.
    pub fn reasoning_params(
        &self,
        effort: &str,
        reasoning_options: &[wf_core::ReasoningOption],
    ) -> Option<serde_json::Value> {
        match self {
            // OpenAI / Azure / Copilot — flat `reasoning_effort`.
            Self::OpenAi(_) | Self::Azure(_) | Self::Copilot(_) => {
                Some(serde_json::json!({"reasoning_effort": effort}))
            }
            // Anthropic — structured thinking block.
            // Parse reasoning_options to determine effort + budget_tokens.
            Self::Anthropic(_) => {
                use wf_core::ReasoningOption;
                let mut params = serde_json::json!({
                    "thinking": {
                        "type": "enabled"
                    }
                });
                if let Some(obj) = params.as_object_mut() {
                    if let Some(thinking) = obj.get_mut("thinking").and_then(|v| v.as_object_mut())
                    {
                        for opt in reasoning_options {
                            match opt {
                                ReasoningOption::Effort { values } => {
                                    if values.is_empty() || values.contains(&effort.to_string()) {
                                        thinking.insert("effort".into(), serde_json::json!(effort));
                                    }
                                }
                                ReasoningOption::BudgetTokens { .. } => {
                                    let budget = match effort {
                                        "low" => 8192,
                                        "medium" => 16384,
                                        "high" => 32768,
                                        _ => 16384,
                                    };
                                    thinking
                                        .insert("budget_tokens".into(), serde_json::json!(budget));
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Some(params)
            }
            _ => None,
        }
    }
}

// ============================================================================
//  embed — Provider-based embedding API (moved from embed.rs)
// ============================================================================

impl LlmProvider {
    pub async fn embed(&self, text: &str) -> anyhow::Result<Vec<f64>> {
        match self {
            Self::OpenAi(client) => {
                let model = client.embedding_model(openai::TEXT_EMBEDDING_ADA_002);
                let embeddings = EmbeddingsBuilder::new(model)
                    .document(text)?
                    .build()
                    .await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Cohere(client) => {
                let model = client.embedding_model(cohere::EMBED_ENGLISH_V3, "search_document");
                let embeddings = EmbeddingsBuilder::new(model)
                    .document(text)?
                    .build()
                    .await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Gemini(client) => {
                let model = client.embedding_model("text-embedding-004");
                let embeddings = EmbeddingsBuilder::new(model)
                    .document(text)?
                    .build()
                    .await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Mistral(client) => {
                let model = client.embedding_model("mistral-embed");
                let embeddings = EmbeddingsBuilder::new(model)
                    .document(text)?
                    .build()
                    .await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            _ => anyhow::bail!(
                "Embeddings not supported for this provider. \
                 Use OpenAI, Cohere, Gemini, or Mistral."
            ),
        }
    }
}

// ============================================================================
//  factory — Provider construction (moved from factory.rs)
// ============================================================================

impl LlmProvider {
    /// Build a provider from environment variables.
    pub fn from_env() -> anyhow::Result<Self> {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            let mut builder = openai::CompletionsClient::builder().api_key(&key);
            if let Ok(url) = std::env::var("OPENAI_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::OpenAi(builder.build()?));
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            let mut builder = anthropic::Client::builder().api_key(&key);
            if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::Anthropic(builder.build()?));
        }
        if let Ok(key) = std::env::var("COHERE_API_KEY") {
            return Ok(Self::Cohere(cohere::Client::new(key)?));
        }
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            return Ok(Self::Gemini(gemini::Client::new(key)?));
        }
        if let Ok(key) = std::env::var("MISTRAL_API_KEY") {
            return Ok(Self::Mistral(mistral::Client::new(&key)?));
        }
        if std::env::var("OLLAMA_API_BASE_URL").is_ok() || Self::is_ollama_running() {
            let mut builder = ollama::Client::builder().api_key(Nothing);
            if let Ok(url) = std::env::var("OLLAMA_API_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::Ollama(builder.build()?));
        }
        if let Ok(key) = std::env::var("AZURE_API_KEY") {
            let endpoint = std::env::var("AZURE_ENDPOINT")
                .map_err(|_| anyhow::anyhow!("AZURE_ENDPOINT must be set with AZURE_API_KEY"))?;
            let api_version =
                std::env::var("AZURE_API_VERSION").unwrap_or_else(|_| "2024-10-21".to_string());
            return Ok(Self::Azure(
                azure::Client::builder()
                    .api_key(&key)
                    .azure_endpoint(endpoint)
                    .api_version(&api_version)
                    .build()?,
            ));
        }
        if std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GITHUB_COPILOT_API_KEY").is_ok()
        {
            return Ok(Self::Copilot(copilot::Client::from_env()?));
        }
        anyhow::bail!(
            "No API key found. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, COHERE_API_KEY, \
             GEMINI_API_KEY, MISTRAL_API_KEY, AZURE_API_KEY, or GITHUB_TOKEN"
        )
    }

    fn is_ollama_running() -> bool {
        std::net::TcpStream::connect_timeout(
            &"127.0.0.1:11434".parse().expect("static socket addr"),
            std::time::Duration::from_millis(200),
        )
        .is_ok()
    }

    /// Build a provider from an API key + optional base URL + provider_id.
    pub fn from_key(
        api_key: &str,
        base_url: Option<&str>,
        provider_id: &str,
    ) -> anyhow::Result<Self> {
        let protocol = crate::ProviderProtocol::from_id(provider_id);
        Self::from_protocol(api_key, base_url, protocol)
    }

    /// Build a provider from an API key + optional base URL + protocol.
    pub fn from_protocol(
        api_key: &str,
        base_url: Option<&str>,
        protocol: crate::ProviderProtocol,
    ) -> anyhow::Result<Self> {
        match protocol {
            ProviderProtocol::Anthropic => {
                let mut builder = anthropic::Client::builder().api_key(api_key);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::Anthropic(builder.build()?))
            }
            ProviderProtocol::Cohere => Ok(Self::Cohere(cohere::Client::new(api_key)?)),
            ProviderProtocol::Gemini => Ok(Self::Gemini(gemini::Client::new(api_key)?)),
            ProviderProtocol::Mistral => Ok(Self::Mistral(mistral::Client::new(api_key)?)),
            ProviderProtocol::Ollama => {
                let mut builder = ollama::Client::builder().api_key(Nothing);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::Ollama(builder.build()?))
            }
            ProviderProtocol::Llamafile => {
                let url = base_url.unwrap_or("http://localhost:8080");
                Ok(Self::Llamafile(llamafile::Client::from_url(url)?))
            }
            ProviderProtocol::Azure => {
                let endpoint = base_url.unwrap_or("").to_string();
                let api_version =
                    std::env::var("AZURE_API_VERSION").unwrap_or_else(|_| "2024-10-21".to_string());
                Ok(Self::Azure(
                    azure::Client::builder()
                        .api_key(api_key)
                        .azure_endpoint(endpoint)
                        .api_version(&api_version)
                        .build()?,
                ))
            }
            ProviderProtocol::Copilot => Ok(Self::Copilot(
                copilot::Client::builder().api_key(api_key).build()?,
            )),
            ProviderProtocol::OpenAi | ProviderProtocol::OpenAiCompatible => {
                let mut builder = openai::CompletionsClient::builder().api_key(api_key);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::OpenAi(builder.build()?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ProviderProtocol ──

    #[test]
    fn test_protocol_from_id() {
        assert_eq!(
            ProviderProtocol::from_id("openai"),
            ProviderProtocol::OpenAi
        );
        assert_eq!(
            ProviderProtocol::from_id("anthropic"),
            ProviderProtocol::Anthropic
        );
        assert_eq!(
            ProviderProtocol::from_id("cohere"),
            ProviderProtocol::Cohere
        );
        assert_eq!(
            ProviderProtocol::from_id("gemini"),
            ProviderProtocol::Gemini
        );
        assert_eq!(
            ProviderProtocol::from_id("google"),
            ProviderProtocol::Gemini
        );
        assert_eq!(
            ProviderProtocol::from_id("mistral"),
            ProviderProtocol::Mistral
        );
        assert_eq!(
            ProviderProtocol::from_id("ollama"),
            ProviderProtocol::Ollama
        );
        assert_eq!(
            ProviderProtocol::from_id("llamafile"),
            ProviderProtocol::Llamafile
        );
        assert_eq!(ProviderProtocol::from_id("azure"), ProviderProtocol::Azure);
        assert_eq!(
            ProviderProtocol::from_id("github-copilot"),
            ProviderProtocol::Copilot
        );
        assert_eq!(
            ProviderProtocol::from_id("custom-myapi"),
            ProviderProtocol::OpenAiCompatible
        );
    }

    #[test]
    fn test_unknown_provider_falls_back_to_openai_compatible() {
        assert_eq!(
            ProviderProtocol::from_id("some-new-provider"),
            ProviderProtocol::OpenAiCompatible
        );
    }

    #[test]
    fn test_protocol_requires_api_key() {
        assert!(ProviderProtocol::OpenAi.requires_api_key());
        assert!(ProviderProtocol::Anthropic.requires_api_key());
        assert!(!ProviderProtocol::Ollama.requires_api_key());
        assert!(!ProviderProtocol::Llamafile.requires_api_key());
    }

    #[test]
    fn test_protocol_supports_embeddings() {
        assert!(ProviderProtocol::OpenAi.supports_embeddings());
        assert!(ProviderProtocol::OpenAiCompatible.supports_embeddings());
        assert!(ProviderProtocol::Cohere.supports_embeddings());
        assert!(ProviderProtocol::Gemini.supports_embeddings());
        assert!(ProviderProtocol::Mistral.supports_embeddings());
        assert!(!ProviderProtocol::Anthropic.supports_embeddings());
        assert!(!ProviderProtocol::Ollama.supports_embeddings());
    }

    #[test]
    fn test_protocol_label() {
        assert_eq!(ProviderProtocol::OpenAi.label(), "OpenAI");
        assert_eq!(
            ProviderProtocol::OpenAiCompatible.label(),
            "OpenAI Compatible"
        );
        assert_eq!(ProviderProtocol::Anthropic.label(), "Anthropic");
        assert_eq!(ProviderProtocol::Ollama.label(), "Ollama");
    }

    // ── from_key routing ──

    #[test]
    fn test_deepseek_routes_to_openai() {
        let result =
            LlmProvider::from_key("test-key", Some("https://api.deepseek.com"), "deepseek");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_groq_routes_to_openai() {
        let result =
            LlmProvider::from_key("test-key", Some("https://api.groq.com/openai/v1"), "groq");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_openrouter_routes_to_openai() {
        let result = LlmProvider::from_key(
            "test-key",
            Some("https://openrouter.ai/api/v1"),
            "openrouter",
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_unknown_provider_falls_back_to_openai() {
        let result = LlmProvider::from_key(
            "test-key",
            Some("https://api.unknown-provider.com/v1"),
            "some-new-provider",
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_anthropic_routes_to_native() {
        let result = LlmProvider::from_key("test-key", None, "anthropic");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Anthropic(_)));
    }

    #[test]
    fn test_cohere_routes_to_native() {
        let result = LlmProvider::from_key("test-key", None, "cohere");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Cohere(_)));
    }

    #[test]
    fn test_gemini_routes_to_native() {
        let result = LlmProvider::from_key("test-key", None, "gemini");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Gemini(_)));
    }

    #[test]
    fn test_mistral_routes_to_native() {
        let result = LlmProvider::from_key("test-key", None, "mistral");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Mistral(_)));
    }

    #[test]
    fn test_ollama_no_auth() {
        let result = LlmProvider::from_key("", Some("http://localhost:11434"), "ollama");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Ollama(_)));
    }

    #[test]
    fn test_llamafile_no_auth() {
        let result = LlmProvider::from_key("", Some("http://localhost:8080"), "llamafile");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Llamafile(_)));
    }

    #[test]
    fn test_provider_with_base_url() {
        let result = LlmProvider::from_key(
            "sk-test",
            Some("https://custom.deepseek.com/v1"),
            "deepseek",
        );
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_provider_without_base_url() {
        let result = LlmProvider::from_key("test-key", None, "openai");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    // ── from_protocol ──

    #[test]
    fn test_from_protocol_openai_compatible() {
        let result =
            LlmProvider::from_protocol("test-key", None, ProviderProtocol::OpenAiCompatible);
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_from_protocol_copilot() {
        let result = LlmProvider::from_protocol("test-key", None, ProviderProtocol::Copilot);
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::Copilot(_)));
    }

    #[test]
    fn test_message_serialization() {
        let msg = Message {
            role: "user".to_string(),
            content: "Hello".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("Hello"));
    }

    #[test]
    fn test_llm_request_serialization() {
        let req = LlmRequest {
            model: "gpt-4".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: "test".to_string(),
            }],
            temperature: 0.7,
            max_tokens: 1000,
            timeout_secs: None,
            max_retries: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("gpt-4"));
    }
}

