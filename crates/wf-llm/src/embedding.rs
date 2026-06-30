//! Embedding services — local fastembed + remote LLM provider.
//!
//! Provides two implementations of [`crate::EmbeddingService`]:
//! - [`EmbeddingService`]: local ONNX inference (fastembed, always available).
//! - [`EmbeddingRouter`]: strategy-based router that can fall back to a remote
//!   LLM provider embedding API when available.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::Result;
use dashmap::DashMap;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use ort::execution_providers::{
    CPUExecutionProvider, CUDAExecutionProvider, CoreMLExecutionProvider,
};
use tokio::sync::Mutex;

use crate::{LlmProvider, ProviderProtocol};
use wf_core::EMBEDDING_DIM;
use wf_core::simd::cosine_similarity_384;

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
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
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
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }

    /// Embed a single text string into a 768-d vector.
    ///
    /// Results are cached by exact text match to avoid recomputation.
    pub async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        if let Some(cached) = self.cache.get(text) {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(*cached);
        }
        self.cache_misses.fetch_add(1, Ordering::Relaxed);

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
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                results[i] = *cached;
            } else {
                uncached.push((i, text.to_string()));
            }
        }

        self.cache_misses
            .fetch_add(uncached.len() as u64, Ordering::Relaxed);

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

    /// Number of cache hits since creation or last reset.
    pub fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    /// Number of cache misses since creation or last reset.
    pub fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }

    /// Clear the embedding cache and reset hit/miss counters.
    pub fn clear_cache(&self) {
        self.cache.clear();
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
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
/// use wf_llm::embedding::{EmbeddingRouter, EmbeddingStrategy};
///
/// let local = wf_llm::embedding::EmbeddingService::new();
/// let router = EmbeddingRouter::new(local, None, EmbeddingStrategy::LocalOnly);
/// ```
pub struct EmbeddingRouter {
    local: crate::embedding::EmbeddingService,
    remote: Option<RemoteEmbedder>,
    strategy: EmbeddingStrategy,
    cache: DashMap<String, [f32; EMBEDDING_DIM]>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

impl EmbeddingRouter {
    pub fn new(
        local: crate::embedding::EmbeddingService,
        remote: Option<Arc<LlmProvider>>,
        strategy: EmbeddingStrategy,
    ) -> Self {
        Self {
            local,
            remote: remote.map(RemoteEmbedder::new),
            strategy,
            cache: DashMap::new(),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }

    pub fn with_strategy(mut self, strategy: EmbeddingStrategy) -> Self {
        if self.strategy != strategy {
            self.clear_router_cache();
            self.strategy = strategy;
        }
        self
    }

    pub fn set_remote(&mut self, provider: Arc<LlmProvider>) {
        self.remote = Some(RemoteEmbedder::new(provider));
        self.clear_router_cache();
    }

    pub fn clear_remote(&mut self) {
        self.remote = None;
        self.clear_router_cache();
    }

    fn clear_router_cache(&self) {
        self.cache.clear();
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
    }

    fn cache_key_for(&self, text: &str, source: &str) -> String {
        format!("{:?}|{}|{}", self.strategy, source, text)
    }

    fn remote_cache_source(remote: &RemoteEmbedder) -> String {
        format!("remote:{:?}", remote.protocol)
    }

    fn lookup_source(&self) -> String {
        if self.use_remote() {
            if let Some(remote) = self.remote.as_ref().filter(|remote| remote.is_available()) {
                return Self::remote_cache_source(remote);
            }
        }
        "local".to_string()
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
        let remote = if self.use_remote() {
            self.remote.as_ref().filter(|remote| remote.is_available())
        } else {
            None
        };
        let lookup_source = remote
            .map(Self::remote_cache_source)
            .unwrap_or_else(|| "local".to_string());
        let lookup_key = self.cache_key_for(text, &lookup_source);

        if let Some(cached) = self.cache.get(&lookup_key) {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            return Ok(*cached);
        }
        self.cache_misses.fetch_add(1, Ordering::Relaxed);

        let (result, insert_source) = if let Some(remote) = remote {
            match remote.embed(text).await {
                Ok(emb) => (emb, Self::remote_cache_source(remote)),
                Err(e) => {
                    if self.strategy == EmbeddingStrategy::RemoteOnly {
                        return Err(e);
                    }
                    tracing::warn!("Remote embedding failed, falling back to local: {}", e);
                    (self.local.embed(text).await?, "local".to_string())
                }
            }
        } else {
            (self.local.embed(text).await?, "local".to_string())
        };

        let insert_key = self.cache_key_for(text, &insert_source);
        self.cache.insert(insert_key, result);
        Ok(result)
    }
}

#[async_trait::async_trait]
impl crate::EmbeddingService for EmbeddingRouter {
    async fn embed(&self, text: &str) -> Result<[f32; EMBEDDING_DIM]> {
        self.embed_impl(text).await
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<[f32; EMBEDDING_DIM]>> {
        let mut results = vec![[0.0f32; EMBEDDING_DIM]; texts.len()];
        let mut uncached: Vec<(usize, String)> = Vec::new();
        let lookup_source = self.lookup_source();

        // ── Phase 1: router-cache probe ──
        for (i, text) in texts.iter().enumerate() {
            let key = self.cache_key_for(text, &lookup_source);
            if let Some(cached) = self.cache.get(&key) {
                self.cache_hits.fetch_add(1, Ordering::Relaxed);
                results[i] = *cached;
            } else {
                uncached.push((i, text.to_string()));
            }
        }

        if uncached.is_empty() {
            return Ok(results);
        }

        // Bulk-count misses for all uncached items.
        self.cache_misses
            .fetch_add(uncached.len() as u64, Ordering::Relaxed);

        // ── Phase 2: embed uncached items ──
        let remote = if self.use_remote() {
            self.remote.as_ref().filter(|r| r.is_available())
        } else {
            None
        };

        if let Some(remote_embedder) = remote {
            // Remote path: one-by-one (no batch API on RemoteEmbedder)
            // with local fallback on failure.
            let insert_source = Self::remote_cache_source(remote_embedder);
            for (idx, text) in &uncached {
                let emb = match remote_embedder.embed(text).await {
                    Ok(emb) => emb,
                    Err(e) => {
                        if self.strategy == EmbeddingStrategy::RemoteOnly {
                            return Err(e);
                        }
                        tracing::warn!("Remote embedding failed, falling back to local: {}", e);
                        self.local.embed(text).await?
                    }
                };
                let key = self.cache_key_for(text, &insert_source);
                self.cache.insert(key, emb);
                results[*idx] = emb;
            }
        } else {
            // Local path: batch the embedding call to amortize
            // model-lock acquisition and ONNX inference overhead.
            let refs: Vec<&str> = uncached.iter().map(|(_, t)| t.as_str()).collect();
            let embeddings = self.local.embed_batch(&refs).await?;

            let insert_source = "local".to_string();
            for (j, (idx, text)) in uncached.iter().enumerate() {
                let emb = embeddings[j];
                let key = self.cache_key_for(text, &insert_source);
                self.cache.insert(key, emb);
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
        self.clear_router_cache();
    }

    fn cache_hits(&self) -> u64 {
        self.cache_hits.load(Ordering::Relaxed)
    }

    fn cache_misses(&self) -> u64 {
        self.cache_misses.load(Ordering::Relaxed)
    }
}

// ============================================================================
//  Existing EmbeddingService impl + trait impl
// ============================================================================

/// Async trait implementation for `crate::EmbeddingService`.
#[async_trait::async_trait]
impl crate::EmbeddingService for EmbeddingService {
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

    fn cache_hits(&self) -> u64 {
        self.cache_hits()
    }

    fn cache_misses(&self) -> u64 {
        self.cache_misses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    /// Helper: access the EmbeddingService trait on Router from tests.
    use crate::EmbeddingService as _;

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

    /// Helper: create a Router without calling the expensive fastembed constructor.
    /// Uses catch_unwind to skip gracefully when ONNX is unavailable (CI, sandbox).
    fn try_router() -> Option<EmbeddingRouter> {
        std::panic::catch_unwind(|| {
            EmbeddingRouter::new(EmbeddingService::new(), None, EmbeddingStrategy::LocalOnly)
        })
        .ok()
    }

    // ========================================================================
    //  EmbeddingRouter cache key & invalidation
    // ========================================================================

    #[test]
    fn test_router_cache_key_format() {
        // Strategy + source + text → deterministic, strategy-scoped key.
        let Some(router) = try_router() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let key_local = router.cache_key_for("hello", "local");
        let key_local2 = router.cache_key_for("hello", "local");
        assert_eq!(
            key_local, key_local2,
            "same (strategy,source,text) → same key"
        );

        let key_diff_text = router.cache_key_for("world", "local");
        assert_ne!(key_local, key_diff_text, "different text → different key");

        // Different source but same text.
        let key_b = router.cache_key_for("hello", "remote:OpenAi");
        assert_ne!(key_local, key_b, "different source → different key");

        assert!(
            key_local.contains("LocalOnly"),
            "key should embed strategy: {:?}",
            key_local
        );
    }

    #[test]
    fn test_router_lookup_source_changes_with_strategy() {
        let Some(mut router) = try_router() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        // LocalOnly, no remote → source should be "local".
        assert_eq!(router.lookup_source(), "local");

        // RemoteOnly with no remote → use_remote is false, source stays "local".
        router.strategy = EmbeddingStrategy::RemoteOnly;
        assert!(!router.use_remote(), "no remote → remote not used");

        // Back to LocalOnly.
        router.strategy = EmbeddingStrategy::LocalOnly;
        assert_eq!(router.lookup_source(), "local");
    }

    #[test]
    fn test_router_cache_clear_resets_counters() {
        let Some(router) = try_router() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        // Inject a miss manually via the private miss counter.
        router.cache_misses.fetch_add(5, Ordering::Relaxed);
        router.cache_hits.fetch_add(3, Ordering::Relaxed);
        assert_eq!(router.cache_misses(), 5);
        assert_eq!(router.cache_hits(), 3);

        router.clear_router_cache();
        assert_eq!(router.cache_misses(), 0, "misses reset after clear");
        assert_eq!(router.cache_hits(), 0, "hits reset after clear");
    }

    #[test]
    fn test_router_cache_clear_on_remote_toggle() {
        let Some(mut router) = try_router() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        // Simulate non-zero counters.
        router.cache_misses.fetch_add(10, Ordering::Relaxed);

        // clear_remote() calls clear_router_cache().
        router.clear_remote();
        assert_eq!(
            router.cache_misses(),
            0,
            "clear_remote() should reset cache counters"
        );

        // Simulate counters again.
        router.cache_misses.fetch_add(7, Ordering::Relaxed);

        // Remote toggle calls clear_router_cache() too.
        router.strategy = EmbeddingStrategy::LocalOnly;
        // No remote, so looking up source still works.
        assert_eq!(router.lookup_source(), "local");
    }

    // ========================================================================
    //  EmbeddingService locals
    // ========================================================================

    #[test]
    fn test_normalize_non_zero_preserves_direction() {
        let mut v = [0.0f32; EMBEDDING_DIM];
        v[0] = 2.0;
        v[1] = 2.0;
        let n = normalize_embedding(v);
        assert!((n[0] - n[1]).abs() < 1e-6, "equal components stay equal");
        let norm: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5, "unit norm: got {}", norm);
    }

    // ========================================================================
    //  EmbeddingRouter batch hit/miss compute
    // ========================================================================

    #[tokio::test]
    async fn test_router_embed_batch_counts_hits_and_misses() {
        let Some(router) = try_router() else {
            eprintln!("SKIP: ONNX/fastembed not available");
            return;
        };
        let texts = ["batch_alpha", "batch_beta", "batch_gamma"];

        // First call: all misses.
        let results = router.embed_batch(&texts).await.unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(router.cache_misses(), 3, "first call: 3 misses");
        assert_eq!(router.cache_hits(), 0, "first call: 0 hits");

        // Second call: all hits (same texts).
        let results2 = router.embed_batch(&texts).await.unwrap();
        assert_eq!(results2.len(), 3);
        assert_eq!(router.cache_misses(), 3, "misses unchanged after cache hit");
        assert_eq!(router.cache_hits(), 3, "second call: 3 hits");

        // Mixed: 2 cached + 1 new.
        let mixed = ["batch_alpha", "batch_beta", "batch_delta"];
        let results3 = router.embed_batch(&mixed).await.unwrap();
        assert_eq!(results3.len(), 3);
        assert_eq!(router.cache_misses(), 4, "1 new miss for batch_delta");
        assert_eq!(router.cache_hits(), 5, "2 cached hits");
    }
}
