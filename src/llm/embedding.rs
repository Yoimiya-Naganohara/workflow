//! Embedding services — local fastembed + remote LLM provider.
//!
//! Provides two implementations of [`crate::llm::EmbeddingService`]:
//! - [`EmbeddingService`]: local ONNX inference (fastembed, always available).
//! - [`EmbeddingRouter`]: strategy-based router that can fall back to a remote
//!   LLM provider embedding API when available.

use std::sync::Arc;

use anyhow::Result;
use dashmap::DashMap;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use ort::execution_providers::{
    CPUExecutionProvider, CUDAExecutionProvider, CoreMLExecutionProvider,
};
use tokio::sync::Mutex;

use crate::core::simd::cosine_similarity_384;
use crate::core::types::EMBEDDING_DIM;
use crate::llm::{LlmProvider, ProviderProtocol};

// ============================================================================
//  EmbeddingStrategy
// ============================================================================

/// Strategy for choosing between local and remote embedding.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingStrategy {
    /// Use only local fastembed (always available, 384-dim).
    LocalOnly,
    /// Use only the remote LLM provider embedding API.
    RemoteOnly,
    /// Try remote first; fall back to local on failure.
    LocalFallback,
    /// Use remote when supported (higher quality), otherwise local.
    #[default]
    QualityFirst,
}

/// Local embedding service using fastembed (ONNX runtime, GPU-accelerated).
///
/// Tries CUDA first; falls back to CPU if no NVIDIA GPU / CUDA toolkit is available.
pub struct EmbeddingService {
    model: Mutex<TextEmbedding>,
    cache: DashMap<String, [f32; EMBEDDING_DIM]>,
}

impl EmbeddingService {
    /// Initialize the embedding model (downloaded on first use).
    ///
    /// Uses all-MiniLM-L6-v2 (384-dim, ~23 MB) with GPU acceleration.
    /// Falls back to CPU if CUDA is unavailable.
    pub fn new() -> Self {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2).with_execution_providers(vec![
                // GPU providers (tried in order; ONNX Runtime skips unavailable ones):
                CUDAExecutionProvider::default().into(),
                CoreMLExecutionProvider::default().into(),
                // CPU fallback:
                CPUExecutionProvider::default().into(),
            ]),
        )
        .expect("Failed to initialize fastembed (all-MiniLM-L6-v2).");
        Self {
            model: Mutex::new(model),
            cache: DashMap::new(),
        }
    }

    /// Embed a single text string into a 768-d vector.
    ///
    /// Results are cached by exact text match to avoid recomputation.
    pub async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        if let Some(cached) = self.cache.get(text) {
            return Ok(*cached);
        }

        let model = self.model.lock().await;
        let embeddings = model.embed(vec![text], Some(1))?;
        let raw = &embeddings[0];
        let mut result = [0.0f32; EMBEDDING_DIM];
        let len = raw.len().min(EMBEDDING_DIM);
        result[..len].copy_from_slice(&raw[..len]);
        let normalized = normalize_embedding(result);

        self.cache.insert(text.to_string(), normalized);
        Ok(normalized)
    }

    /// Embed multiple texts.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>> {
        let mut results = vec![[0.0f32; EMBEDDING_DIM]; texts.len()];
        let mut uncached: Vec<(usize, String)> = Vec::new();

        for (i, text) in texts.iter().enumerate() {
            if let Some(cached) = self.cache.get(*text) {
                results[i] = *cached;
            } else {
                uncached.push((i, text.to_string()));
            }
        }

        if uncached.is_empty() {
            return Ok(results);
        }

        let texts_to_embed: Vec<&str> = uncached.iter().map(|(_, t)| t.as_str()).collect();
        let model = self.model.lock().await;
        let embeddings = model.embed(texts_to_embed, Some(texts.len()))?;

        for ((idx, text), embedding) in uncached.iter().zip(embeddings.iter()) {
            let mut result = [0.0f32; EMBEDDING_DIM];
            let len = embedding.len().min(EMBEDDING_DIM);
            result[..len].copy_from_slice(&embedding[..len]);
            let normalized = normalize_embedding(result);
            self.cache.insert(text.clone(), normalized);
            results[*idx] = normalized;
        }

        Ok(results)
    }

    /// Cosine similarity between two embeddings.
    pub fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
        cosine_similarity_384(a, b)
    }

    /// Number of cached embeddings.
    pub fn cache_size(&self) -> usize {
        self.cache.len()
    }

    /// Clear the embedding cache.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }
}

impl Default for EmbeddingService {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_embedding(mut embedding: [f32; EMBEDDING_DIM]) -> [f32; EMBEDDING_DIM] {
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut embedding {
            *value /= norm;
        }
    }
    embedding
}

// ============================================================================
//  RemoteEmbedder — wraps LlmProvider for embeddings
// ============================================================================

/// Wraps an [`LlmProvider`] for remote embedding calls.
struct RemoteEmbedder {
    provider: Arc<LlmProvider>,
    protocol: ProviderProtocol,
}

impl RemoteEmbedder {
    fn new(provider: Arc<LlmProvider>) -> Self {
        let protocol = match &*provider {
            LlmProvider::OpenAi(_) => ProviderProtocol::OpenAiCompatible,
            LlmProvider::Anthropic(_) => ProviderProtocol::Anthropic,
            LlmProvider::Cohere(_) => ProviderProtocol::Cohere,
            LlmProvider::Gemini(_) => ProviderProtocol::Gemini,
            LlmProvider::Mistral(_) => ProviderProtocol::Mistral,
            LlmProvider::Ollama(_) => ProviderProtocol::Ollama,
            LlmProvider::Llamafile(_) => ProviderProtocol::Llamafile,
            LlmProvider::Azure(_) => ProviderProtocol::Azure,
            LlmProvider::Copilot(_) => ProviderProtocol::Copilot,
        };
        Self { provider, protocol }
    }

    fn is_available(&self) -> bool {
        self.protocol.supports_embeddings()
    }

    async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        let raw = self.provider.embed(text).await?;
        let mut result = [0.0f32; EMBEDDING_DIM];
        let len = raw.len().min(EMBEDDING_DIM);
        for i in 0..len {
            result[i] = raw[i] as f32;
        }
        Ok(normalize_embedding(result))
    }
}

// ============================================================================
//  EmbeddingRouter — strategy-based composite
// ============================================================================

/// Strategy-based embedding router that combines local and remote providers.
///
/// ```
/// use std::sync::Arc;
/// use workflow::llm::embedding::{EmbeddingRouter, EmbeddingStrategy};
///
/// let local = workflow::llm::embedding::EmbeddingService::new();
/// let router = EmbeddingRouter::new(local, None, EmbeddingStrategy::LocalOnly);
/// ```
pub struct EmbeddingRouter {
    local: crate::llm::embedding::EmbeddingService,
    remote: Option<RemoteEmbedder>,
    strategy: EmbeddingStrategy,
    cache: DashMap<String, [f32; EMBEDDING_DIM]>,
}

impl EmbeddingRouter {
    pub fn new(
        local: crate::llm::embedding::EmbeddingService,
        remote: Option<Arc<LlmProvider>>,
        strategy: EmbeddingStrategy,
    ) -> Self {
        Self {
            local,
            remote: remote.map(RemoteEmbedder::new),
            strategy,
            cache: DashMap::new(),
        }
    }

    pub fn with_strategy(mut self, strategy: EmbeddingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    pub fn set_remote(&mut self, provider: Arc<LlmProvider>) {
        self.remote = Some(RemoteEmbedder::new(provider));
    }

    pub fn clear_remote(&mut self) {
        self.remote = None;
    }

    fn use_remote(&self) -> bool {
        match self.strategy {
            EmbeddingStrategy::LocalOnly => false,
            EmbeddingStrategy::RemoteOnly => self.remote.as_ref().is_some_and(|r| r.is_available()),
            EmbeddingStrategy::LocalFallback | EmbeddingStrategy::QualityFirst => {
                self.remote.as_ref().is_some_and(|r| r.is_available())
            }
        }
    }

    async fn embed_impl(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        if let Some(cached) = self.cache.get(text) {
            return Ok(*cached);
        }

        let result = if self.use_remote() {
            let remote = self.remote.as_ref().unwrap();
            match remote.embed(text).await {
                Ok(emb) => {
                    if self.strategy == EmbeddingStrategy::LocalFallback {
                        return Ok(emb);
                    }
                    emb
                }
                Err(e) => {
                    if self.strategy == EmbeddingStrategy::RemoteOnly {
                        return Err(e);
                    }
                    // Fall back to local
                    tracing::warn!("Remote embedding failed, falling back to local: {}", e);
                    self.local.embed(text).await?
                }
            }
        } else {
            self.local.embed(text).await?
        };

        self.cache.insert(text.to_string(), result);
        Ok(result)
    }
}

#[async_trait::async_trait]
impl crate::llm::EmbeddingService for EmbeddingRouter {
    async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        self.embed_impl(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>> {
        // Batch uncached texts
        let mut results = vec![[0.0f32; EMBEDDING_DIM]; texts.len()];
        let mut uncached: Vec<(usize, String)> = Vec::new();

        for (i, text) in texts.iter().enumerate() {
            if let Some(cached) = self.cache.get(*text) {
                results[i] = *cached;
            } else {
                uncached.push((i, text.to_string()));
            }
        }

        if uncached.is_empty() {
            return Ok(results);
        }

        for (idx, text) in &uncached {
            if let Ok(emb) = self.embed_impl(text).await {
                results[*idx] = emb;
            }
        }

        Ok(results)
    }

    fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
        cosine_similarity_384(a, b)
    }

    fn cache_size(&self) -> usize {
        self.cache.len()
    }

    fn clear_cache(&self) {
        self.cache.clear();
    }
}

// ============================================================================
//  Existing EmbeddingService impl + trait impl
// ============================================================================

/// Async trait implementation for `crate::llm::EmbeddingService`.
#[async_trait::async_trait]
impl crate::llm::EmbeddingService for EmbeddingService {
    async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        self.embed(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>> {
        self.embed_batch(texts).await
    }

    fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
        self.similarity(a, b)
    }

    fn cache_size(&self) -> usize {
        self.cache_size()
    }

    fn clear_cache(&self) {
        self.clear_cache();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_embedding() {
        let mut embedding = [0.0f32; EMBEDDING_DIM];
        embedding[0] = 3.0;
        embedding[1] = 4.0;

        let normalized = normalize_embedding(embedding);
        assert!((normalized[0] - 0.6).abs() < 1e-6);
        assert!((normalized[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_zero_embedding() {
        let embedding = [0.0f32; EMBEDDING_DIM];
        let normalized = normalize_embedding(embedding);
        assert_eq!(normalized, embedding);
    }

    #[test]
    fn test_cosine_similarity() {
        let a = [1.0f32; EMBEDDING_DIM];
        let b = [1.0f32; EMBEDDING_DIM];
        let sim = cosine_similarity_384(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }
}
