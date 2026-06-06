pub mod chat;
pub mod embed;
pub mod embedding;
pub mod factory;
pub mod types;

pub use types::*;
use crate::core::types::EMBEDDING_DIM;

use anyhow::Result;
use async_trait::async_trait;

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
    async fn do_complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let prompt = request.messages.last().map(|m| m.content.as_str()).unwrap_or("");
        let response = match self {
            Self::OpenAi(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Anthropic(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Cohere(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Gemini(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Mistral(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Ollama(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Llamafile(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Azure(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
            Self::Copilot(c) => {
                c.agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build()
                    .prompt(prompt)
                    .await?
            }
        };
        Ok(LlmResponse {
            content: response,
            tokens_used: 0,
        })
    }

    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        self.do_complete(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("gpt-4"));
    }

    #[test]
    fn test_deepseek_routes_to_openai_compatible() {
        let result = LlmProvider::from_key("test-key", Some("https://api.deepseek.com"), "deepseek");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_groq_routes_to_openai_compatible() {
        let result = LlmProvider::from_key("test-key", Some("https://api.groq.com/openai/v1"), "groq");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }

    #[test]
    fn test_openrouter_routes_to_openai_compatible() {
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
        if let LlmProvider::OpenAi(_) = result.unwrap() {
            // Success
        } else {
            panic!("Expected OpenAi variant");
        }
    }

    #[test]
    fn test_provider_without_base_url() {
        let result = LlmProvider::from_key("test-key", None, "openai");
        assert!(result.is_ok());
        assert!(matches!(result.unwrap(), LlmProvider::OpenAi(_)));
    }
}
