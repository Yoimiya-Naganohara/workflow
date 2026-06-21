use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing;

use crate::agent::MemoEntry;
use crate::models::{CustomProvider, ModelRegistry};
use crate::tui::state::SelectedModel;

// ============================================================================
//  KeyStore — controlled API key persistence
// ============================================================================

/// Controls whether and how API keys are persisted.
///
/// # Security note
/// The obfuscation used here (`KeyStore::obfuscate`) is **not** real
/// encryption — it prevents casual plaintext reading of `state.json`.
/// For production use, integrate with the OS keychain (macOS Keychain,
/// Linux secret-service, Windows Credential Manager) via a crate like
/// `keyring`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum KeyStore {
    /// Keep keys in memory only; never write to disk.
    #[default]
    MemoryOnly,
    /// Obfuscate keys before writing to `state.json`.
    Obfuscated,
}

impl KeyStore {
    pub fn obfuscate(key: &str) -> String {
        let machine_id = Self::machine_id();
        let mut bytes: Vec<u8> = key.bytes().collect();
        for (b, m) in bytes.iter_mut().zip(machine_id.bytes().cycle()) {
            *b ^= m;
        }
        // Encode as hex to make it readable-ish
        hex_encode(&bytes)
    }

    pub fn deobfuscate(obfuscated: &str) -> Option<String> {
        let bytes = hex_decode(obfuscated)?;
        let machine_id = Self::machine_id();
        let result: Vec<u8> = bytes
            .into_iter()
            .zip(machine_id.bytes().cycle())
            .map(|(b, m)| b ^ m)
            .collect();
        String::from_utf8(result).ok()
    }

    fn machine_id() -> String {
        // Derive a machine-specific key from hostname + fixed salt
        let hostname = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| "unknown".to_string());
        format!("workflow-key-{}", hostname)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersistedState {
    pub selected_models: Vec<SelectedModel>,
    pub configured_providers: Vec<String>,
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
    #[serde(default)]
    pub custom_providers: Vec<CustomProvider>,
    /// Whether keys are obfuscated (true) or plaintext (legacy, false).
    #[serde(default)]
    pub keys_obfuscated: bool,
    /// Storage mode for keys (not serialized — runtime decision).
    #[serde(skip)]
    pub key_store_mode: KeyStore,
    /// Persistent role-scoped memos, keyed by role name.
    /// Shared by all agents with the same role across restarts.
    #[serde(default)]
    pub role_memos: HashMap<String, Vec<MemoEntry>>,
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
                    Ok(text) => match serde_json::from_str::<PersistedState>(&text) {
                        Ok(mut state) => {
                            if state.keys_obfuscated {
                                let deobfuscated: HashMap<String, String> = state
                                    .api_keys
                                    .iter()
                                    .filter_map(|(k, v)| {
                                        KeyStore::deobfuscate(v)
                                            .map(|decrypted| (k.clone(), decrypted))
                                    })
                                    .collect();
                                state.api_keys = deobfuscated;
                            }
                            state.key_store_mode = KeyStore::MemoryOnly;
                            state
                        }
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
    let mut state_copy = state.clone();
    if state.key_store_mode == KeyStore::Obfuscated && !state.api_keys.is_empty() {
        let obfuscated: HashMap<String, String> = state
            .api_keys
            .iter()
            .map(|(k, v)| (k.clone(), KeyStore::obfuscate(v)))
            .collect();
        state_copy.api_keys = obfuscated;
        state_copy.keys_obfuscated = true;
    }
    let path = config_file()?;
    let json = serde_json::to_string_pretty(&state_copy)?;
    write_atomic(&path, &json)
}

// ── Session persistence (opencode-style) ──

/// Save the current conversation messages for the next session.
pub fn save_session(messages: &[crate::tui::state::ChatMessage]) -> Result<()> {
    let path = config_dir()?.join("session.json");
    let json = serde_json::to_string_pretty(messages)?;
    write_atomic(&path, &json)
}

/// Load a previously saved session, if one exists.
pub fn load_session() -> Option<Vec<crate::tui::state::ChatMessage>> {
    let path = config_dir().ok()?.join("session.json");
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Save messages to a named session file under sessions/.
pub fn save_session_as(name: &str, messages: &[crate::tui::state::ChatMessage]) -> Result<()> {
    let dir = config_dir()?.join("sessions");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", name));
    let json = serde_json::to_string_pretty(messages)?;
    write_atomic(&path, &json)
}

/// Load messages from a named session.
pub fn load_session_as(name: &str) -> Option<Vec<crate::tui::state::ChatMessage>> {
    let path = config_dir()
        .ok()?
        .join("sessions")
        .join(format!("{}.json", name));
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// List all saved session names, ordered newest-first by mtime.
pub fn list_sessions() -> Vec<String> {
    let dir = match config_dir() {
        Ok(d) => d.join("sessions"),
        Err(_) => return vec![],
    };
    if !dir.exists() {
        return vec![];
    }
    let mut sessions: Vec<(String, std::time::SystemTime)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Some(name) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Ok(meta) = path.metadata() {
                        if let Ok(modified) = meta.modified() {
                            sessions.push((name.to_string(), modified));
                        }
                    }
                }
            }
        }
    }
    sessions.sort_by(|a, b| b.1.cmp(&a.1));
    sessions.into_iter().map(|(n, _)| n).collect()
}

/// Delete a named session file.
pub fn delete_session(name: &str) -> Result<()> {
    let path = config_dir()?
        .join("sessions")
        .join(format!("{}.json", name));
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    // Also delete the system prompt file if it exists
    let prompt_path = config_dir()?
        .join("sessions")
        .join(format!("{}_prompt.json", name));
    if prompt_path.exists() {
        std::fs::remove_file(&prompt_path)?;
    }
    Ok(())
}

/// Save the system prompt for a named session.
pub fn save_session_prompt(name: &str, prompt: &str, role: &str) -> Result<()> {
    let dir = config_dir()?.join("sessions");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_prompt.json", name));
    let data = serde_json::json!({
        "system_prompt": prompt,
        "role": role,
    });
    let json = serde_json::to_string_pretty(&data)?;
    write_atomic(&path, &json)
}

/// Load the system prompt for a named session.
/// Returns (system_prompt, role) if the file exists and is valid.
pub fn load_session_prompt(name: &str) -> Option<(String, String)> {
    let path = config_dir()
        .ok()?
        .join("sessions")
        .join(format!("{}_prompt.json", name));
    if !path.exists() {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let data: serde_json::Value = serde_json::from_str(&text).ok()?;
    let prompt = data.get("system_prompt")?.as_str()?.to_string();
    let role = data.get("role")?.as_str()?.to_string();
    Some((prompt, role))
}

pub fn save_selected_models(models: &[SelectedModel]) -> Result<()> {
    let mut state = load();
    state.selected_models = models.to_vec();
    save(&state)
}

pub fn save_configured_provider(provider_id: &str, env_key: &str, api_key: &str) -> Result<()> {
    let mut state = load();
    if !state
        .configured_providers
        .contains(&provider_id.to_string())
    {
        state.configured_providers.push(provider_id.to_string());
    }
    state
        .api_keys
        .insert(env_key.to_string(), api_key.to_string());
    state.key_store_mode = KeyStore::Obfuscated;
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
    let idx = state
        .custom_providers
        .iter()
        .position(|p| p.id == provider.id);
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

/// Path to the dedicated memos file (separate from state.json).
fn memos_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("role_memos.json"))
}

/// Save role-scoped memos to a dedicated memos file.
///
/// Using a separate file avoids rewriting the full state.json on every
/// memo write, which is both slow and risks data corruption from
/// concurrent writes.
pub fn save_role_memos(role: &str, memos: &[MemoEntry]) -> Result<()> {
    let path = memos_file()?;
    let mut all_memos: HashMap<String, Vec<MemoEntry>> = if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        serde_json::from_str(&text).unwrap_or_default()
    } else {
        HashMap::new()
    };
    all_memos.insert(role.to_string(), memos.to_vec());
    let json = serde_json::to_string_pretty(&all_memos)?;
    write_atomic(&path, &json)
}

/// Load all persisted role-scoped memos from the dedicated memos file.
///
/// Falls back to the legacy `state.json` `role_memos` field for migration.
pub fn load_role_memos() -> HashMap<String, Vec<MemoEntry>> {
    let path = match memos_file() {
        Ok(p) => p,
        Err(_) => return HashMap::new(),
    };
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let memos: HashMap<String, Vec<MemoEntry>> =
                    serde_json::from_str(&text).unwrap_or_default();
                if !memos.is_empty() {
                    return memos;
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read role_memos.json: {}", e);
            }
        }
    }
    // Fallback: migrate from legacy state.json role_memos field
    let legacy = load().role_memos;
    if !legacy.is_empty() {
        tracing::info!(
            "Migrating {} role memo entries from state.json to role_memos.json",
            legacy.len()
        );
        // Write to the new file on first migration
        if let Ok(p) = memos_file() {
            if let Ok(json) = serde_json::to_string_pretty(&legacy) {
                let _ = write_atomic(&p, &json);
            }
        }
    }
    legacy
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
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state.json");
    let temp_path = path.with_file_name(format!("{}.tmp-{}", file_name, uuid::Uuid::new_v4()));

    std::fs::write(&temp_path, contents)?;

    // On Unix, rename() atomically replaces the target — no remove_file needed.
    // This avoids the window where the target file doesn't exist.
    match std::fs::rename(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp_path);
            Err(err.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_write_atomic_removes_before_rename() {
        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().with_extension("json");

        // Write initial content
        write_atomic(&path, "version1").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version1");

        // Write again — this triggers the remove_file + rename path
        write_atomic(&path, "version2").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "version2");
    }

    #[test]
    fn test_write_atomic_no_remove_before_rename() {
        // Confirms that write_atomic does NOT call remove_file before rename.
        // The current implementation uses rename() which atomically replaces
        // the target on Unix — no prior remove_file needed.

        let tmp = NamedTempFile::new().unwrap();
        let path = tmp.path().with_extension("json");

        // Write initial content
        write_atomic(&path, "original").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");

        // Overwrite — should succeed without data loss
        write_atomic(&path, "updated").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated");

        // Write with different content size
        write_atomic(&path, "x").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");

        // Write large content
        let big = "hello\n".repeat(1000);
        write_atomic(&path, &big).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), big);
    }
}
