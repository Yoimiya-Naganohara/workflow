use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing;

use crate::models::{CustomProvider, ModelRegistry};
use crate::tui::state::SelectedModel;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub selected_models: Vec<SelectedModel>,
    pub configured_providers: Vec<String>,
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    #[serde(default)]
    pub custom_providers: Vec<CustomProvider>,
}

fn config_dir() -> Result<PathBuf> {
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
    let config_dir = PathBuf::from(home).join(".workflow");
    std::fs::create_dir_all(&config_dir)?;
    Ok(config_dir)
}

fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("state.json"))
}

pub fn load() -> PersistedState {
    match config_file() {
        Ok(path) => {
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(text) => match serde_json::from_str(&text) {
                        Ok(state) => state,
                        Err(e) => {
                            tracing::error!("Failed to parse config: {}", e);
                            PersistedState::default()
                        }
                    },
                    Err(e) => {
                        tracing::error!("Failed to read config: {}", e);
                        PersistedState::default()
                    }
                }
            } else {
                PersistedState::default()
            }
        }
        Err(e) => {
            tracing::error!("Failed to get config path: {}", e);
            PersistedState::default()
        }
    }
}

pub fn save(state: &PersistedState) -> Result<()> {
    let path = config_file()?;
    let json = serde_json::to_string_pretty(state)?;
    write_atomic(&path, &json)
}

pub fn save_selected_models(models: &[SelectedModel]) -> Result<()> {
    let mut state = load();
    state.selected_models = models.to_vec();
    save(&state)
}

pub fn save_configured_provider(provider_id: &str, env_key: &str, api_key: &str) -> Result<()> {
    let mut state = load();
    if !state.configured_providers.contains(&provider_id.to_string()) {
        state.configured_providers.push(provider_id.to_string());
    }
    state.api_keys.insert(env_key.to_string(), api_key.to_string());
    save(&state)
}

pub fn load_api_keys() -> HashMap<String, String> {
    load().api_keys
}

pub fn load_custom_providers() -> Vec<CustomProvider> {
    load().custom_providers
}

pub fn save_custom_provider(provider: &CustomProvider) -> Result<()> {
    let mut state = load();
    let idx = state.custom_providers.iter().position(|p| p.id == provider.id);
    if let Some(i) = idx {
        state.custom_providers[i] = provider.clone();
    } else {
        state.custom_providers.push(provider.clone());
    }
    save(&state)
}

pub fn remove_custom_provider(custom_id: &str) -> Result<()> {
    let mut state = load();
    state.custom_providers.retain(|p| p.id != custom_id);
    save(&state)
}

pub fn save_provider_cache(registry: &ModelRegistry) -> Result<()> {
    let path = config_dir()?.join("providers_cache.json");
    let json = serde_json::to_string_pretty(registry)?;
    write_atomic(&path, &json)
}

pub fn load_provider_cache() -> Option<ModelRegistry> {
    let path = config_dir().ok()?.join("providers_cache.json");
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let file_name = path.file_name().and_then(|name| name.to_str()).unwrap_or("state.json");
    let temp_path = path.with_file_name(format!("{}.tmp-{}", file_name, uuid::Uuid::new_v4()));

    std::fs::write(&temp_path, contents)?;

    if path.exists() {
        std::fs::remove_file(path)?;
    }

    match std::fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp_path);
            Err(err.into())
        }
    }
}
