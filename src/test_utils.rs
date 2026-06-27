use crate::core::constants::EMBEDDING_DIM;
use crate::llm::EmbeddingService;
use anyhow::Result;
use async_trait::async_trait;

/// Mock embedding service returning deterministic vectors.
///
/// `embed` returns `[1.0, 0, 0, ..., 0]`.  `similarity` uses the real
/// cosine similarity implementation.  Cache methods are no-ops.
pub struct MockEmbed;

/// Alias for compatibility with modules that named their mock differently.
pub use MockEmbed as MockEmbedding;

#[async_trait]
impl EmbeddingService for MockEmbed {
    async fn embed(&self, _text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        let mut e = [0.0f32; EMBEDDING_DIM];
        e[0] = 1.0;
        Ok(e)
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>> {
        Ok(texts
            .iter()
            .map(|_| {
                let mut e = [0.0f32; EMBEDDING_DIM];
                e[0] = 1.0;
                e
            })
            .collect())
    }

    fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
        crate::core::simd::cosine_similarity_384(a, b)
    }

    fn cache_size(&self) -> usize {
        0
    }

    fn clear_cache(&self) {}

    fn cache_hits(&self) -> u64 {
        0
    }

    fn cache_misses(&self) -> u64 {
        0
    }
}
