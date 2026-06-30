//! Provider client — LLM provider client with health tracking.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;

use crate::config::ProviderConfig;
use crate::llm::{LlmProvider, LlmRequest, LlmResponse};

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
    /// Timestamp (epoch ns) of last successful use.
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

    pub fn last_used(&self) -> Option<Duration> {
        let ns = self.last_used.load(Ordering::Relaxed);
        if ns == 0 {
            None
        } else {
            let stored = UNIX_EPOCH + Duration::from_nanos(ns);
            SystemTime::now().duration_since(stored).ok()
        }
    }

    pub fn error_count(&self) -> u64 {
        self.error_count.load(Ordering::Relaxed)
    }

    /// Mark a successful call.
    pub fn mark_success(&self) {
        self.healthy.store(true, Ordering::Relaxed);
        self.error_count.store(0, Ordering::Relaxed);
        self.last_used.store(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            Ordering::Relaxed,
        );
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

    /// Call complete() with health tracking.
    pub async fn complete(&self, request: LlmRequest) -> Result<LlmResponse> {
        let result = self.inner.complete(request).await;
        match &result {
            Ok(_) => self.mark_success(),
            Err(_) => self.mark_failure(),
        }
        result
    }

    /// Call chat() with health tracking.
    pub async fn chat(&self, model: &str, system: &str, message: &str) -> Result<String> {
        let result = self.inner.chat(model, system, message).await;
        match &result {
            Ok(_) => self.mark_success(),
            Err(_) => self.mark_failure(),
        }
        result
    }
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ProviderProtocol;

    #[test]
    fn test_client_starts_healthy() {
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let inner = LlmProvider::from_protocol(&config.api_key, None, config.protocol).unwrap();
        let client = ProviderClient::new(config, inner);
        assert!(client.is_healthy());
        assert_eq!(client.error_count(), 0);
    }

    #[test]
    fn test_mark_failure_tracks_errors() {
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let inner = LlmProvider::from_protocol(&config.api_key, None, config.protocol).unwrap();
        let client = ProviderClient::new(config, inner);
        assert!(client.is_healthy());
        client.mark_failure();
        assert!(client.is_healthy());
        assert_eq!(client.error_count(), 1);
        client.mark_failure();
        assert!(client.is_healthy());
        assert_eq!(client.error_count(), 2);
        client.mark_failure();
        assert!(!client.is_healthy());
        assert_eq!(client.error_count(), 3);
    }

    #[test]
    fn test_mark_success_resets_errors() {
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let inner = LlmProvider::from_protocol(&config.api_key, None, config.protocol).unwrap();
        let client = ProviderClient::new(config, inner);
        client.mark_failure();
        client.mark_failure();
        client.mark_success();
        assert!(client.is_healthy());
        assert_eq!(client.error_count(), 0);
    }

    #[test]
    fn test_reset_health() {
        let config = ProviderConfig {
            id: "test".to_string(),
            name: "Test".to_string(),
            protocol: ProviderProtocol::OpenAiCompatible,
            api_key: "sk-test".to_string(),
            ..Default::default()
        };
        let inner = LlmProvider::from_protocol(&config.api_key, None, config.protocol).unwrap();
        let client = ProviderClient::new(config, inner);
        client.mark_failure();
        client.mark_failure();
        client.mark_failure();
        assert!(!client.is_healthy());
        client.reset_health();
        assert!(client.is_healthy());
        assert_eq!(client.error_count(), 0);
    }
}
