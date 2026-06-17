use super::*;
use anyhow::Result;
use rig::client::{Nothing, ProviderClient};
use rig::providers::anthropic;
use rig::providers::azure;
use rig::providers::cohere;
use rig::providers::copilot;
use rig::providers::gemini;
use rig::providers::llamafile;
use rig::providers::mistral;
use rig::providers::ollama;
use rig::providers::openai;

impl LlmProvider {
    /// Build a provider from environment variables.
    ///
    /// Checks `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, `COHERE_API_KEY`,
    /// `GEMINI_API_KEY`, `MISTRAL_API_KEY`, `OLLAMA_API_BASE_URL`,
    /// `AZURE_API_KEY`, and `GITHUB_TOKEN` in order.
    pub fn from_env() -> Result<Self> {
        if let Ok(key) = std::env::var("OPENAI_API_KEY") {
            let mut builder = openai::CompletionsClient::builder().api_key(&key);
            if let Ok(url) = std::env::var("OPENAI_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::OpenAi(builder.build()?));
        }
        if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
            let mut builder = anthropic::Client::builder().api_key(&key);
            if let Ok(url) = std::env::var("ANTHROPIC_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::Anthropic(builder.build()?));
        }
        if let Ok(key) = std::env::var("COHERE_API_KEY") {
            return Ok(Self::Cohere(cohere::Client::new(key)?));
        }
        if let Ok(key) = std::env::var("GEMINI_API_KEY") {
            return Ok(Self::Gemini(gemini::Client::new(key)?));
        }
        if let Ok(key) = std::env::var("MISTRAL_API_KEY") {
            return Ok(Self::Mistral(mistral::Client::new(&key)?));
        }
        if std::env::var("OLLAMA_API_BASE_URL").is_ok() || Self::is_ollama_running() {
            let mut builder = ollama::Client::builder().api_key(Nothing);
            if let Ok(url) = std::env::var("OLLAMA_API_BASE_URL") {
                builder = builder.base_url(&url);
            }
            return Ok(Self::Ollama(builder.build()?));
        }
        if let Ok(key) = std::env::var("AZURE_API_KEY") {
            let endpoint = std::env::var("AZURE_ENDPOINT")
                .map_err(|_| anyhow::anyhow!("AZURE_ENDPOINT must be set with AZURE_API_KEY"))?;
            let api_version =
                std::env::var("AZURE_API_VERSION").unwrap_or_else(|_| "2024-10-21".to_string());
            return Ok(Self::Azure(
                azure::Client::builder()
                    .api_key(&key)
                    .azure_endpoint(endpoint)
                    .api_version(&api_version)
                    .build()?,
            ));
        }
        if std::env::var("GITHUB_TOKEN").is_ok() || std::env::var("GITHUB_COPILOT_API_KEY").is_ok()
        {
            return Ok(Self::Copilot(copilot::Client::from_env()?));
        }
        anyhow::bail!(
            "No API key found. Set OPENAI_API_KEY, ANTHROPIC_API_KEY, COHERE_API_KEY, \
             GEMINI_API_KEY, MISTRAL_API_KEY, AZURE_API_KEY, or GITHUB_TOKEN"
        )
    }

    fn is_ollama_running() -> bool {
        std::net::TcpStream::connect_timeout(
            &"127.0.0.1:11434".parse().expect("static socket addr"),
            std::time::Duration::from_millis(200),
        )
        .is_ok()
    }

    /// Build a provider from an API key + optional base URL + protocol.
    pub fn from_key(api_key: &str, base_url: Option<&str>, provider_id: &str) -> Result<Self> {
        let protocol = ProviderProtocol::from_id(provider_id);
        Self::from_protocol(api_key, base_url, protocol)
    }

    /// Build a provider from an API key + optional base URL + protocol.
    pub fn from_protocol(
        api_key: &str,
        base_url: Option<&str>,
        protocol: ProviderProtocol,
    ) -> Result<Self> {
        match protocol {
            ProviderProtocol::Anthropic => {
                let mut builder = anthropic::Client::builder().api_key(api_key);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::Anthropic(builder.build()?))
            }
            ProviderProtocol::Cohere => Ok(Self::Cohere(cohere::Client::new(api_key)?)),
            ProviderProtocol::Gemini => Ok(Self::Gemini(gemini::Client::new(api_key)?)),
            ProviderProtocol::Mistral => Ok(Self::Mistral(mistral::Client::new(api_key)?)),
            ProviderProtocol::Ollama => {
                let mut builder = ollama::Client::builder().api_key(Nothing);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::Ollama(builder.build()?))
            }
            ProviderProtocol::Llamafile => {
                let url = base_url.unwrap_or("http://localhost:8080");
                Ok(Self::Llamafile(llamafile::Client::from_url(url)?))
            }
            ProviderProtocol::Azure => {
                let endpoint = base_url.unwrap_or("").to_string();
                let api_version =
                    std::env::var("AZURE_API_VERSION").unwrap_or_else(|_| "2024-10-21".to_string());
                Ok(Self::Azure(
                    azure::Client::builder()
                        .api_key(api_key)
                        .azure_endpoint(endpoint)
                        .api_version(&api_version)
                        .build()?,
                ))
            }
            ProviderProtocol::Copilot => Ok(Self::Copilot(
                copilot::Client::builder().api_key(api_key).build()?,
            )),
            ProviderProtocol::OpenAi | ProviderProtocol::OpenAiCompatible => {
                let mut builder = openai::CompletionsClient::builder().api_key(api_key);
                if let Some(url) = base_url {
                    builder = builder.base_url(url);
                }
                Ok(Self::OpenAi(builder.build()?))
            }
        }
    }
}
