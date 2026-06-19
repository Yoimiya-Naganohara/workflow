//! Application controller — synchronous business-logic layer.
//!
//! Async operations (shell, chat, pool query, model fetch) have been
//! moved to [`crate::tui::effect`].  This module only contains
//! synchronous helpers used by the dialogs and the handler.

use std::sync::Arc;

use anyhow::Result;
use rig::client::Nothing;
use rig::providers::{llamafile, ollama};

use crate::core::types::AgentId;
use crate::llm::LlmProvider;
use crate::models::CustomProvider;
use crate::provider::ProviderClient;
use crate::tui::state::{
    AgentEntry, AppState, ChatMessage, CoreState, MessageRole, MessageStatus, SelectedModel,
    UiState,
};

// ============================================================================
//  Agent setup (sync, called from handler)
// ============================================================================

/// Ensure a root agent exists. Returns the agent ID if available.
pub fn ensure_initial_agent_sync(core: &mut CoreState, goal_hint: &str) -> Option<AgentId> {
    if let Some(agent_id) = core.responsible_agent_id {
        if let Ok(pool) = core.agent_pool.try_read() {
            if pool.get_agent(&agent_id).is_some() {
                return Some(agent_id);
            }
        }
    }

    let runtime = match &core.runtime {
        Some(r) => r.clone(),
        None => return None,
    };

    let goal = if goal_hint.trim().is_empty() {
        "Own the user conversation, produce plans, and execute approved plans."
    } else {
        goal_hint
    };

    let agent_id = match runtime.try_read() {
        Ok(rt) => match core.agent_pool.try_write() {
            Ok(mut pool) => {
                // Evict stale and LRU agents before creating new ones
                pool.evict_stale(core.responsible_agent_id.as_ref());
                pool.evict_lru(core.responsible_agent_id.as_ref());

                // Try to reuse an idle agent (same role, Idle status)
                let existing = pool
                    .agents()
                    .iter()
                    .find(|a| {
                        a.role == core.default_role && a.status == crate::agent::AgentStatus::Idle
                    })
                    .map(|a| a.id);

                if let Some(existing_id) = existing {
                    pool.mark_active(&existing_id);
                    Some(existing_id)
                } else {
                    Some(rt.bootstrap_root_agent(goal, &core.default_role, &mut pool))
                }
            }
            Err(_) => return None,
        },
        Err(_) => return None,
    };

    if let Some(agent_id) = agent_id {
        core.responsible_agent_id = Some(agent_id);
        let entry = AgentEntry {
            id: crate::agent::AgentPool::agent_id_str(&agent_id),
            name: format!(
                "planner-{:04x}",
                u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])
            ),
            status: crate::tui::state::AgentStatus::Running,
            budget: 0,
        };
        if let Some(slot) = core.agents.iter_mut().find(|a| a.id == "agent-000") {
            *slot = entry;
        } else if !core.agents.iter().any(|a| a.id == entry.id) {
            core.agents.push(entry);
        }
        Some(agent_id)
    } else {
        None
    }
}

/// Switch to a named session: clear current messages and restore the saved ones.
/// Also restores the system prompt that was cached when the session was saved.
pub fn switch_session(core: &mut CoreState, ui: &mut UiState, name: &str) {
    let Some(messages) = crate::persistence::load_session_as(name) else {
        return;
    };

    // Clear current state
    core.messages.clear();
    core.responsible_agent_id = None;
    core.agents.clear();

    // Restore system prompt cache from saved session
    // This ensures the restored session uses the same system prompt as before
    if let Some((prompt, role)) = crate::persistence::load_session_prompt(name) {
        ui.cached_system_prompt = Some(prompt);
        ui.cached_prompt_role = role;
    } else {
        // No saved prompt - clear cache so it rebuilds on next message
        ui.cached_system_prompt = None;
        ui.cached_prompt_role.clear();
    }

    // Restore saved messages
    for msg in &messages {
        core.messages.push(ChatMessage {
            role: msg.role.clone(),
            content: msg.content.clone(),
            reasoning: String::new(),
            timestamp: msg.timestamp.clone(),
            status: MessageStatus::Completed,
        });
    }

    // Inject into agent context if an agent already exists
    if let Some(agent_id) = core.responsible_agent_id {
        if let Ok(mut pool) = core.agent_pool.try_write() {
            if let Some(agent) = pool.get_agent_mut(&agent_id) {
                agent.context.clear();
                for msg in &messages {
                    let role = match msg.role {
                        MessageRole::User => "user",
                        MessageRole::Agent => "assistant",
                        _ => continue,
                    };
                    agent.context.push(crate::llm::types::Message {
                        role: role.to_string(),
                        content: msg.content.clone(),
                    });
                }
            }
        }
    }
}

// ============================================================================
//  Provider management
// ============================================================================

pub fn is_no_auth_provider(provider_id: &str) -> bool {
    matches!(provider_id, "ollama" | "llamafile")
}

pub fn is_custom_provider(provider_id: &str) -> bool {
    provider_id.starts_with("custom-")
}

pub fn get_or_create_provider_client(
    state: &mut CoreState,
    provider_id: &str,
) -> Result<Arc<ProviderClient>> {
    if let Some(client) = state.provider_clients.get(provider_id) {
        return Ok(client.clone());
    }

    let provider = state
        .models
        .providers()
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| anyhow::anyhow!("Provider not found: {}", provider_id))?;

    if is_no_auth_provider(provider_id) {
        let llm_provider = match provider_id {
            "ollama" => {
                let mut builder = ollama::Client::builder().api_key(Nothing);
                if let Some(url) = provider.api.as_deref() {
                    builder = builder.base_url(url);
                }
                LlmProvider::Ollama(builder.build()?)
            }
            "llamafile" => {
                let url = provider.api.as_deref().unwrap_or("http://localhost:8080");
                LlmProvider::Llamafile(llamafile::Client::from_url(url)?)
            }
            _ => anyhow::bail!("unexpected no-auth provider: {}", provider_id),
        };
        let config = provider.to_provider_config("");
        let client = Arc::new(ProviderClient::new(config, llm_provider));
        state
            .provider_clients
            .insert(provider_id.to_string(), client.clone());
        return Ok(client);
    }

    let env_key = provider.env.first().cloned().unwrap_or_default();

    if is_custom_provider(provider_id) && !state.api_keys.contains_key(&env_key) {
        let llm_provider = LlmProvider::from_protocol(
            "",
            provider.api.as_deref(),
            crate::llm::ProviderProtocol::OpenAiCompatible,
        )?;
        let config = provider.to_provider_config("");
        let client = Arc::new(ProviderClient::new(config, llm_provider));
        state
            .provider_clients
            .insert(provider_id.to_string(), client.clone());
        return Ok(client);
    }

    if env_key.is_empty() {
        anyhow::bail!("Provider {} has no env var configured", provider_id);
    }
    let api_key = state
        .api_keys
        .get(&env_key)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{} not set. Use /connect to configure.", env_key))?;
    let llm_provider = LlmProvider::from_key(&api_key, provider.api.as_deref(), provider_id)?;
    let config = provider.to_provider_config(&api_key);
    let client = Arc::new(ProviderClient::new(config, llm_provider));
    state
        .provider_clients
        .insert(provider_id.to_string(), client.clone());
    Ok(client)
}

// ============================================================================
//  Persistence
// ============================================================================

pub fn save_api_key(provider_id: &str, env_key: &str, api_key: &str) -> Result<()> {
    crate::persistence::save_configured_provider(provider_id, env_key, api_key)
}

pub fn save_selected_models(models: &[SelectedModel]) -> Result<()> {
    crate::persistence::save_selected_models(models)
}

pub async fn load_initial_state(state: &mut AppState) {
    // Eagerly initialise the tiktoken BPE file so the status bar
    // token display works on first render (downloads ~1 MB on first run).
    crate::tui::tokenizer::init();

    let persisted = crate::persistence::load();
    state.core.selected_models = persisted.selected_models;
    state.core.configured_providers = persisted.configured_providers;
    state.core.api_keys.extend(persisted.api_keys);
    state.core.provider_clients.clear();

    merge_custom_providers(state);

    if !state.core.configured_providers.is_empty() || !state.core.selected_models.is_empty() {
        if let Some(cached) = crate::persistence::load_provider_cache() {
            state.core.models = cached;
        }
    }
    if !state.core.selected_models.is_empty() {
        if let Some(first) = state.core.selected_models.first() {
            state.ui.context_limit = state
                .core
                .models
                .get_context_limit(&first.provider_id, &first.model_id);
        }
        state.core.messages.push(ChatMessage::system(format!(
            "Loaded {} selected models",
            state.core.selected_models.len()
        )));
    }
    let warm_ids: Vec<_> = state
        .core
        .selected_models
        .iter()
        .map(|s| s.provider_id.clone())
        .collect();
    for provider_id in warm_ids {
        let _ = get_or_create_provider_client(&mut state.core, &provider_id);
    }

    // ── Sandbox re-hydration ──
    // Deserialised agents have sandbox: None (serde skip).  Rebuild the
    // sandbox handle from the persisted agent_id.  SandboxHandle::new()
    // is idempotent — existing work/ directories and src symlinks are
    // preserved, not overwritten.
    {
        let mut pool = state.core.agent_pool.write().await;
        for agent in pool.agents_mut() {
            let handle = crate::tools::sandbox::SandboxHandle::new(&agent.id)
                .map(std::sync::Arc::new)
                .ok();
            agent.sandbox = handle;
        }
    }

    // ── Load persisted session (opencode-style) ──
    if let Some(mut session) = crate::persistence::load_session() {
        let count = session.len();
        state.core.messages.append(&mut session);
        state.core.messages.push(ChatMessage::system(format!(
            "📋 Restored {} messages from previous session",
            count,
        )));
    }
}

// ============================================================================
//  Custom provider management
// ============================================================================

fn provider_env_key(custom_id: &str) -> String {
    format!("CUSTOM_{}_API_KEY", custom_id.to_uppercase())
}

/// Save a custom provider (called from the wizard dialog).
pub fn save_custom_provider(
    state: &mut CoreState,
    name: &str,
    url: &str,
    key: &str,
    models_str: &str,
) {
    let custom_id = CustomProvider::slug(name);
    let env_key = provider_env_key(&custom_id);

    let models: Vec<String> = if models_str.trim().is_empty() {
        vec!["default".to_string()]
    } else {
        models_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let provider = CustomProvider {
        id: custom_id.clone(),
        name: name.to_string(),
        api_url: url.to_string(),
        api_key: key.to_string(),
        models: models.clone(),
    };

    if let Err(e) = crate::persistence::save_custom_provider(&provider) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Failed to save custom provider: {}", e),
            reasoning: String::new(),
            timestamp: now,
            status: MessageStatus::Error,
        });
        return;
    }

    if !key.is_empty() {
        if !state.configured_providers.contains(&custom_id) {
            state.configured_providers.push(custom_id.clone());
        }
        state.api_keys.insert(env_key.clone(), key.to_string());
        let _ = crate::persistence::save_configured_provider(&custom_id, &env_key, key);
    } else if !state.configured_providers.contains(&custom_id) {
        state.configured_providers.push(custom_id.clone());
        let _ = crate::persistence::save_configured_provider(&custom_id, &env_key, "");
    }

    state.models.add_custom_provider(&provider);

    let provider_id = format!("custom-{}", custom_id);
    let _ = get_or_create_provider_client(state, &provider_id);

    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    let model_list = models.join(", ");
    state.messages.push(ChatMessage {
        role: MessageRole::System,
        content: format!(
            "Custom provider \"{}\" configured — {} model(s): {}",
            name,
            models.len(),
            model_list
        ),
        reasoning: String::new(),
        timestamp: now,
        status: MessageStatus::Completed,
    });
}

/// Load custom providers from persistence and merge them into the model registry.
pub fn merge_custom_providers(state: &mut AppState) {
    let custom_providers = crate::persistence::load_custom_providers();
    if custom_providers.is_empty() {
        return;
    }
    for cp in &custom_providers {
        let env_key = provider_env_key(&cp.id);
        if !cp.api_key.is_empty() {
            state
                .core
                .api_keys
                .insert(env_key.clone(), cp.api_key.clone());
            if !state.core.configured_providers.contains(&cp.id) {
                state.core.configured_providers.push(cp.id.clone());
            }
        }
        state.core.models.add_custom_provider(cp);
    }
    if !custom_providers.is_empty() {
        let count = custom_providers.len();
        state.core.messages.push(ChatMessage::system(format!(
            "Loaded {} custom provider(s)",
            count
        )));
    }
}
