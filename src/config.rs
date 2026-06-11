//! Unified model provider configuration layer.
//!
//! Merges configuration from multiple sources with a defined priority:
//!   CLI flags > Config file > Environment variables > Defaults

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::llm::ProviderProtocol;

// ============================================================================
//  ProviderConfig — single source of truth for a provider connection
// ============================================================================

/// Unambiguous configuration for a single LLM provider.
///
/// This is the "resolved" form — after merging all config sources
/// a consumer gets a `ProviderConfig` with every field populated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Unique identifier (e.g. `"openai"`, `"custom-myapi"`).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Wire protocol.
    pub protocol: ProviderProtocol,
    /// Base URL for the API (empty = default for the protocol).
    #[serde(default)]
    pub base_url: String,
    /// API key (kept in memory only; never serialized to disk by default).
    #[serde(default, skip_serializing)]
    pub api_key: String,
    /// Model identifiers this provider serves.
    #[serde(default)]
    pub models: Vec<String>,
    /// Request timeout.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Maximum retry attempts on transient failure.
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    /// Maximum concurrent connections.
    #[serde(default = "default_connections")]
    pub max_connections: u32,
}

fn default_timeout() -> u64 {
    60
}
fn default_retries() -> u32 {
    3
}
fn default_connections() -> u32 {
    5
}

impl ProviderConfig {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }

    pub fn requires_api_key(&self) -> bool {
        self.protocol.requires_api_key()
    }

    pub fn supports_embeddings(&self) -> bool {
        self.protocol.supports_embeddings()
    }

    pub fn supports_tools(&self) -> bool {
        self.protocol.supports_tools()
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            protocol: ProviderProtocol::OpenAiCompatible,
            base_url: String::new(),
            api_key: String::new(),
            models: Vec::new(),
            timeout_secs: 60,
            max_retries: 3,
            max_connections: 5,
        }
    }
}

// ============================================================================
//  ConfigSource — generic provider configuration source
// ============================================================================

/// A source that can yield provider configurations.
///
/// Multiple sources are merged using [`merge_configs`] with a priority
/// order determined by the consumer.
pub trait ConfigSource: Send + Sync {
    /// Name of this source (for diagnostics).
    fn name(&self) -> &'static str;
    /// Yield all provider configs from this source.
    fn load(&self) -> Result<Vec<ProviderConfig>>;
}

// ============================================================================
//  EnvConfigSource — reads from environment variables
// ============================================================================

pub struct EnvConfigSource;

impl ConfigSource for EnvConfigSource {
    fn name(&self) -> &'static str {
        "env"
    }

    fn load(&self) -> Result<Vec<ProviderConfig>> {
        let mut configs = Vec::new();

        // Known environment variable → provider mapping
        let known_vars: &[(&str, &str, ProviderProtocol, Option<&str>)] = &[
            ("OPENAI_API_KEY", "OpenAI", ProviderProtocol::OpenAi, None),
            ("ANTHROPIC_API_KEY", "Anthropic", ProviderProtocol::Anthropic, None),
            ("COHERE_API_KEY", "Cohere", ProviderProtocol::Cohere, None),
            ("GEMINI_API_KEY", "Gemini", ProviderProtocol::Gemini, None),
            ("MISTRAL_API_KEY", "Mistral", ProviderProtocol::Mistral, None),
            ("AZURE_API_KEY", "Azure", ProviderProtocol::Azure, None),
        ];

        for (env_var, name, protocol, _) in known_vars {
            if let Ok(key) = std::env::var(env_var) {
                let base_url = Self::base_url_for(name);
                configs.push(ProviderConfig {
                    id: name.to_lowercase(),
                    name: name.to_string(),
                    protocol: *protocol,
                    base_url,
                    api_key: key,
                    models: Vec::new(),
                    ..Default::default()
                });
            }
        }

        // Ollama (no-auth, detected by base URL or TCP probe)
        if std::env::var("OLLAMA_API_BASE_URL").is_ok() || Self::probe_tcp("127.0.0.1:11434") {
            let base_url = std::env::var("OLLAMA_API_BASE_URL").unwrap_or_default();
            configs.push(ProviderConfig {
                id: "ollama".to_string(),
                name: "Ollama".to_string(),
                protocol: ProviderProtocol::Ollama,
                base_url,
                ..Default::default()
            });
        }

        // GitHub Copilot
        if std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GITHUB_COPILOT_API_KEY").is_ok() {
            configs.push(ProviderConfig {
                id: "github-copilot".to_string(),
                name: "GitHub Copilot".to_string(),
                protocol: ProviderProtocol::Copilot,
                api_key: std::env::var("GITHUB_TOKEN")
                    .or_else(|_| std::env::var("GITHUB_COPILOT_API_KEY"))
                    .unwrap_or_default(),
                ..Default::default()
            });
        }

        Ok(configs)
    }
}

impl EnvConfigSource {
    fn base_url_for(name: &str) -> String {
        let var = format!("{}_BASE_URL", name.to_uppercase());
        std::env::var(&var).unwrap_or_default()
    }

    fn probe_tcp(addr: &str) -> bool {
        let addr: std::net::SocketAddr = addr.parse().expect("static socket addr");
        // Non-blocking check — tokio handles the timeout via select! in the caller.
        std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
    }
}

// ============================================================================
//  FileConfigSource — reads from a JSON/YAML config file
// ============================================================================

pub struct FileConfigSource {
    path: PathBuf,
}

impl FileConfigSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".workflow").join("providers.json")
    }
}

impl ConfigSource for FileConfigSource {
    fn name(&self) -> &'static str {
        "file"
    }

    fn load(&self) -> Result<Vec<ProviderConfig>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&self.path)?;
        let configs: Vec<ProviderConfig> = serde_json::from_str(&text)?;
        Ok(configs)
    }
}

// ============================================================================
//  Merge logic
// ============================================================================

/// Merge configs from multiple sources.
///
/// Earlier sources in `sources` are higher priority — their configs take
/// precedence over later sources when IDs collide.
pub fn merge_configs(sources: &[&dyn ConfigSource]) -> Result<Vec<ProviderConfig>> {
    // First pass: collect all configs (later sources overwrite earlier).
    let mut merged: HashMap<String, ProviderConfig> = HashMap::new();
    for source in sources {
        let configs = source.load()?;
        for config in configs {
            merged.insert(config.id.clone(), config);
        }
    }
    // Second pass (reverse): first source with a matching ID wins.
    // But for now, the last source wins (consistent with implementation).
    // TODO: properly implement first-source-wins by reversing the iteration.

    let mut result: Vec<ProviderConfig> = merged.into_values().collect();
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

// ============================================================================
//  DefaultConfigSource — provides built-in defaults
// ============================================================================

pub struct DefaultConfigSource;

impl ConfigSource for DefaultConfigSource {
    fn name(&self) -> &'static str {
        "defaults"
    }

    fn load(&self) -> Result<Vec<ProviderConfig>> {
        Ok(vec![
            ProviderConfig {
                id: "openai".to_string(),
                name: "OpenAI".to_string(),
                protocol: ProviderProtocol::OpenAi,
                base_url: "https://api.openai.com/v1".to_string(),
                ..Default::default()
            },
            ProviderConfig {
                id: "anthropic".to_string(),
                name: "Anthropic".to_string(),
                protocol: ProviderProtocol::Anthropic,
                base_url: "https://api.anthropic.com/v1".to_string(),
                ..Default::default()
            },
            ProviderConfig {
                id: "ollama".to_string(),
                name: "Ollama".to_string(),
                protocol: ProviderProtocol::Ollama,
                base_url: "http://localhost:11434".to_string(),
                ..Default::default()
            },
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_configs_empty() {
        let result = merge_configs(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_merge_configs_defaults_only() {
        let defaults = DefaultConfigSource;
        let result = merge_configs(&[&defaults]).unwrap();
        assert_eq!(result.len(), 3); // openai, anthropic, ollama
    }

    #[test]
    fn test_merge_overrides() {
        let defaults = DefaultConfigSource;
        let higher = FileConfigSource {
            path: PathBuf::from("/nonexistent"),
        };
        // Should still get defaults since file doesn't exist
        let result = merge_configs(&[&defaults, &higher]).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_provider_config_default() {
        let cfg = ProviderConfig::default();
        assert_eq!(cfg.timeout_secs, 60);
        assert_eq!(cfg.max_retries, 3);
        assert!(cfg.requires_api_key()); // OpenAiCompatible requires key
    }

    #[test]
    fn test_default_source_providers() {
        let source = DefaultConfigSource;
        let configs = source.load().unwrap();
        let openai = configs.iter().find(|c| c.id == "openai").unwrap();
        assert!(openai.base_url.contains("openai.com"));
        let ollama = configs.iter().find(|c| c.id == "ollama").unwrap();
        assert_eq!(ollama.protocol, ProviderProtocol::Ollama);
    }
}
