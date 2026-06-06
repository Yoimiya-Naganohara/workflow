//! Application controller — business-logic layer behind the TUI.
//!
//! The TUI handler delegates all non-presentation operations
//! (provider setup, model fetching, chat execution, persistence,
//! shell commands) to this module.  The controller never
//! imports crossterm or ratatui.

use std::sync::Arc;

use anyhow::Result;
use futures::future::AbortHandle;
use rig::client::Nothing;
use rig::providers::{llamafile, ollama};
use tokio::sync::RwLock;

use crate::agent::{Agent, AgentConfig, AgentPool, AgentStatus};
use crate::llm::LlmProvider;
use crate::tui::state::{AgentEntry, AppState, ChatMessage, MessageRole, MessageStatus, SelectedModel};

// ============================================================================
//  Provider management
// ============================================================================

pub fn is_no_auth_provider(provider_id: &str) -> bool {
    matches!(provider_id, "ollama" | "llamafile")
}

pub fn get_or_create_provider_client(state: &mut AppState, provider_id: &str) -> Result<Arc<LlmProvider>> {
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
        let client = match provider_id {
            "ollama" => {
                let mut builder = ollama::Client::builder().api_key(Nothing);
                if let Some(url) = provider.api.as_deref() {
                    builder = builder.base_url(url);
                }
                Arc::new(LlmProvider::Ollama(builder.build()?))
            }
            "llamafile" => {
                let url = provider.api.as_deref().unwrap_or("http://localhost:8080");
                Arc::new(LlmProvider::Llamafile(llamafile::Client::from_url(url)?))
            }
            _ => anyhow::bail!("unexpected no-auth provider: {}", provider_id),
        };
        state.provider_clients.insert(provider_id.to_string(), client.clone());
        return Ok(client);
    }

    let env_key = provider.env.first().cloned().unwrap_or_default();
    if env_key.is_empty() {
        anyhow::bail!("Provider {} has no env var configured", provider_id);
    }
    let api_key = state
        .api_keys
        .get(&env_key)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{} not set. Press Ctrl+P to configure.", env_key))?;
    let client = Arc::new(LlmProvider::from_key(&api_key, provider.api.as_deref(), provider_id)?);
    state.provider_clients.insert(provider_id.to_string(), client.clone());
    Ok(client)
}

pub fn setup_no_auth_provider(state: &mut AppState, provider_id: &str) {
    if state.configured_providers.contains(&provider_id.to_string()) {
        return;
    }
    state.configured_providers.push(provider_id.to_string());
    state.models.select_provider(provider_id);
    let _ = get_or_create_provider_client(state, provider_id);
    let provider_name = state
        .models
        .providers()
        .iter()
        .find(|p| p.id == provider_id)
        .map(|p| p.name.as_str())
        .unwrap_or(provider_id);
    let now = chrono::Local::now().format("%H:%M:%S").to_string();
    state.messages.push(ChatMessage {
        role: MessageRole::System,
        content: format!("{} configured (no API key required)", provider_name),
        timestamp: now,
        status: MessageStatus::Completed,
    });
}

// ============================================================================
//  Model registry
// ============================================================================

pub async fn fetch_model_registry(state: Arc<RwLock<AppState>>) {
    // Show cached providers immediately, if available
    {
        let mut s = state.write().await;
        if s.models.providers().is_empty() {
            if let Some(cached) = crate::persistence::load_provider_cache() {
                s.models = cached;
            }
        }
    }

    // Background fetch fresh data
    let state_clone = state.clone();
    tokio::spawn(async move {
        let mut registry = crate::models::ModelRegistry::new();
        match registry.fetch().await {
            Ok(()) => {
                let count = registry.providers().len();
                let _ = crate::persistence::save_provider_cache(&registry);
                let mut s = state_clone.write().await;
                s.models = registry;
                s.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("Loaded {} providers", count),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }
            Err(e) => {
                let mut s = state_clone.write().await;
                let is_empty = s.models.providers().is_empty();
                let msg = if is_empty {
                    format!("Failed to load providers: {}", e)
                } else {
                    format!("Background refresh failed: {}", e)
                };
                let status = if is_empty {
                    MessageStatus::Error
                } else {
                    MessageStatus::Completed
                };
                s.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: msg,
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status,
                });
            }
        }
    });
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
    let persisted = crate::persistence::load();
    state.selected_models = persisted.selected_models;
    state.configured_providers = persisted.configured_providers;
    state.api_keys.extend(persisted.api_keys);
    state.provider_clients.clear();

    if !state.configured_providers.is_empty() || !state.selected_models.is_empty() {
        if let Some(cached) = crate::persistence::load_provider_cache() {
            state.models = cached;
        }
    }
    if !state.selected_models.is_empty() {
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Loaded {} selected models", state.selected_models.len()),
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            status: MessageStatus::Completed,
        });
    }
    // Warm provider clients (clone to avoid borrow conflict)
    let warm_ids: Vec<_> = state.selected_models.iter().map(|s| s.provider_id.clone()).collect();
    for provider_id in warm_ids {
        let _ = get_or_create_provider_client(state, &provider_id);
    }
}

// ============================================================================
//  Shell command
// ============================================================================

pub fn execute_shell(state: &Arc<RwLock<AppState>>, arg: &str) {
    let state_clone = state.clone();
    let arg = arg.to_string();
    tokio::spawn(async move {
        let output = tokio::process::Command::new("sh").arg("-c").arg(&arg).output().await;
        let mut s = state_clone.write().await;
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let mut content = String::new();
                if !stdout.is_empty() {
                    content.push_str(&stdout);
                }
                if !stderr.is_empty() {
                    if !content.is_empty() {
                        content.push('\n');
                    }
                    content.push_str(&stderr);
                }
                if content.is_empty() {
                    content = format!("(exit code: {})", out.status.code().unwrap_or(-1));
                }
                s.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    timestamp: now,
                    status: MessageStatus::Completed,
                });
            }
            Err(e) => {
                s.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: format!("Error: {}", e),
                    timestamp: now,
                    status: MessageStatus::Error,
                });
            }
        }
    });
}

// ============================================================================
//  Chat / Agent execution
// ============================================================================

pub fn submit_chat(state: &Arc<RwLock<AppState>>, input: &str, response_index: usize, request_id: u64) -> AbortHandle {
    let state_clone = state.clone();
    let input_clone = input.to_string();
    let (abort_handle, abort_registration) = AbortHandle::new_pair();

    tokio::spawn(async move {
        use futures::future::Abortable;

        let task = async {
            let mut s = state_clone.write().await;

            let runtime = match &s.runtime {
                Some(r) => r.clone(),
                None => return Err::<String, anyhow::Error>(anyhow::anyhow!("Runtime not initialized")),
            };

            let selected = s.selected_models.first().cloned();
            if let Some(ref sel) = selected {
                let provider_id = sel.provider_id.clone();
                if !s.configured_providers.iter().any(|id| id == &provider_id) {
                    return Err(anyhow::anyhow!("Provider {} is not configured", provider_id));
                }
                if let Ok(client) = get_or_create_provider_client(&mut s, &provider_id) {
                    let mut rt = runtime.write().await;
                    rt.set_provider_from_state(client);
                    rt.set_default_model(&sel.model_id);
                }
            }
            drop(s);

            let rt = runtime.read().await;
            let pool = Arc::new(RwLock::new(AgentPool::new()));
            let result = rt.chat_with_goal(&input_clone, &pool).await?;
            Ok::<String, anyhow::Error>(result)
        };

        let result = Abortable::new(task, abort_registration).await;

        let mut s = state_clone.write().await;
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        match result {
            Ok(Ok(response)) => {
                if let Some(message) = s.messages.get_mut(response_index) {
                    message.content = if response.is_empty() {
                        "(no text response)".to_string()
                    } else {
                        response.clone()
                    };
                    message.status = MessageStatus::Completed;
                }
                if let Some(mut plan) = crate::plan::Plan::parse_from_response(&response) {
                    plan.goal = input_clone.clone();
                    s.current_plan = Some(plan);
                    s.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Plan detected. Type /apply to approve and execute.".to_string(),
                        timestamp: now.clone(),
                        status: MessageStatus::Completed,
                    });
                }
            }
            Ok(Err(e)) => {
                if let Some(message) = s.messages.get_mut(response_index) {
                    message.content = format!("Error: {}", e);
                    message.status = MessageStatus::Error;
                } else {
                    s.messages.push(ChatMessage {
                        role: MessageRole::Agent,
                        content: format!("Error: {}", e),
                        timestamp: now,
                        status: MessageStatus::Error,
                    });
                }
            }
            Err(_) => {
                if let Some(message) = s.messages.get_mut(response_index) {
                    message.content += " (cancelled)";
                    message.status = MessageStatus::Completed;
                }
            }
        }
        if s.active_chat_request_id == request_id {
            s.active_chat_abort = None;
            s.active_chat_requests = 0;
        }
    });

    abort_handle
}

// ============================================================================
//  Plan execution
// ============================================================================

pub fn execute_plan(state: &Arc<RwLock<AppState>>) {
    let state_clone = state.clone();
    tokio::spawn(async move {
        let tasks: Vec<(usize, String)> = {
            let s = state_clone.read().await;
            s.current_plan
                .as_ref()
                .map(|p| {
                    p.tasks
                        .iter()
                        .filter(|t| t.status == crate::plan::TaskStatus::Pending)
                        .map(|t| (t.id, t.description.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };

        for (task_id, task_desc) in tasks {
            // Mark task as running
            {
                let mut s = state_clone.write().await;
                if let Some(p) = &mut s.current_plan {
                    p.mark_task_running(task_id);
                }
                s.agents.push(AgentEntry {
                    id: format!("worker-{:03}", task_id),
                    name: format!("Task {}: {}", task_id, task_desc.chars().take(20).collect::<String>()),
                    status: crate::tui::state::AgentStatus::Running,
                    budget: 0,
                });
            }

            // Spawn worker agent
            let agent_pool = {
                let s = state_clone.read().await;
                s.agent_pool.clone()
            };
            {
                let mut pool = agent_pool.write().await;
                let agent = Agent {
                    id: rand::random(),
                    name: format!("worker-{}", task_id),
                    role: "worker".to_string(),
                    parent_id: None,
                    children: Vec::new(),
                    depth: 0,
                    goal: task_desc.clone(),
                    config: AgentConfig::default(),
                    status: AgentStatus::Planning,
                    result: None,
                    child_results: Vec::new(),
                };
                pool.add_agent(agent);
            }

            // Simulate execution
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Mark task completed
            {
                let mut s = state_clone.write().await;
                if let Some(p) = &mut s.current_plan {
                    p.mark_task_completed(task_id, "Completed".to_string());
                }
                if let Some(agent) = s.agents.iter_mut().find(|a| a.id == format!("worker-{:03}", task_id)) {
                    agent.status = crate::tui::state::AgentStatus::Completed;
                }
                s.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: format!("Task {} completed: {}", task_id, task_desc),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }
        }

        let mut s = state_clone.write().await;
        if let Some(p) = &s.current_plan
            && p.status == crate::plan::PlanStatus::Completed
        {
            s.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "Plan execution completed!".to_string(),
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                status: MessageStatus::Completed,
            });
        }
    });
}
