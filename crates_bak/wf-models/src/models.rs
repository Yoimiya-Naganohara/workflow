use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

// ============================================================================
//  Provider/Model core types (from models.dev/api.json)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub env: Vec<String>,
    pub api: Option<String>,
    pub doc: Option<String>,
    #[serde(default)]
    pub models: HashMap<String, Model>,
}

impl Provider {
    /// Convert to a ProviderConfig with the given API key.
    pub fn to_provider_config(&self, api_key: &str) -> crate::config::ProviderConfig {
        let protocol = wf_llm::ProviderProtocol::from_id(&self.id);
        crate::config::ProviderConfig {
            id: self.id.clone(),
            name: self.name.clone(),
            protocol,
            base_url: self.api.clone().unwrap_or_default(),
            api_key: api_key.to_string(),
            models: self.models.keys().cloned().collect(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub family: Option<String>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub reasoning_options: Vec<ReasoningOption>,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default = "default_true")]
    pub temperature: bool,
    pub knowledge: Option<String>,
    pub release_date: Option<String>,
    pub last_updated: Option<String>,
    #[serde(default)]
    pub modalities: Modalities,
    #[serde(default)]
    pub open_weights: bool,
    #[serde(default)]
    pub limit: ModelLimit,
    #[serde(default = "default_cost")]
    pub cost: Cost,
    pub status: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_cost() -> Cost {
    Cost {
        input: 0.0,
        output: 0.0,
        cache_read: None,
        cache_write: None,
        reasoning: None,
    }
}

pub use wf_core::ReasoningOption;

// ============================================================================
//  ModelCapabilities — derived feature flags for a model
// ============================================================================

/// Human-readable capabilities summary for a model.
#[derive(Debug, Clone, Default)]
pub struct ModelCapabilities {
    pub supports_tool_call: bool,
    pub supports_reasoning: bool,
    pub supports_vision: bool,
    pub supports_attachment: bool,
    pub max_context: u64,
    pub max_output: u64,
    pub cost_input: f64,
    pub cost_output: f64,
}

impl Model {
    /// Derive capabilities from the model's metadata.
    pub fn capabilities(&self) -> ModelCapabilities {
        let has_vision = self
            .modalities
            .input
            .iter()
            .any(|m| m == "image" || m == "vision");
        let has_attachment = self.attachment || has_vision;
        ModelCapabilities {
            supports_tool_call: self.tool_call,
            supports_reasoning: self.reasoning,
            supports_vision: has_vision,
            supports_attachment: has_attachment,
            max_context: self.limit.context,
            max_output: self.limit.output,
            cost_input: self.cost.input,
            cost_output: self.cost.output,
        }
    }

    /// One-line capability badge string for UI display.
    /// E.g. `[T] [R] [V] [A] ctx:128K`
    pub fn capability_badge(&self) -> String {
        let caps = self.capabilities();
        let mut parts = Vec::new();
        if caps.supports_tool_call {
            parts.push("T");
        }
        if caps.supports_reasoning {
            parts.push("R");
        }
        if caps.supports_vision {
            parts.push("V");
        }
        if caps.supports_attachment {
            parts.push("A");
        }
        let badges = if parts.is_empty() {
            "-".to_string()
        } else {
            parts.join(" ")
        };

        let ctx = if caps.max_context >= 1024 {
            let k = (caps.max_context as f64 / 1024.0).round() as u64;
            format!("{}K", k)
        } else {
            caps.max_context.to_string()
        };

        format!("[{}] ctx:{}", badges, ctx)
    }
}

// ============================================================================
//  Modalities, Limit, Cost
// ============================================================================

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Modalities {
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelLimit {
    #[serde(default)]
    pub context: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub input: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<f64>,
}

// ============================================================================
//  ProviderSource trait — pluggable provider discovery
// ============================================================================

/// A source that yields [`Provider`] entries.
///
/// Multiple sources can be combined in a [`ProviderRegistry`].
#[async_trait::async_trait]
pub trait ProviderSource: Send + Sync {
    /// Human-readable name for diagnostics.
    fn name(&self) -> &'static str;
    /// Fetch providers from this source.
    async fn fetch(&self) -> Result<Vec<Provider>>;
    /// Priority — higher values override lower when merging.
    fn priority(&self) -> u8 {
        0
    }
}

// ============================================================================
//  ModelsDevSource — fetches from models.dev/api.json
// ============================================================================

pub struct ModelsDevSource {
    client: Option<reqwest::Client>,
}

impl Default for ModelsDevSource {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .ok(),
        }
    }
}

impl ModelsDevSource {
    pub fn new() -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to build HTTP client (TLS issue): {}", e))?;
        Ok(Self {
            client: Some(client),
        })
    }
}

// ── ETag persistence for conditional HTTP requests ──

fn etag_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".workflow").join("providers_etag")
}

fn read_etag() -> Option<String> {
    let path = etag_path();
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn write_etag(etag: &str) {
    if let Some(parent) = etag_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(etag_path(), etag);
}

/// Filter out providers/models with empty or null identifiers.
/// These entries would fail at runtime if kept.
fn filter_valid_providers(providers: Vec<Provider>) -> Vec<Provider> {
    providers
        .into_iter()
        .filter(|p| {
            if p.id.is_empty() || p.name.is_empty() {
                tracing::warn!("Skipping provider with empty id/name");
                return false;
            }
            true
        })
        .map(|mut p| {
            // Remove models with empty id/name
            p.models.retain(|_, m| {
                if m.id.is_empty() || m.name.is_empty() {
                    tracing::warn!("Skipping model with empty id/name in provider '{}'", p.id);
                    return false;
                }
                true
            });
            p
        })
        .collect()
}

#[async_trait::async_trait]
impl ProviderSource for ModelsDevSource {
    fn name(&self) -> &'static str {
        "models.dev"
    }

    async fn fetch(&self) -> Result<Vec<Provider>> {
        let client = self.client.as_ref().ok_or_else(|| {
            anyhow::anyhow!("HTTP client unavailable (TLS initialization failed)")
        })?;
        let etag = read_etag();
        let mut req = client.get("https://models.dev/api.json");
        if let Some(ref etag) = etag {
            req = req.header("If-None-Match", etag);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if status == 304 {
            // Not modified — keep existing cache.
            return Ok(Vec::new());
        }
        if !status.is_success() {
            anyhow::bail!("models.dev HTTP {}", status);
        }
        // Store new ETag for next request.
        if let Some(new_etag) = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
        {
            write_etag(new_etag);
        }
        let text = resp.text().await?;
        let data: HashMap<String, Provider> = serde_json::from_str(&text).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse models.dev/api.json ({} bytes): {}",
                text.len(),
                e
            )
        })?;
        let mut providers: Vec<Provider> = data
            .into_iter()
            .map(|(id, mut p)| {
                p.id = id;
                p
            })
            .collect();
        providers = filter_valid_providers(providers);
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(providers)
    }

    fn priority(&self) -> u8 {
        10
    }
}

// ============================================================================
//  LocalFileSource — reads from a local JSON file
// ============================================================================

pub struct LocalFileSource {
    path: PathBuf,
}

impl LocalFileSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".workflow")
            .join("local_providers.json")
    }
}

#[async_trait::async_trait]
impl ProviderSource for LocalFileSource {
    fn name(&self) -> &'static str {
        "local_file"
    }

    async fn fetch(&self) -> Result<Vec<Provider>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let text = tokio::fs::read_to_string(&self.path).await?;
        let data: HashMap<String, Provider> =
            serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("Parse error: {}", e))?;
        let mut providers: Vec<Provider> = data
            .into_iter()
            .map(|(id, mut p)| {
                p.id = id;
                p
            })
            .collect();
        providers = filter_valid_providers(providers);
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(providers)
    }

    fn priority(&self) -> u8 {
        5
    }
}

// ============================================================================
//  ProviderRegistry — multi-source provider aggregation
// ============================================================================

/// Aggregates providers from multiple [`ProviderSource`]s.
///
/// Higher-priority sources override lower-priority ones when provider IDs
/// collide.  This lets e.g. a local file override models.dev data.
pub struct ProviderRegistry {
    sources: Vec<Box<dyn ProviderSource>>,
    providers: Vec<Provider>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            providers: Vec::new(),
        }
    }

    pub fn add_source(&mut self, source: Box<dyn ProviderSource>) {
        self.sources.push(source);
    }

    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub fn get_provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn get_model(&self, provider_id: &str, model_id: &str) -> Option<&Model> {
        self.get_provider(provider_id)
            .and_then(|p| p.models.get(model_id))
    }

    pub fn set_providers(&mut self, providers: Vec<Provider>) {
        self.providers = providers;
    }

    /// Fetch from all sources and merge (higher priority wins on conflict).
    pub async fn fetch_all(&mut self) -> Result<()> {
        let mut merged: HashMap<String, (u8, Provider)> = HashMap::new();

        for source in &self.sources {
            match source.fetch().await {
                Ok(providers) => {
                    for p in providers {
                        let priority = source.priority();
                        let entry = merged.entry(p.id.clone());
                        match entry {
                            std::collections::hash_map::Entry::Occupied(mut e) => {
                                if priority > e.get().0 {
                                    e.insert((priority, p));
                                }
                            }
                            std::collections::hash_map::Entry::Vacant(e) => {
                                e.insert((priority, p));
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("Provider source '{}' failed: {}", source.name(), e);
                }
            }
        }

        let mut providers: Vec<Provider> = merged.into_values().map(|(_, p)| p).collect();
        providers.sort_by(|a, b| a.name.cmp(&b.name));
        self.providers = providers;
        Ok(())
    }

    /// Replace or add a single provider (e.g. custom provider).
    pub fn upsert_provider(&mut self, provider: Provider) {
        if let Some(pos) = self.providers.iter().position(|p| p.id == provider.id) {
            self.providers[pos] = provider;
        } else {
            self.providers.push(provider);
        }
        self.providers.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Remove a provider by ID.
    pub fn remove_provider(&mut self, id: &str) {
        self.providers.retain(|p| p.id != id);
    }

    pub fn search_models(&self, query: &str) -> Vec<(&Provider, &Model)> {
        if query.is_empty() {
            return self
                .providers
                .iter()
                .flat_map(|p| p.models.values().map(move |m| (p, m)))
                .collect();
        }
        let query_lower = query.to_lowercase();
        self.providers
            .iter()
            .filter(|p| {
                matches_query(&p.name, &query_lower)
                    || matches_query(&p.id, &query_lower)
                    || p.models.values().any(|m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family
                                .as_deref()
                                .is_some_and(|f| matches_query(f, &query_lower))
                    })
            })
            .flat_map(|p| {
                let query_lower = query_lower.clone();
                p.models
                    .values()
                    .filter(move |m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family
                                .as_deref()
                                .is_some_and(|f| matches_query(f, &query_lower))
                            || matches_query(&p.name, &query_lower)
                            || matches_query(&p.id, &query_lower)
                    })
                    .map(move |m| (p, m))
            })
            .collect()
    }

    pub fn get_context_limit(&self, provider_id: &str, model_id: &str) -> u64 {
        self.get_model(provider_id, model_id)
            .map(|m| m.limit.context)
            .unwrap_or(0)
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
//  Legacy ModelRegistry (backward compatible)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRegistry {
    providers: Vec<Provider>,
    selected_provider: Option<String>,
    selected_model: Option<String>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            selected_provider: None,
            selected_model: None,
        }
    }

    pub fn with_providers(providers: Vec<Provider>) -> Self {
        Self {
            providers,
            selected_provider: None,
            selected_model: None,
        }
    }

    pub async fn fetch(&mut self) -> Result<()> {
        let source = ModelsDevSource::new()?;
        let providers = source.fetch().await?;
        if !providers.is_empty() {
            self.providers = providers;
        }
        Ok(())
    }

    pub fn providers(&self) -> &[Provider] {
        &self.providers
    }

    pub fn selected_provider(&self) -> Option<&str> {
        self.selected_provider.as_deref()
    }

    pub fn selected_model(&self) -> Option<&str> {
        self.selected_model.as_deref()
    }

    pub fn select_provider(&mut self, id: &str) {
        self.selected_provider = Some(id.to_string());
        self.selected_model = None;
    }

    pub fn select_model(&mut self, id: &str) {
        self.selected_model = Some(id.to_string());
    }

    pub fn get_provider(&self, id: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn get_model(&self, provider_id: &str, model_id: &str) -> Option<&Model> {
        self.get_provider(provider_id)
            .and_then(|p| p.models.get(model_id))
    }

    pub fn current_model(&self) -> Option<(&Provider, &Model)> {
        let provider_id = self.selected_provider.as_ref()?;
        let model_id = self.selected_model.as_ref()?;
        let provider = self.get_provider(provider_id)?;
        let model = provider.models.get(model_id)?;
        Some((provider, model))
    }

    pub fn get_context_limit(&self, provider_id: &str, model_id: &str) -> u64 {
        self.get_model(provider_id, model_id)
            .map(|m| m.limit.context)
            .unwrap_or(0)
    }

    pub fn search_models(&self, query: &str) -> Vec<(&Provider, &Model)> {
        if query.is_empty() {
            return self
                .providers
                .iter()
                .flat_map(|p| p.models.values().map(move |m| (p, m)))
                .collect();
        }
        let query_lower = query.to_lowercase();
        self.providers
            .iter()
            .filter(|p| {
                matches_query(&p.name, &query_lower)
                    || matches_query(&p.id, &query_lower)
                    || p.models.values().any(|m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family
                                .as_deref()
                                .is_some_and(|f| matches_query(f, &query_lower))
                    })
            })
            .flat_map(|p| {
                let query_lower = query_lower.clone();
                p.models
                    .values()
                    .filter(move |m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family
                                .as_deref()
                                .is_some_and(|f| matches_query(f, &query_lower))
                            || matches_query(&p.name, &query_lower)
                            || matches_query(&p.id, &query_lower)
                    })
                    .map(move |m| (p, m))
            })
            .collect()
    }

    /// Search models, filtering to only configured providers.
    pub fn search_configured_models<'a>(
        &'a self,
        query: &str,
        configured_ids: &[String],
    ) -> Vec<(&'a Provider, &'a Model)> {
        let all = self.search_models(query);
        all.into_iter()
            .filter(|(p, _)| configured_ids.iter().any(|id| id == &p.id))
            .collect()
    }

    // ── Custom provider support ──

    pub fn add_custom_provider(&mut self, custom: &CustomProvider) {
        let id = format!("custom-{}", Self::sanitize_id(&custom.id));
        let models: HashMap<String, Model> = if custom.models.is_empty() {
            let mut m = HashMap::new();
            m.insert(
                "default".to_string(),
                Model {
                    id: "default".to_string(),
                    name: "Default Model".to_string(),
                    family: None,
                    attachment: false,
                    reasoning: false,
                    reasoning_options: vec![],
                    tool_call: true,
                    temperature: true,
                    knowledge: None,
                    release_date: None,
                    last_updated: None,
                    modalities: Modalities {
                        input: vec!["text".to_string()],
                        output: vec!["text".to_string()],
                    },
                    open_weights: false,
                    limit: ModelLimit {
                        context: 128000,
                        output: 4096,
                        input: None,
                    },
                    cost: default_cost(),
                    status: None,
                },
            );
            m
        } else {
            custom
                .models
                .iter()
                .map(|m_id| {
                    let model = Model {
                        id: m_id.clone(),
                        name: m_id.clone(),
                        family: None,
                        attachment: false,
                        reasoning: false,
                        reasoning_options: vec![],
                        tool_call: true,
                        temperature: true,
                        knowledge: None,
                        release_date: None,
                        last_updated: None,
                        modalities: Modalities {
                            input: vec!["text".to_string()],
                            output: vec!["text".to_string()],
                        },
                        open_weights: false,
                        limit: ModelLimit {
                            context: 128000,
                            output: 4096,
                            input: None,
                        },
                        cost: default_cost(),
                        status: None,
                    };
                    (m_id.clone(), model)
                })
                .collect()
        };

        let provider = Provider {
            id: id.clone(),
            name: custom.name.clone(),
            env: vec![format!(
                "CUSTOM_{}_API_KEY",
                Self::sanitize_id(&custom.id).to_uppercase()
            )],
            api: Some(custom.api_url.clone()),
            doc: None,
            models,
        };

        if let Some(pos) = self.providers.iter().position(|p| p.id == id) {
            self.providers[pos] = provider;
        } else {
            self.providers.push(provider);
        }
        self.providers.sort_by(|a, b| a.name.cmp(&b.name));
    }

    pub fn remove_custom_provider(&mut self, custom_id: &str) {
        let id = format!("custom-{}", Self::sanitize_id(custom_id));
        self.providers.retain(|p| p.id != id);
    }

    /// Ensure all well-known providers exist in the registry.
    /// This guarantees `/connect` always shows the full list of known providers
    /// even when the remote fetch fails or the local cache is stale.
    pub fn ensure_builtin_defaults(&mut self) {
        let models_for = |ids: &[&str]| -> HashMap<String, Model> {
            ids.iter()
                .map(|id| {
                    (
                        id.to_string(),
                        Model {
                            id: id.to_string(),
                            name: id.to_string(),
                            family: None,
                            attachment: false,
                            reasoning: false,
                            reasoning_options: vec![],
                            tool_call: true,
                            temperature: true,
                            knowledge: None,
                            release_date: None,
                            last_updated: None,
                            modalities: Modalities {
                                input: vec!["text".to_string()],
                                output: vec!["text".to_string()],
                            },
                            open_weights: false,
                            limit: ModelLimit {
                                context: 128000,
                                output: 4096,
                                input: None,
                            },
                            cost: default_cost(),
                            status: None,
                        },
                    )
                })
                .collect()
        };

        // All well-known providers with their env vars, API URLs, and model IDs.
        let defaults: &[(&str, &str, &[&str], &str, &[&str])] = &[
            (
                "openai",
                "OpenAI",
                &["OPENAI_API_KEY"],
                "https://api.openai.com/v1",
                &["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-3.5-turbo"],
            ),
            (
                "anthropic",
                "Anthropic",
                &["ANTHROPIC_API_KEY"],
                "https://api.anthropic.com/v1",
                &[
                    "claude-sonnet-4-20250514",
                    "claude-3.5-sonnet",
                    "claude-3-haiku",
                ],
            ),
            (
                "cohere",
                "Cohere",
                &["COHERE_API_KEY"],
                "https://api.cohere.ai/v1",
                &["command-r-plus", "command-r", "command"],
            ),
            (
                "gemini",
                "Gemini",
                &["GEMINI_API_KEY"],
                "https://generativelanguage.googleapis.com/v1beta",
                &["gemini-2.0-flash", "gemini-1.5-pro", "gemini-1.5-flash"],
            ),
            (
                "mistral",
                "Mistral",
                &["MISTRAL_API_KEY"],
                "https://api.mistral.ai/v1",
                &[
                    "mistral-large-latest",
                    "mistral-medium-latest",
                    "mistral-small-latest",
                ],
            ),
            (
                "azure",
                "Azure",
                &["AZURE_API_KEY"],
                "https://YOUR_RESOURCE.openai.azure.com",
                &["gpt-4o", "gpt-4o-mini"],
            ),
            (
                "github-copilot",
                "GitHub Copilot",
                &["GITHUB_TOKEN", "GITHUB_COPILOT_API_KEY"],
                "https://api.githubcopilot.com",
                &["gpt-4o-copilot", "claude-sonnet-4-copilot"],
            ),
            (
                "ollama",
                "Ollama",
                &[],
                "http://localhost:11434",
                &["llama3.1", "qwen2.5", "mistral", "codellama"],
            ),
            (
                "llamafile",
                "Llamafile",
                &[],
                "http://localhost:8080",
                &["default"],
            ),
        ];

        for (id, name, env_vars, api_url, model_ids) in defaults {
            if self.providers.iter().any(|p| p.id == *id) {
                continue;
            }
            self.providers.push(Provider {
                id: id.to_string(),
                name: name.to_string(),
                env: env_vars.iter().map(|s| s.to_string()).collect(),
                api: Some(api_url.to_string()),
                doc: None,
                models: models_for(model_ids),
            });
        }
    }

    fn sanitize_id(name: &str) -> String {
        name.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect()
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
//  Custom provider support
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub api_url: String,
    pub api_key: String,
    pub models: Vec<String>,
}

impl CustomProvider {
    pub fn slug(name: &str) -> String {
        let slug: String = name
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if slug.is_empty() {
            "custom".to_string()
        } else {
            slug
        }
    }
}

// ============================================================================
//  Filter helpers
// ============================================================================

pub(crate) fn matches_query(value: &str, query_lower: &str) -> bool {
    query_lower.is_empty() || value.to_lowercase().contains(query_lower)
}

pub fn filter_providers<'a>(providers: &'a [Provider], query: &str) -> Vec<&'a Provider> {
    if query.is_empty() {
        return providers.iter().collect();
    }
    let query_lower = query.to_lowercase();
    providers
        .iter()
        .filter(|p| matches_query(&p.name, &query_lower) || matches_query(&p.id, &query_lower))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_capabilities() {
        let model = Model {
            id: "test".to_string(),
            name: "Test".to_string(),
            family: None,
            attachment: false,
            reasoning: true,
            reasoning_options: vec![],
            tool_call: true,
            temperature: true,
            knowledge: None,
            release_date: None,
            last_updated: None,
            modalities: Modalities {
                input: vec!["text".to_string(), "image".to_string()],
                output: vec!["text".to_string()],
            },
            open_weights: false,
            limit: ModelLimit {
                context: 128000,
                output: 4096,
                input: None,
            },
            cost: default_cost(),
            status: None,
        };
        let caps = model.capabilities();
        assert!(caps.supports_tool_call);
        assert!(caps.supports_reasoning);
        assert!(caps.supports_vision);
        assert!(caps.supports_attachment);
        assert_eq!(caps.max_context, 128000);
    }

    #[test]
    fn test_capability_badge() {
        let model = Model {
            id: "test".to_string(),
            name: "Test".to_string(),
            family: None,
            attachment: false,
            reasoning: true,
            reasoning_options: vec![],
            tool_call: true,
            temperature: true,
            knowledge: None,
            release_date: None,
            last_updated: None,
            modalities: Modalities {
                input: vec!["text".to_string()],
                output: vec!["text".to_string()],
            },
            open_weights: false,
            limit: ModelLimit {
                context: 128000,
                output: 4096,
                input: None,
            },
            cost: default_cost(),
            status: None,
        };
        let badge = model.capability_badge();
        assert!(badge.contains("T"));
        assert!(badge.contains("R"));
        assert!(badge.contains("ctx:125K"));
    }

    #[test]
    fn test_provider_registry_empty() {
        let reg = ProviderRegistry::new();
        assert!(reg.providers().is_empty());
    }

    #[test]
    fn test_provider_registry_upsert() {
        let mut reg = ProviderRegistry::new();
        let p = Provider {
            id: "test".to_string(),
            name: "Test".to_string(),
            env: vec![],
            api: None,
            doc: None,
            models: HashMap::new(),
        };
        reg.upsert_provider(p);
        assert_eq!(reg.providers().len(), 1);
    }

    #[test]
    fn test_matches_query() {
        assert!(matches_query("GPT-4", "gpt"));
        assert!(!matches_query("GPT-4", "claude"));
        assert!(matches_query("", "")); // empty query matches everything
    }

    #[test]
    fn test_model_registry_default() {
        let reg = ModelRegistry::default();
        assert!(reg.providers().is_empty());
        assert!(reg.selected_model().is_none());
    }

    #[test]
    fn test_model_registry_select() {
        let mut reg = ModelRegistry::new();
        reg.select_provider("openai");
        assert_eq!(reg.selected_provider(), Some("openai"));
        reg.select_model("gpt-4");
        assert_eq!(reg.selected_model(), Some("gpt-4"));
    }

    #[test]
    fn test_get_context_limit_returns_zero_for_unknown() {
        let reg = ModelRegistry::new();
        assert_eq!(reg.get_context_limit("unknown", "unknown"), 0);
    }

    #[test]
    fn test_custom_provider_slug() {
        assert_eq!(CustomProvider::slug("My Custom API"), "mycustomapi");
        assert_eq!(CustomProvider::slug("my-custom-api"), "my-custom-api");
        assert_eq!(CustomProvider::slug("!!!invalid###"), "invalid");
        assert_eq!(CustomProvider::slug(""), "custom");
    }

    // ── ModelsDevSource integration tests ──

    #[tokio::test]
    async fn test_models_dev_fetch() {
        let source = ModelsDevSource::new().expect("source should build");
        let providers = source.fetch().await.expect("fetch should succeed");
        assert!(!providers.is_empty(), "should have at least one provider");

        let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert!(
            ids.contains(&"openai"),
            "should contain openai, got: {:?}",
            ids
        );
        assert!(
            ids.contains(&"anthropic"),
            "should contain anthropic, got: {:?}",
            ids
        );
        assert!(
            ids.contains(&"google") || ids.contains(&"gemini"),
            "should contain google/gemini, got: {:?}",
            ids
        );

        // Verify each provider has at least one model
        for p in &providers {
            assert!(
                !p.models.is_empty(),
                "provider '{}' should have at least one model",
                p.id
            );
        }

        // Verify openai has gpt-4o
        let openai = providers.iter().find(|p| p.id == "openai").unwrap();
        assert!(
            openai.models.contains_key("gpt-4o"),
            "openai should have gpt-4o"
        );
    }

    #[tokio::test]
    async fn test_models_dev_fetch_with_etag() {
        // Write a fake ETag first
        let etag_path = etag_path();
        let _ = std::fs::create_dir_all(etag_path.parent().unwrap());
        std::fs::write(&etag_path, "\"test-initial-etag\"").unwrap();

        let source = ModelsDevSource::new().expect("source should build");
        let result = source.fetch().await;

        // Clean up
        let _ = std::fs::remove_file(&etag_path);

        // Should succeed whether 200 or 304
        let providers = result.expect("fetch should not fail even with stale etag");
        // If server returned 304, providers will be empty
        if providers.is_empty() {
            // 304 case - that's fine, but verify the ETag was sent
            eprintln!("Server returned 304 (not modified) — ETag working");
        } else {
            // 200 case - fresh data
            assert!(providers.len() > 1);
        }
    }
}
