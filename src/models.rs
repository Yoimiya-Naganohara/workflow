use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub env: Vec<String>,
    pub api: Option<String>,
    pub doc: Option<String>,
    pub models: HashMap<String, Model>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub family: Option<String>,
    #[serde(default)]
    pub attachment: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub tool_call: bool,
    #[serde(default = "default_true")]
    pub temperature: bool,
    pub knowledge: Option<String>,
    pub release_date: Option<String>,
    pub last_updated: Option<String>,
    pub modalities: Modalities,
    #[serde(default)]
    pub open_weights: bool,
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

pub(crate) fn matches_query(value: &str, query_lower: &str) -> bool {
    query_lower.is_empty() || value.to_lowercase().contains(query_lower)
}

pub(crate) fn filter_providers<'a>(providers: &'a [Provider], query: &str) -> Vec<&'a Provider> {
    if query.is_empty() {
        return providers.iter().collect();
    }
    let query_lower = query.to_lowercase();
    providers
        .iter()
        .filter(|p| matches_query(&p.name, &query_lower) || matches_query(&p.id, &query_lower))
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelLimit {
    pub context: u64,
    pub output: u64,
    #[serde(default)]
    pub input: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write: Option<f64>,
    #[serde(default)]
    pub reasoning: Option<f64>,
}

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
        let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build()?;

        let resp = client.get("https://models.dev/api.json").send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("Failed to fetch models: HTTP {}", status);
        }

        let text = resp.text().await?;
        let data: HashMap<String, Provider> =
            serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("Failed to parse models JSON: {}", e))?;

        self.providers = data
            .into_iter()
            .map(|(id, mut p)| {
                p.id = id;
                p
            })
            .collect();

        self.providers.sort_by(|a, b| a.name.cmp(&b.name));

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
        self.get_provider(provider_id).and_then(|p| p.models.get(model_id))
    }

    pub fn current_model(&self) -> Option<(&Provider, &Model)> {
        let provider_id = self.selected_provider.as_ref()?;
        let model_id = self.selected_model.as_ref()?;
        let provider = self.get_provider(provider_id)?;
        let model = provider.models.get(model_id)?;
        Some((provider, model))
    }

    /// Get the context window limit for a given model (provider_id + model_id).
    /// Returns 0 if the model or provider is not found.
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

        let query_lower2 = query_lower.clone();

        self.providers
            .iter()
            .filter(|p| {
                matches_query(&p.name, &query_lower)
                    || matches_query(&p.id, &query_lower)
                    || p.models.values().any(|m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family.as_deref().is_some_and(|f| matches_query(f, &query_lower))
                    })
            })
            .flat_map(move |p| {
                let query_lower = query_lower2.clone();
                p.models
                    .values()
                    .filter(move |m| {
                        matches_query(&m.name, &query_lower)
                            || matches_query(&m.id, &query_lower)
                            || m.family.as_deref().is_some_and(|f| matches_query(f, &query_lower))
                            || matches_query(&p.name, &query_lower)
                            || matches_query(&p.id, &query_lower)
                    })
                    .map(move |m| (p, m))
            })
            .collect()
    }
}

// ── Custom provider support ──

/// A user-defined custom provider (OpenAI-compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomProvider {
    pub id: String,
    pub name: String,
    pub api_url: String,
    pub api_key: String,
    pub models: Vec<String>,
}

impl CustomProvider {
    /// Build a provider slug from the custom name.
    pub fn slug(name: &str) -> String {
        let slug: String = name
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if slug.is_empty() { "custom".to_string() } else { slug }
    }
}

impl ModelRegistry {
    /// Add a custom provider into the registry so it appears in dialogs.
    pub fn add_custom_provider(&mut self, custom: &CustomProvider) {
        let id = format!("custom-{}", Self::sanitize_id(&custom.id));
        // Build model entries from the custom provider's model list
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

        // Replace existing or push new
        if let Some(pos) = self.providers.iter().position(|p| p.id == id) {
            self.providers[pos] = provider;
        } else {
            self.providers.push(provider);
        }
        self.providers.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Remove a custom provider by its custom ID.
    pub fn remove_custom_provider(&mut self, custom_id: &str) {
        let id = format!("custom-{}", Self::sanitize_id(custom_id));
        self.providers.retain(|p| p.id != id);
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
