use super::*;
use anyhow::Result;
use rig::client::EmbeddingsClient;
use rig::embeddings::EmbeddingsBuilder;
use rig::providers::cohere;
use rig::providers::openai;

impl LlmProvider {
    pub async fn embed(&self, text: &str) -> Result<Vec<f64>> {
        match self {
            Self::OpenAi(client) => {
                let model = client.embedding_model(openai::TEXT_EMBEDDING_ADA_002);
                let embeddings = EmbeddingsBuilder::new(model).document(text)?.build().await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Cohere(client) => {
                let model = client.embedding_model(cohere::EMBED_ENGLISH_V3, "search_document");
                let embeddings = EmbeddingsBuilder::new(model).document(text)?.build().await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Gemini(client) => {
                let model = client.embedding_model("text-embedding-004");
                let embeddings = EmbeddingsBuilder::new(model).document(text)?.build().await?;
                Ok(embeddings
                    .first()
                    .map(|(_, e)| e.first().vec.to_vec())
                    .unwrap_or_default())
            }
            Self::Mistral(client) => {
                let model = client.embedding_model("mistral-embed");
                let embeddings = EmbeddingsBuilder::new(model).document(text)?.build().await?;
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
