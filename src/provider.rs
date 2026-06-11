//! Provider client pool — cached LLM provider clients with health tracking.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::Result;
use tokio::sync::RwLock;

use crate::config::ProviderConfig;
use crate::llm::LlmProvider;

// ============================================================================
//  ProviderClient — a tracked LLM provider client
// ============================================================================

/// A tracked LLM provider client with health and usage metadata.
pub struct ProviderClient {
    /// The underlying LLM provider.
    pub inner: LlmProvider,
    /// Provider configuration snapshot.
    pub config: ProviderConfig,
    /// Whether the client is believed to be healthy.
    healthy: AtomicBool,
    /// Timestamp (monotonic ns) of last successful use.
    last_used: AtomicU64,
    /// Connection error count since last successful call.
    error_count: AtomicU64,
}

impl ProviderClient {
    pub fn new(config: ProviderConfig, inner: LlmProvider) -> Self {
        Self {
            inner,
            config,
            healthy: AtomicBool::new(true),
            last_used: AtomicU64::new(0),
            error_count: AtomicU64::new(0),
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    pub fn last_used(&self) -> Option<Instant> {
        let ns = self.last_used.load(Ordering::Relaxed);
        if ns == 0 {
            None
        } else {
            Some(Instant::now() - Duration::from_nanos(ns))
        }
    }

    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Mark a successful call.
    pub fn mark_success(&self) {
        self.healthy.store(true, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.last_used
            .store(Instant::now().elapsed().as_nanos() as u64, Ordering::Relaxed);
    }

    /// Mark a failed call. Marks unhealthy after 3 consecutive failures.
    pub fn mark_failure(&self) {
        let count = self.error_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count >= 3 {
            self.healthy.store(false, Ordering::Relaxed);
        }
    }

    /// Reset health (e.g. after successful reconnection).
    pub fn reset_health(&self) {
        self.healthy.store(true, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
    }

    /// Rebuild the underlying provider client (e.g. after API key change).
    pub fn rebuild(&mut self) -> Result<()> {
        let new_client = LlmProvider::from_protocol(
            &self.config.api_key,
            if self.config.base_url.is_empty() {
                None
            } else {
                Some(&self.config.base_url)
            },
            self.config.protocol,
        )?;
        self.inner = new_client;
        self.reset_health();
        Ok(())
    }
}

// ============================================================================
//  ClientPool — cached provider clients with TTL
// ============================================================================

/// A pool of LLM provider clients with TTL-based eviction.
///
/// Clients are created on first use and cached until TTL expiry or
/// health failure.  The pool is thread-safe and cheaply cloneable via
/// `Arc`.
pub struct ClientPool {
    clients: RwLock<HashMap<String, PoolEntry>>,
    ttl: Duration,
}

struct PoolEntry {
    client: Arc<ProviderClient>,
    created_at: Instant,
}

impl ClientPool {
    /// Create a new pool with the given TTL.
    ///
    /// After `ttl` of inactivity, an unused client is eligible for eviction.
    pub fn new(ttl: Duration) -> Self {
        Self {
            clients: RwLock::new(HashMap::new()),
            ttl,
        }
    }

    /// Number of cached clients.
    pub async fn len(&self) -> usize {
        self.clients.read().await.len()
    }

    /// Whether the pool is empty.
    pub async fn is_empty(&self) -> bool {
        self.clients.read().await.is_empty()
    }

    /// Get a client by provider ID, creating it if necessary.
    ///
    /// If a cached client exists and is healthy, returns it.
    /// If unhealthy but still within TTL, attempts to rebuild.
    /// If TTL expired, rebuilds the client.
    pub async fn get_or_create(&self, config: &ProviderConfig) -> Result<Arc<ProviderClient>> {
        // Fast path — existing healthy client
        {
            let clients = self.clients.read().await;
            if let Some(entry) = clients.get(&config.id) {
                if entry.client.is_healthy() {
                    entry.client.mark_success();
                    return Ok(entry.client.clone());
                }
            }
        }

        // Slow path — create or rebuild
        let mut clients = self.clients.write().await;

        // Re-check after acquiring write lock
        if let Some(entry) = clients.get(&config.id) {
            if entry.client.is_healthy() {
                entry.client.mark_success();
                return Ok(entry.client.clone());
            }
            // Unhealthy — remove and recreate below
            clients.remove(&config.id);
        }

        let inner = LlmProvider::from_protocol(
            &config.api_key,
            if config.base_url.is_empty() {
                None
            } else {
                Some(&config.base_url)
            },
            config.protocol,
        )?;

        let client = Arc::new(ProviderClient::new(config.clone(), inner));
        clients.insert(
            config.id.clone(),
            PoolEntry {
                client: client.clone(),
                created_at: Instant::now(),
            },
        );
        Ok(client)
    }

    /// Remove and return a client by ID (e.g. on API key change).
    pub async fn remove(&self, provider_id: &str) -> Option<Arc<ProviderClient>> {
        self.clients.write().await.remove(provider_id).map(|e| e.client)
    }

    /// Evict clients whose TTL has expired and are not in use.
    pub async fn evict_stale(&self) -> usize {
        let mut clients = self.clients.write().await;
        let before = clients.len();
        clients.retain(|_, entry| {
            // Keep if within TTL or unhealthy
            entry.created_at.elapsed() < self.ttl || !entry.client.is_healthy()
        });
        before - clients.len()
    }

    /// Remove all clients.
    pub async fn clear(&self) {
        self.clients.write().await.clear();
    }
}

impl Default for ClientPool {
    fn default() -> Self {
        Self::new(Duration::from_secs(3600))
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderProtocol;

    #[tokio::test]
    async fn test_pool_creates_client() {
        let pool = ClientPool::new(Duration::from_secs(60));
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let client = pool.get_or_create(&config).await.unwrap();
        assert!(client.is_healthy());
        assert_eq!(pool.len().await, 1);
    }

    #[tokio::test]
    async fn test_pool_returns_cached() {
        let pool = ClientPool::new(Duration::from_secs(60));
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let a = pool.get_or_create(&config).await.unwrap();
        let b = pool.get_or_create(&config).await.unwrap();
        // Same Arc — cached
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn test_pool_evict() {
        let pool = ClientPool::new(Duration::from_nanos(1)); // immediate expiry
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let _ = pool.get_or_create(&config).await.unwrap();
        // Yield to let time pass
        tokio::time::sleep(Duration::from_millis(10)).await;
        let evicted = pool.evict_stale().await;
        assert_eq!(evicted, 1);
    }
}
