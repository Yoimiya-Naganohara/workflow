use crate::llm::LlmProvider;
use crate::simd::cosine_similarity_768;
use crate::traits::EmbeddingService as EmbeddingServiceTrait;
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
        let normalized = normalize_embedding(embedding);

        self.cache.insert(text.to_string(), normalized);
        Ok(normalized)
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; 768]>> {
        use std::collections::HashMap;

        let mut results = vec![[0.0f32; 768]; texts.len()];
        let mut pending: HashMap<&str, Vec<usize>> = HashMap::new();

        for (index, text) in texts.iter().enumerate() {
            if let Some(cached) = self.cache.get(*text) {
                results[index] = *cached;
            } else {
                pending.entry(*text).or_default().push(index);
            }
        }

        for (text, indexes) in pending {
            let embedding = self.embed(text).await?;
            for index in indexes {
                results[index] = embedding;
            }
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

#[async_trait::async_trait]
impl EmbeddingServiceTrait for EmbeddingService {
    async fn embed(&self, text: &str) -> Result<[f32; 768]> {
        self.embed(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; 768]>> {
        self.embed_batch(texts).await
    }

    fn similarity(&self, a: &[f32; 768], b: &[f32; 768]) -> f32 {
        self.similarity(a, b)
    }

    fn cache_size(&self) -> usize {
        self.cache_size()
    }

    fn clear_cache(&self) {
        self.clear_cache();
    }
}

fn normalize_embedding(mut embedding: [f32; 768]) -> [f32; 768] {
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut embedding {
            *value /= norm;
        }
    }
    embedding
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_embedding() {
        let mut embedding = [0.0f32; 768];
        embedding[0] = 3.0;
        embedding[1] = 4.0;

        let normalized = normalize_embedding(embedding);
        assert!((normalized[0] - 0.6).abs() < 1e-6);
        assert!((normalized[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_zero_embedding() {
        let embedding = [0.0f32; 768];
        let normalized = normalize_embedding(embedding);
        assert_eq!(normalized, embedding);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = [1.0f32; 768];
        let b = [1.0f32; 768];
        let sim = cosine_similarity_768(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }
}
