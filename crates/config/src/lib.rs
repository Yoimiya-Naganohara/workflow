use std::fmt;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use aes_gcm::{
    AeadCore, Aes256Gcm, Key, KeyInit, Nonce,
    aead::{Aead, OsRng},
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
    OpenAiCompatible,
}

impl ProviderProtocol {
    pub fn from_id(provider_id: &str) -> Self {
        match provider_id {
            _ => Self::OpenAiCompatible,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::OpenAiCompatible => "OpenAI Compatible",
        }
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
//  DefaultConfigSource — provides built-in defaults
// ============================================================================

pub struct DefaultConfigSource;

impl ConfigSource for DefaultConfigSource {
    fn name(&self) -> &'static str {
        "defaults"
    }

    fn load(&self) -> Result<Vec<ProviderConfig>> {
        Ok(vec![ProviderConfig {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            ..Default::default()
        }])
    }
}

// ============================================================================
//  ProjectConfig — describes an opened project directory
// ============================================================================

/// Configuration for an opened project that agents can work with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    /// Absolute path to the project root directory.
    pub path: String,
    /// Optional human-friendly name (defaults to directory name).
    #[serde(default)]
    pub name: String,
}

impl ProjectConfig {
    pub fn new(path: impl Into<String>) -> Self {
        let path = path.into();
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        Self { path, name }
    }

    pub fn name(&self) -> String {
        if self.name.is_empty() {
            std::path::Path::new(&self.path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.path.clone())
        } else {
            self.name.clone()
        }
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
    #[serde(default)]
    pub project_path: String,
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
    PathBuf::from(home)
        .join(".workflow")
        .join(".encryption_key")
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
    std::fs::write(&path, key)?;
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
    fn test_provider_config_default() {
        let cfg = ProviderConfig::default();
        assert_eq!(cfg.timeout_secs, 60);
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn test_default_source_providers() {
        let source = DefaultConfigSource;
        let configs = source.load().expect("default config source should load");
        let openai = configs
            .iter()
            .find(|c| c.id == "openai")
            .expect("defaults should include openai");
        assert!(openai.base_url.contains("openai.com"));
    }
}
