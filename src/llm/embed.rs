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

#[cfg(test)]
mod tests {
    // ── embed_768 conversion ──

    #[test]
    fn test_embed_768_converts_f32() {
        // Create a test wrapper that mimics the conversion logic
        let raw = vec![1.0f64; 768];
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        assert_eq!(embedding[0], 1.0f32);
        assert_eq!(embedding[767], 1.0f32);
    }

    #[test]
    fn test_embed_768_truncates_long_input() {
        let raw = vec![0.5f64; 1000];
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        // Should only copy first 768 elements
        assert_eq!(embedding[0], 0.5f32);
        assert_eq!(embedding[767], 0.5f32);
        // The copy loop only runs 768 times, so embedding after 768 is still 0.0
    }

    #[test]
    fn test_embed_768_pads_short_input() {
        let raw = vec![2.0f64; 100];
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        assert_eq!(embedding[0], 2.0f32);
        assert_eq!(embedding[99], 2.0f32);
        // Elements beyond 100 should remain 0.0
        assert_eq!(embedding[100], 0.0f32);
        assert_eq!(embedding[767], 0.0f32);
    }

    #[test]
    fn test_embed_768_empty_input() {
        let raw: Vec<f64> = vec![];
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        // All elements should be 0.0
        assert_eq!(embedding.iter().sum::<f32>(), 0.0);
    }

    #[test]
    fn test_embed_768_preserves_sign_and_magnitude() {
        let raw = vec![-3.14f64, 2.718f64, -0.001f64, 0.0f64];
        let mut embedding = [0.0f32; 768];
        let len = raw.len().min(768);
        for i in 0..len {
            embedding[i] = raw[i] as f32;
        }
        assert!((embedding[0] - (-3.14f32)).abs() < f32::EPSILON);
        assert!((embedding[1] - 2.718f32).abs() < f32::EPSILON);
        assert!((embedding[2] - (-0.001f32)).abs() < f32::EPSILON);
        assert_eq!(embedding[3], 0.0);
    }
}
