//! Integration tests for the config module — merge_configs, ProviderConfig.
use workflow::config::*;

/// Re-export anyhow Result for the ConfigSource trait impl.
use anyhow::Result;

/// Verify that DefaultConfigSource provides the expected providers.
#[test]
fn test_default_config_source() {
    let source = DefaultConfigSource;
    let configs = source.load().expect("default configs should load");
    assert!(
        !configs.is_empty(),
        "should have at least one default provider"
    );

    let openai = configs
        .iter()
        .find(|c| c.id == "openai")
        .expect("defaults should include openai");
    assert_eq!(openai.name, "OpenAI");
    assert!(openai.base_url.contains("api.openai.com"));
    assert!(openai.requires_api_key());
    assert!(openai.supports_tools());

    let ollama = configs
        .iter()
        .find(|c| c.id == "ollama")
        .expect("defaults should include ollama");
    assert_eq!(ollama.name, "Ollama");
    assert!(!ollama.requires_api_key());
}

/// Verify merge_configs produces correctly merged output.
#[test]
fn test_merge_configs_deduplicates_by_id() {
    let source_a = SingleSource {
        configs: vec![ProviderConfig {
            id: "custom".into(),
            name: "Custom A".into(),
            protocol: workflow::llm::ProviderProtocol::OpenAiCompatible,
            base_url: "https://a.example.com".into(),
            ..Default::default()
        }],
    };
    let source_b = SingleSource {
        configs: vec![ProviderConfig {
            id: "custom".into(), // same id → overwrite
            name: "Custom B".into(),
            protocol: workflow::llm::ProviderProtocol::OpenAiCompatible,
            base_url: "https://b.example.com".into(),
            ..Default::default()
        }],
    };
    let merged = merge_configs(&[&source_a, &source_b])
        .expect("merge should succeed");
    // "custom" id should be deduplicated — source_b wins
    let custom = merged
        .iter()
        .find(|c| c.id == "custom")
        .expect("merged configs should include custom");
    assert_eq!(custom.name, "Custom B");
    assert_eq!(custom.base_url, "https://b.example.com");
    assert_eq!(merged.len(), 1);
}

/// Merge defaults with an empty override should still yield defaults.
#[test]
fn test_merge_defaults_with_empty_file() {
    let defaults = DefaultConfigSource;
    let empty = SingleSource { configs: vec![] };
    let merged = merge_configs(&[&defaults, &empty])
        .expect("merge with empty should succeed");
    assert_eq!(merged.len(), 3); // openai, anthropic, ollama
}

/// ProviderConfig default values are set correctly.
#[test]
fn test_provider_defaults() {
    let cfg = ProviderConfig::default();
    assert_eq!(cfg.timeout_secs, 60);
    assert_eq!(cfg.max_retries, 3);
    assert_eq!(cfg.max_connections, 5);
    assert!(cfg.models.is_empty());
}

/// Helper: injects a fixed set of configs for testing.
struct SingleSource {
    configs: Vec<ProviderConfig>,
}

impl ConfigSource for SingleSource {
    fn name(&self) -> &'static str {
        "single"
    }
    fn load(&self) -> Result<Vec<ProviderConfig>> {
        Ok(self.configs.clone())
    }
}
