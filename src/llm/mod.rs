pub mod chat;
pub mod embed;
pub mod embedding;
pub mod factory;
pub mod types;

use crate::core::types::EMBEDDING_DIM;
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
    fn clear_cache(&self);
}

use rig::client::CompletionClient;
use rig::completion::Prompt;
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
    async fn do_complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let timeout_secs = request.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS);
        let max_retries = request.max_retries.unwrap_or(DEFAULT_MAX_RETRIES);
        let timeout = Duration::from_secs(timeout_secs);

        let system_prompt = request
            .messages
            .first()
            .filter(|m| m.role == "system")
            .map(|m| m.content.as_str())
            .unwrap_or("");
        let prompt = request.messages.last().map(|m| m.content.as_str()).unwrap_or("");

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
                (resp.output, total)
            }};
        }

        let mut last_error = None;
        for attempt in 0..=max_retries {
            let result = tokio::time::timeout(timeout, async {
                let (content, tokens_used): (String, u32) = match self {
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
                Ok(LlmResponse { content, tokens_used })
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
                Err(_elapsed) => {
                    // Timeout elapsed
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
            tracing::warn!(
                "LLM request failed (attempt {}/{}), retrying in {}s: {}",
                attempt + 1,
                max_retries + 1,
                backoff.as_secs(),
                last_error.as_ref().unwrap()
            );
            tokio::time::sleep(backoff).await;
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("LLM request failed after all retries")))
    }

    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.do_complete(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ProviderProtocol ──

    #[test]
    fn test_protocol_from_id() {
        assert_eq!(ProviderProtocol::from_id("openai"), ProviderProtocol::OpenAi);
        assert_eq!(ProviderProtocol::from_id("anthropic"), ProviderProtocol::Anthropic);
        assert_eq!(ProviderProtocol::from_id("cohere"), ProviderProtocol::Cohere);
        assert_eq!(ProviderProtocol::from_id("gemini"), ProviderProtocol::Gemini);
        assert_eq!(ProviderProtocol::from_id("google"), ProviderProtocol::Gemini);
        assert_eq!(ProviderProtocol::from_id("mistral"), ProviderProtocol::Mistral);
        assert_eq!(ProviderProtocol::from_id("ollama"), ProviderProtocol::Ollama);
        assert_eq!(ProviderProtocol::from_id("llamafile"), ProviderProtocol::Llamafile);
        assert_eq!(ProviderProtocol::from_id("azure"), ProviderProtocol::Azure);
        assert_eq!(ProviderProtocol::from_id("github-copilot"), ProviderProtocol::Copilot);
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
        assert_eq!(ProviderProtocol::OpenAiCompatible.label(), "OpenAI Compatible");
        assert_eq!(ProviderProtocol::Anthropic.label(), "Anthropic");
        assert_eq!(ProviderProtocol::Ollama.label(), "Ollama");
    }

    // ── from_key routing ──

    #[test]
    fn test_deepseek_routes_to_openai() {
        let result = LlmProvider::from_key("test-key", Some("https://api.deepseek.com"), "deepseek");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_groq_routes_to_openai() {
        let result = LlmProvider::from_key("test-key", Some("https://api.groq.com/openai/v1"), "groq");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_openrouter_routes_to_openai() {
        let result = LlmProvider::from_key("test-key", Some("https://openrouter.ai/api/v1"), "openrouter");
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
        let result = LlmProvider::from_key("sk-test", Some("https://custom.deepseek.com/v1"), "deepseek");
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
        let result = LlmProvider::from_protocol("test-key", None, ProviderProtocol::OpenAiCompatible);
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
