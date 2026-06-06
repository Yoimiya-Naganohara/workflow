use anyhow::Result;
use rig::client::{CompletionClient, EmbeddingsClient, ProviderClient};
use rig::completion::Prompt;
use rig::embeddings::EmbeddingsBuilder;
use rig::providers::anthropic;
use rig::providers::openai;
use serde::{Deserialize, Serialize};

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

pub enum LlmProvider {
    OpenAi(openai::Client),
    Anthropic(anthropic::Client),
}

impl LlmProvider {
    pub fn openai_from_env() -> Result<Self> {
        Ok(Self::OpenAi(openai::Client::from_env()?))
    }

    pub fn anthropic_from_env() -> Result<Self> {
        Ok(Self::Anthropic(anthropic::Client::from_env()?))
    }

    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        match self {
            Self::OpenAi(client) => {
                let agent = client
                    .agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build();

                let prompt = request
                    .messages
                    .last()
                    .map(|m| m.content.as_str())
                    .unwrap_or("");

                let response = agent.prompt(prompt).await?;
                Ok(LlmResponse {
                    content: response,
                    tokens_used: 0,
                })
            }
            Self::Anthropic(client) => {
                let agent = client
                    .agent(&request.model)
                    .temperature(request.temperature)
                    .max_tokens(request.max_tokens)
                    .build();

                let prompt = request
                    .messages
                    .last()
                    .map(|m| m.content.as_str())
                    .unwrap_or("");

                let response = agent.prompt(prompt).await?;
                Ok(LlmResponse {
                    content: response,
                    tokens_used: 0,
                })
            }
        }
    }

    pub async fn embed(&self, text: &str) -> Result<Vec<f64>> {
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
            Self::Anthropic(_) => {
                anyhow::bail!("Anthropic does not support embeddings. Use OpenAI or a local model.")
            }
        }
    }

    pub async fn embed_768(&self, text: &str) -> Result<[f32; 768]> {
        let raw = self.embed(text).await?;
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        Ok(embedding)
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
}
