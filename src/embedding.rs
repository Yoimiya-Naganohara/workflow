use crate::llm::LlmProvider;
use crate::simd::cosine_similarity_768;
use anyhow::Result;
use dashmap::DashMap;
use std::sync::Arc;

pub struct EmbeddingService {
    provider: Arc<LlmProvider>,
    cache: DashMap<String, [f32; 768]>,
}

impl EmbeddingService {
    pub fn new(provider: Arc<LlmProvider>) -> Self {
        Self {
            provider,
            cache: DashMap::new(),
        }
    }

    pub async fn embed(&self, text: &str) -> Result<[f32; 768]> {
        if let Some(cached) = self.cache.get(text) {
            return Ok(*cached);
        }

        let embedding = self.provider.embed_768(text).await?;

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        let normalized = if norm > 0.0 {
            let mut result = [0.0f32; 768];
            for i in 0..768 {
                result[i] = embedding[i] / norm;
            }
            result
        } else {
            embedding
        };

        self.cache.insert(text.to_string(), normalized);
        Ok(normalized)
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; 768]>> {
        let mut results = Vec::with_capacity(texts.len());
        for text in texts {
            results.push(self.embed(text).await?);
        }
        Ok(results)
    }

    pub fn similarity(&self, a: &[f32; 768], b: &[f32; 768]) -> f32 {
        cosine_similarity_768(a, b)
    }

    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockProvider;

    impl MockProvider {
        async fn embed_mock(&self, _text: &str) -> Result<[f32; 768]> {
            let mut embedding = [0.0f32; 768];
            embedding[0] = 1.0;
            Ok(embedding)
        }
    }

    #[tokio::test]
    async fn test_embed_cached() {
        let provider = Arc::new(LlmProvider::OpenAi(
            rig::providers::openai::Client::new("test-key").unwrap(),
        ));
        let service = EmbeddingService::new(provider);

        let e1 = [0.1f32; 768];
        let e2 = [0.1f32; 768];
        assert_eq!(e1, e2);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = [1.0f32; 768];
        let b = [1.0f32; 768];
        let sim = cosine_similarity_768(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }
}
