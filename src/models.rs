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

    pub async fn fetch(&mut self) -> Result<()> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;

        let resp = client.get("https://models.dev/api.json").send().await?;
        let status = resp.status();
        if !status.is_success() {
            anyhow::bail!("Failed to fetch models: HTTP {}", status);
        }

        let text = resp.text().await?;
        let data: HashMap<String, Provider> = serde_json::from_str(&text)
            .map_err(|e| anyhow::anyhow!("Failed to parse models JSON: {}", e))?;

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

    pub fn search_models(&self, query: &str) -> Vec<(&Provider, &Model)> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for provider in &self.providers {
            for model in provider.models.values() {
                if model.name.to_lowercase().contains(&query_lower)
                    || model.id.to_lowercase().contains(&query_lower)
                    || model
                        .family
                        .as_ref()
                        .map(|f| f.to_lowercase().contains(&query_lower))
                        .unwrap_or(false)
                {
                    results.push((provider, model));
                }
            }
        }

        results.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        results
    }
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::new()
    }
}
