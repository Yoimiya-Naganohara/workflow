use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ── Model types ──────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub attachment: Option<bool>,
    #[serde(default)]
    pub reasoning: Option<bool>,
    #[serde(default)]
    pub reasoning_options: Vec<ReasoningOption>,
    #[serde(default)]
    pub tool_call: Option<bool>,
    #[serde(default)]
    pub structured_output: Option<bool>,
    #[serde(default)]
    pub temperature: Option<bool>,
    #[serde(default)]
    pub modalities: Option<Modalities>,
    #[serde(default)]
    pub limit: Option<Limit>,
    #[serde(default)]
    pub cost: Option<Cost>,
    #[serde(default)]
    pub interleaved: Option<InterleavedConfig>,
    #[serde(default)]
    pub open_weights: Option<bool>,
}

/// Helper: deserialize a Vec<String>, filtering out null elements and treating null field as empty.
fn string_vec_skip_nulls<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<Vec<Option<String>>> = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default().into_iter().flatten().collect())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ReasoningOption {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(default, deserialize_with = "string_vec_skip_nulls")]
    pub values: Vec<String>,
    #[serde(default)]
    pub min: Option<i64>,
    #[serde(default)]
    pub max: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Modalities {
    pub input: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Limit {
    pub context: usize,
    pub output: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum InterleavedConfig {
    Bool(bool),
    Object { field: String },
}

// ── Provider types ───────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub npm: Option<String>,
    #[serde(default)]
    pub doc: Option<String>,
    pub models: HashMap<String, ModelInfo>,
}

pub struct Providers {
    providers: Vec<ProviderInfo>,
}

impl Default for Providers {
    fn default() -> Self {
        Self::new()
    }
}

impl Providers {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub async fn fetch_provider_informations(&mut self, url: Option<&str>) {
        let url = url.unwrap_or("https://models.dev/api.json");
        if let Ok(response) = reqwest::get(url).await
            && let Ok(map) = response.json::<HashMap<String, ProviderInfo>>().await
        {
            self.providers = map.into_values().collect();
        }
    }

    pub fn get_providers(&self) -> &[ProviderInfo] {
        &self.providers
    }

    pub async fn load_from_file(&mut self, path: Option<&Path>) {
        let path = path.unwrap_or_else(|| Path::new("api.json"));
        match std::fs::read_to_string(path) {
            Ok(contents) if !contents.is_empty() => {
                match serde_json::from_str::<HashMap<String, ProviderInfo>>(&contents) {
                    Ok(map) => {
                        self.providers = map.into_values().collect();
                    }
                    Err(e) => {
                        eprintln!("providers: JSON deserialize error: {e}");
                    }
                }
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("providers: read file error: {e}");
            }
        }
    }

    pub async fn save_to_file(&self, path: Option<&Path>) {
        let path = path.unwrap_or_else(|| Path::new("api.json"));
        if let Ok(contents) = serde_json::to_string(&self.providers)
            && std::fs::write(path, contents).is_err()
        {
            eprintln!("providers: write file error: {path:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_from_file() {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let api_path = std::fs::canonicalize(manifest_dir.join("../../api.json")).unwrap();

        let mut providers = Providers::new();
        providers.load_from_file(Some(&api_path)).await;
        assert!(!providers.providers.is_empty(), "no providers loaded");
        if let Some(p) = providers.providers.first() {
            assert!(!p.models.is_empty(), "first provider has no models");
            println!(
                "loaded {} providers, first: {} ({} models)",
                providers.providers.len(),
                p.name,
                p.models.len()
            );
        }
    }
}
