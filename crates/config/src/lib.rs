use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use aes_gcm::{
    AeadCore, KeyInit,
    aead::{OsRng, Aead},
    Aes256Gcm, Key, Nonce,
};
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use futures::Stream;
use serde::{Deserialize, Serialize};

// ============================================================================
//  ProviderProtocol — maps to a rig provider client type
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderProtocol {
    OpenAi,
    OpenAiCompatible,
    Anthropic,
    Cohere,
    Gemini,
    Mistral,
    Ollama,
    Llamafile,
    Azure,
    Copilot,
}

impl ProviderProtocol {
    pub fn from_id(provider_id: &str) -> Self {
        match provider_id {
            "openai" => Self::OpenAi,
            "anthropic" => Self::Anthropic,
            "cohere" => Self::Cohere,
            "gemini" | "google" => Self::Gemini,
            "mistral" => Self::Mistral,
            "ollama" => Self::Ollama,
            "llamafile" => Self::Llamafile,
            "azure" => Self::Azure,
            "github-copilot" | "copilot" => Self::Copilot,
            _ if provider_id.starts_with("custom-") => Self::OpenAiCompatible,
            _ => Self::OpenAiCompatible,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::OpenAiCompatible => "OpenAI Compatible",
            Self::Anthropic => "Anthropic",
            Self::Cohere => "Cohere",
            Self::Gemini => "Gemini",
            Self::Mistral => "Mistral",
            Self::Ollama => "Ollama",
            Self::Llamafile => "Llamafile",
            Self::Azure => "Azure",
            Self::Copilot => "GitHub Copilot",
        }
    }

    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Self::Ollama | Self::Llamafile)
    }

    pub fn supports_embeddings(&self) -> bool {
        matches!(
            self,
            Self::OpenAi | Self::OpenAiCompatible | Self::Cohere | Self::Gemini | Self::Mistral
        )
    }

    pub fn supports_tools(&self) -> bool {
        !matches!(self, Self::Llamafile)
    }
}

impl fmt::Display for ProviderProtocol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ============================================================================
//  Message / Request / Response types
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub temperature: f64,
    pub max_tokens: u64,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub max_retries: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tokens_used: u32,
    pub cached_input_tokens: u32,
    pub cache_creation_input_tokens: u32,
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<String>> + Send>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneReason {
    Normal,
    LoopTerminated,
    StreamError,
}

#[derive(Debug, Clone)]
pub enum ToolEvent {
    AgentStart,
    AgentEnd,
    TurnStart,
    TurnEnd,
    MessageStart,
    MessageEnd,
    Text(String),
    Reasoning(String),
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    TokenUsage {
        input: u32,
        output: u32,
        cached_input: u32,
        cache_creation_input: u32,
        reasoning_tokens: u32,
    },
    Done {
        reason: DoneReason,
    },
}

pub type ToolChatStream = Pin<Box<dyn Stream<Item = ToolEvent> + Send>>;

// ============================================================================
//  ProviderConfig — single source of truth for a provider connection
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub protocol: ProviderProtocol,
    #[serde(default)]
    pub base_url: String,
    #[serde(default, skip_serializing)]
    pub api_key: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default = "default_retries")]
    pub max_retries: u32,
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

pub trait ConfigSource: Send + Sync {
    fn name(&self) -> &'static str;
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

        let known_vars: &[(&str, &str, ProviderProtocol, Option<&str>)] = &[
            ("OPENAI_API_KEY", "OpenAI", ProviderProtocol::OpenAi, None),
            (
                "ANTHROPIC_API_KEY",
                "Anthropic",
                ProviderProtocol::Anthropic,
                None,
            ),
            ("COHERE_API_KEY", "Cohere", ProviderProtocol::Cohere, None),
            ("GEMINI_API_KEY", "Gemini", ProviderProtocol::Gemini, None),
            (
                "MISTRAL_API_KEY",
                "Mistral",
                ProviderProtocol::Mistral,
                None,
            ),
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

        if std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GITHUB_COPILOT_API_KEY").is_ok()
        {
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
        let Ok(addr) = addr.parse::<std::net::SocketAddr>() else {
            return false;
        };
        std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
    }
}

// ============================================================================
//  FileConfigSource — reads from a JSON config file
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

pub fn merge_configs(sources: &[&dyn ConfigSource]) -> Result<Vec<ProviderConfig>> {
    let mut merged: HashMap<String, ProviderConfig> = HashMap::new();
    for source in sources {
        let configs = source.load()?;
        for config in configs {
            merged.insert(config.id.clone(), config);
        }
    }

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

// ============================================================================
//  UserConfig — persisted user preferences (last-used provider, model, api key)
//  The API key is encrypted at rest using AES-256-GCM.
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserConfig {
    pub selected_provider: String,
    pub selected_model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub encrypted: bool,
}

impl UserConfig {
    pub fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(f) = std::fs::File::create(&path) {
                let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
            }
        }
        let mut data = self.clone();
        if !data.api_key.is_empty() {
            data.api_key = encrypt_api_key(&data.api_key)?;
            data.encrypted = true;
        } else {
            data.encrypted = false;
        }
        let json = serde_json::to_string_pretty(&data)?;
        std::fs::write(&path, json)?;
        Ok(())
    }

    pub fn load() -> Result<Option<Self>> {
        let path = Self::path();
        if !path.exists() {
            return Ok(None);
        }
        let json = std::fs::read_to_string(&path)?;
        let mut config: UserConfig = serde_json::from_str(&json)?;
        if config.encrypted && !config.api_key.is_empty() {
            config.api_key = decrypt_api_key(&config.api_key)?;
        }
        config.encrypted = false;
        Ok(Some(config))
    }

    fn path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".workflow").join("config.json")
    }
}

fn enc_key_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".workflow").join(".encryption_key")
}

fn load_or_generate_key() -> Result<[u8; 32]> {
    let path = enc_key_path();
    if path.exists() {
        let data = std::fs::read(&path)?;
        if data.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&data);
            return Ok(key);
        }
    }
    let mut key = [0u8; 32];
    use aes_gcm::aead::rand_core::RngCore;
    aes_gcm::aead::OsRng.fill_bytes(&mut key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(f) = std::fs::File::create(&path) {
            let _ = f.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
    }
    std::fs::write(&path, &key)?;
    Ok(key)
}

fn encrypt_api_key(plaintext: &str) -> Result<String> {
    let key_bytes = load_or_generate_key()?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
    let mut buf = nonce.to_vec();
    buf.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&buf))
}

fn decrypt_api_key(encoded: &str) -> Result<String> {
    let key_bytes = load_or_generate_key()?;
    let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
    let cipher = Aes256Gcm::new(key);
    let data = BASE64
        .decode(encoded)
        .map_err(|e| anyhow::anyhow!("base64 decode failed: {e}"))?;
    if data.len() < 12 {
        anyhow::bail!("invalid encrypted data");
    }
    let (nonce_bytes, ciphertext) = data.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| anyhow::anyhow!("invalid utf-8: {e}"))
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
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_merge_overrides() {
        let defaults = DefaultConfigSource;
        let higher = FileConfigSource {
            path: PathBuf::from("/nonexistent"),
        };
        let result = merge_configs(&[&defaults, &higher]).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_provider_config_default() {
        let cfg = ProviderConfig::default();
        assert_eq!(cfg.timeout_secs, 60);
        assert_eq!(cfg.max_retries, 3);
        assert!(cfg.requires_api_key());
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
