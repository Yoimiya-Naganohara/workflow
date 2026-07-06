//! Test utilities — mock implementations for testing.
//!
//! Only compiled in test builds (`#[cfg(test)]`).

use anyhow::Result;
use async_trait::async_trait;
use wf_core::EMBEDDING_DIM;
use wf_llm::EmbeddingService;

/// Mock embedding service returning deterministic vectors.
pub struct MockEmbed;

/// Alias for compatibility.
pub use MockEmbed as MockEmbedding;

#[async_trait]
impl EmbeddingService for MockEmbed {
    async fn embed(&self, _: &str) -> Result<[f32; EMBEDDING_DIM]> {
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
        wf_core::simd::cosine_similarity_384(a, b)
    }

    fn cache_size(&self) -> usize {
        0
    }

    fn cache_hits(&self) -> u64 {
        0
    }

    fn cache_misses(&self) -> u64 {
        0
    }

    fn clear_cache(&self) {}
}
