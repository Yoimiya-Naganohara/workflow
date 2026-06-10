//! Application controller — business-logic layer behind the TUI.
//!
//! The TUI handler delegates all non-presentation operations
//! (provider setup, model fetching, chat execution, persistence,
//! shell commands) to this module.  The controller never
//! imports crossterm or ratatui.

use std::sync::Arc;

use anyhow::Result;
use futures::{StreamExt, future::AbortHandle};
use rig::client::Nothing;
use rig::providers::{llamafile, ollama};
use tokio::sync::RwLock;

use crate::llm::{self, LlmProvider};
use crate::models::CustomProvider;
use crate::tui::state::{AgentEntry, AppState, ChatMessage, MessageRole, MessageStatus, SelectedModel};

// ============================================================================
//  Provider management
// ============================================================================

pub fn is_no_auth_provider(provider_id: &str) -> bool {
    matches!(provider_id, "ollama" | "llamafile")
}

pub fn is_custom_provider(provider_id: &str) -> bool {
    provider_id.starts_with("custom-")
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

    // Custom provider without API key (no-auth custom)
    if is_custom_provider(provider_id) && !state.api_keys.contains_key(&env_key) {
        let client = Arc::new(LlmProvider::from_protocol("", provider.api.as_deref(), crate::llm::ProviderProtocol::OpenAiCompatible)?);
        state.provider_clients.insert(provider_id.to_string(), client.clone());
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

    // Merge custom providers into the model registry
    merge_custom_providers(state);

    if !state.configured_providers.is_empty() || !state.selected_models.is_empty() {
        if let Some(cached) = crate::persistence::load_provider_cache() {
            state.models = cached;
        }
    }
    if !state.selected_models.is_empty() {
        if let Some(first) = state.selected_models.first() {
            state.context_limit = state.models.get_context_limit(&first.provider_id, &first.model_id);
        }
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
//  Pool commands
// ============================================================================

pub fn execute_pool_command(state: &Arc<RwLock<AppState>>, cmd: &str, now: &str) {
    let state_clone = state.clone();
    let cmd = cmd.to_string();
    let now = now.to_string();

    tokio::spawn(async move {
        let mut s = state_clone.write().await;

        let (content, status) = match cmd.as_str() {
            "stats" => {
                let stats = if let Some(runtime) = &s.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        let total = rt.experience_count();
                        let bedrock = rt.bedrock_count();
                        let fluid = rt.fluid_count();
                        let pending = rt.pending_suspended();
                        let remaining = rt.remaining_budget();
                        let permits = rt.available_permits();
                        [
                            "Experience Pool Statistics:".to_string(),
                            format!("  Total entries:    {}", total),
                            format!("  Bedrock (A-track): {}", bedrock),
                            format!("  Fluid  (B-track): {}", fluid),
                            format!("  Pending suspend:  {}", pending),
                            format!("  Remaining budget: {}", remaining),
                            format!("  Available permits:{}", permits),
                        ]
                        .join("\n")
                    } else {
                        "Runtime locked".to_string()
                    }
                } else {
                    "Runtime not available".to_string()
                };
                (stats, MessageStatus::Completed)
            }
            "flush" => {
                let result = if let Some(runtime) = &s.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        match rt.flush_experience_pool() {
                            Ok(()) => "Experience pool flushed to disk".to_string(),
                            Err(e) => format!("Flush failed: {}", e),
                        }
                    } else {
                        "Runtime locked".to_string()
                    }
                } else {
                    "Runtime not available".to_string()
                };
                let status = if result.contains("failed") {
                    MessageStatus::Error
                } else {
                    MessageStatus::Completed
                };
                (result, status)
            }
            "clear" => {
                let msg = if s.runtime.is_some() {
                    "Pool clear requires runtime write access — not available".to_string()
                } else {
                    "Runtime not available".to_string()
                };
                (msg, MessageStatus::Completed)
            }
            "export" => (
                "Export not yet implemented. Pool file is at ~/.workflow/experience_a.bin".to_string(),
                MessageStatus::Completed,
            ),
            "import" => ("Import not yet implemented".to_string(), MessageStatus::Completed),
            _ if cmd.starts_with("query ") || cmd.starts_with("q ") => {
                let query_text = cmd.splitn(2, ' ').nth(1).unwrap_or("").trim().to_string();
                if query_text.is_empty() {
                    ("Usage: /pool query <text>".to_string(), MessageStatus::Completed)
                } else if let Some(runtime) = &s.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        match rt.embed(&query_text).await {
                            Ok(emb) => {
                                let results = rt.search_experience(&emb, 10);
                                if results.is_empty() {
                                    ("No matching experiences found.".to_string(), MessageStatus::Completed)
                                } else {
                                    let lines: Vec<String> = results
                                        .iter()
                                        .enumerate()
                                        .map(|(i, (entry, score))| {
                                            let ts = entry.timestamp;
                                            format!(
                                                "  #{:<3} score={:.4}  weight={:.2}  ts={}  tools={:016b}",
                                                i + 1,
                                                score,
                                                entry.weight,
                                                ts,
                                                entry.tool_bitmap
                                            )
                                        })
                                        .collect();
                                    (
                                        format!(
                                            "Top {} experiences for \"{}\":\n{}",
                                            results.len(),
                                            query_text,
                                            lines.join("\n")
                                        ),
                                        MessageStatus::Completed,
                                    )
                                }
                            }
                            Err(e) => (format!("Embedding failed: {}", e), MessageStatus::Error),
                        }
                    } else {
                        ("Runtime locked".to_string(), MessageStatus::Completed)
                    }
                } else {
                    ("Runtime not available".to_string(), MessageStatus::Completed)
                }
            }
            _ => (
                format!("Unknown pool command: {}. Use /pool for help.", cmd),
                MessageStatus::Completed,
            ),
        };

        s.messages.push(ChatMessage {
            role: MessageRole::System,
            content,
            timestamp: now,
            status,
        });
        s.pool_stats.last_flush_result = Some("OK".to_string());
    });
}

// ============================================================================
//  Chat / Agent execution
// ============================================================================

async fn ensure_initial_agent(state: &Arc<RwLock<AppState>>, goal_hint: &str) -> Result<crate::core::types::AgentId> {
    let existing = {
        let s = state.read().await;
        if let Some(agent_id) = s.responsible_agent_id {
            let pool = s.agent_pool.read().await;
            if pool.get_agent(&agent_id).is_some() {
                Some(agent_id)
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(agent_id) = existing {
        return Ok(agent_id);
    }

    let (runtime, agent_pool) = {
        let s = state.read().await;
        let runtime = s
            .runtime
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Runtime not initialized"))?;
        (runtime, s.agent_pool.clone())
    };

    let goal = if goal_hint.trim().is_empty() {
        "Own the user conversation, produce plans, and execute approved plans."
    } else {
        goal_hint
    };

    let agent_id = {
        let runtime = runtime.read().await;
        let mut pool = agent_pool.write().await;
        runtime.bootstrap_root_agent(goal, "general_business_analyst", &mut pool)
    };

    let mut s = state.write().await;
    s.responsible_agent_id = Some(agent_id);
    let entry = AgentEntry {
        id: crate::agent::AgentPool::agent_id_str(&agent_id),
        name: format!("planner-{:04x}", u16::from(agent_id[0]) << 8 | u16::from(agent_id[1])),
        status: crate::tui::state::AgentStatus::Running,
        budget: 0,
    };
    if let Some(slot) = s.agents.iter_mut().find(|a| a.id == "agent-000") {
        *slot = entry;
    } else if !s.agents.iter().any(|a| a.id == entry.id) {
        s.agents.push(entry);
    }
    Ok(agent_id)
}

/// Find the slot index of the currently streaming message (fallback to last thinking/streaming).
fn find_streaming_slot(messages: &[ChatMessage], preferred: usize) -> usize {
    messages
        .get(preferred)
        .filter(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
        .map(|_| preferred)
        .unwrap_or_else(|| {
            messages
                .iter()
                .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
                .unwrap_or(preferred)
        })
}

pub fn submit_chat(state: &Arc<RwLock<AppState>>, input: &str, response_index: usize, request_id: u64) -> AbortHandle {
    let state_clone = state.clone();
    let input_clone = input.to_string();
    let (abort_handle, abort_registration) = AbortHandle::new_pair();

    tokio::spawn(async move {
        let mut full_response = String::new();

        // ── Setup: get runtime, provider, model, system prompt ──
        let default_tool_prompt = concat!(
            "You are a helpful assistant with access to tools. ",
            "You can read/write files, execute shell commands, and list directories. ",
            "Always use the appropriate tool when asked. ",
            "Produce a concrete result."
        );
        let (provider, model_id, system_prompt) = {
            let mut s = state_clone.write().await;
            let runtime = match &s.runtime {
                Some(r) => r.clone(),
                None => {
                    if let Some(msg) = s.messages.get_mut(response_index) {
                        msg.content = "Runtime not initialized".to_string();
                        msg.status = MessageStatus::Error;
                    }
                    s.active_chat_requests = 0;
                    return;
                }
            };

            let selected = s.selected_models.first().cloned();
            if let Some(ref sel) = selected {
                let provider_id = sel.provider_id.clone();
                if s.configured_providers.iter().any(|id| id == &provider_id) {
                    if let Ok(client) = get_or_create_provider_client(&mut s, &provider_id) {
                        let mut rt = runtime.write().await;
                        rt.set_provider_from_state(client);
                        rt.set_default_model(&sel.model_id);
                    }
                }
            }
            drop(s);

            let agent_id = match ensure_initial_agent(&state_clone, &input_clone).await {
                Ok(id) => id,
                Err(e) => {
                    let mut s = state_clone.write().await;
                    if let Some(msg) = s.messages.get_mut(response_index) {
                        msg.content = format!("Initial agent unavailable: {}", e);
                        msg.status = MessageStatus::Error;
                    }
                    s.active_chat_requests = 0;
                    return;
                }
            };

            let rt = runtime.read().await;
            let provider = match &rt.provider {
                Some(p) => p.clone(),
                None => {
                    let mut s = state_clone.write().await;
                    if let Some(msg) = s.messages.get_mut(response_index) {
                        msg.content = "No LLM provider configured".to_string();
                        msg.status = MessageStatus::Error;
                    }
                    s.active_chat_requests = 0;
                    return;
                }
            };
            let model_id = rt.model_id.clone();
            let agent_prompt = {
                let s = state_clone.read().await;
                let pool = s.agent_pool.read().await;
                pool.get_agent(&agent_id)
                    .map(|agent| agent.config.system_prompt.clone())
                    .unwrap_or_else(|| default_tool_prompt.to_string())
            };
            let system_prompt = format!(
                "{}\n\nYou are the workflow agent. Chat with the user, clarify the goal, and delegate tasks by calling the `spawn_agent` tool (roles: planner, developer, tester, reviewer, worker, etc.). You are fully responsible for all spawned agents — the human does not approve or manage them.\n\nYou have access to tools: read_file, write_file, sh, list_dir, and spawn_agent. Always use the appropriate tool when asked.",
                agent_prompt
            );
            (provider, model_id, system_prompt)
        };

        // ── Tool-enabled stream ──
        let tool_server = {
            let s = state_clone.read().await;
            s.tool_server.clone()
        };
        // ── Build conversation history from previous messages ──
        // IMPORTANT: Exclude the current turn's messages (user input at response_index-1
        // and empty agent response at response_index) to avoid sending the user
        // message twice (once in history, once as the prompt) and to avoid sending
        // an empty assistant message that confuses the LLM.
        let history = {
            let s = state_clone.read().await;
            let mut hist: Vec<(String, String)> = Vec::new();
            for (i, msg) in s.messages.iter().enumerate() {
                // Skip the current turn's user message (response_index - 1)
                // and the empty agent response (response_index).
                if i >= response_index.saturating_sub(1) {
                    break;
                }
                match msg.role {
                    crate::tui::state::MessageRole::User => {
                        hist.push(("user".to_string(), msg.content.clone()));
                    }
                    crate::tui::state::MessageRole::Agent => {
                        hist.push(("assistant".to_string(), msg.content.clone()));
                    }
                    _ => {}
                }
            }
            hist
        };

        let mut stream = match provider
            .chat_with_tools_stream_mcp(&model_id, &system_prompt, &input_clone, &history, &tool_server)
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let mut s = state_clone.write().await;
                if let Some(msg) = s.messages.get_mut(response_index) {
                    msg.content = format!("Error: {}", e);
                    msg.status = MessageStatus::Error;
                }
                s.active_chat_requests = 0;
                return;
            }
        };

        use futures::future::Abortable;
        let stream_result = Abortable::new(
            async {
                while let Some(event) = stream.next().await {
                    match event {
                        llm::ToolEvent::Text(text) => {
                            full_response.push_str(&text);
                            let mut s = state_clone.write().await;
                            let slot = find_streaming_slot(&s.messages, response_index);
                            if let Some(msg) = s.messages.get_mut(slot) {
                                msg.content = full_response.clone();
                                msg.status = MessageStatus::Streaming;
                            }
                        }
                        llm::ToolEvent::ToolCall { name, args, .. } => {
                            let args_str = match &args {
                                serde_json::Value::Object(map) if map.len() <= 3 => {
                                    // Compact one-line for typical spawn/tool calls
                                    let parts: Vec<String> = map
                                        .iter()
                                        .map(|(k, v)| {
                                            let val = match v {
                                                serde_json::Value::String(s) => {
                                                    if s.len() > 60 {
                                                        format!("\"{}…\"", &s[..57])
                                                    } else {
                                                        format!("\"{}\"", s)
                                                    }
                                                }
                                                other => other.to_string(),
                                            };
                                            format!("{}: {}", k, val)
                                        })
                                        .collect();
                                    parts.join(", ")
                                }
                                other => serde_json::to_string_pretty(other).unwrap_or_else(|_| format!("{:?}", other)),
                            };
                            let tool_msg = format!("🔧 {} — {}", name, args_str);
                            let now = chrono::Local::now().format("%H:%M:%S").to_string();
                            let mut s = state_clone.write().await;
                            s.messages.push(ChatMessage {
                                role: MessageRole::Decision,
                                content: tool_msg,
                                timestamp: now,
                                status: MessageStatus::Completed,
                            });
                        }
                        llm::ToolEvent::Done => break,
                    }
                }
                Ok::<String, anyhow::Error>(full_response)
            },
            abort_registration,
        )
        .await;

        // ── Finalize ──
        {
            let mut s = state_clone.write().await;
            let slot = s
                .messages
                .get(response_index)
                .filter(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
                .map(|_| response_index)
                .or_else(|| {
                    s.messages
                        .iter()
                        .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
                })
                .unwrap_or(response_index);

            let response_text = match &stream_result {
                Ok(Ok(full)) => Some(full.clone()),
                _ => None,
            };

            match stream_result {
                Ok(Ok(full)) => {
                    if let Some(msg) = s.messages.get_mut(slot) {
                        if !full.is_empty() {
                            msg.content = full;
                        }
                        msg.status = MessageStatus::Completed;
                    }
                }
                Ok(Err(e)) => {
                    if let Some(msg) = s.messages.get_mut(slot) {
                        msg.content = format!("Error: {}", e);
                        msg.status = MessageStatus::Error;
                    }
                }
                Err(_) => {
                    // Aborted (Ctrl+X)
                    if let Some(msg) = s.messages.get_mut(slot) {
                        msg.content += " (cancelled)";
                        msg.status = MessageStatus::Completed;
                    }
                }
            }
            if s.active_chat_request_id == request_id {
                s.active_chat_abort = None;
                s.active_chat_requests = 0;
            }

            // Record experience for successful chat responses to seed the pool.
            if let Some(full) = response_text {
                if !full.is_empty() {
                    if let Some(runtime) = &s.runtime {
                        if let Ok(rt) = runtime.try_read() {
                            if let Ok(emb) = rt.embed(&input_clone).await {
                                rt.record_experience(crate::core::types::ExperienceEntry {
                                    embedding: emb,
                                    applicability_vector: [0.0f32; 128],
                                    tool_bitmap: 0,
                                    role_template_id: None,
                                    weight: 0.6,
                                    domain_version: 0,
                                    timestamp: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_secs(),
                                    l2_override_weight: 0.0,
                                    l2_override_created_at: 0,
                                });
                            }
                        }
                    }
                }
            }
        }
    });

    abort_handle
}

// ============================================================================
//  Custom provider management
// ============================================================================

fn provider_env_key(custom_id: &str) -> String {
    format!("CUSTOM_{}_API_KEY", custom_id.to_uppercase())
}

pub fn save_custom_provider(state: &mut AppState, name: &str, url: &str, key: &str, models_str: &str) {
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

    // Persist
    if let Err(e) = crate::persistence::save_custom_provider(&provider) {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Failed to save custom provider: {}", e),
            timestamp: now,
            status: MessageStatus::Error,
        });
        return;
    }

    // Store API key for client creation
    if !key.is_empty() {
        if !state.configured_providers.contains(&custom_id) {
            state.configured_providers.push(custom_id.clone());
        }
        state.api_keys.insert(env_key.clone(), key.to_string());
        // Also persist the key
        let _ = crate::persistence::save_configured_provider(&custom_id, &env_key, key);
    } else {
        // No-auth custom provider
        if !state.configured_providers.contains(&custom_id) {
            state.configured_providers.push(custom_id.clone());
        }
    }

    // Merge into model registry
    state.models.add_custom_provider(&provider);

    // Pre-create provider client
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
        timestamp: now,
        status: MessageStatus::Completed,
    });
}

pub fn remove_custom_provider(state: &Arc<RwLock<AppState>>, name: &str, now: &str) {
    let custom_id = CustomProvider::slug(name);
    let provider_id = format!("custom-{}", custom_id);
    let now = now.to_string();
    let state_clone = state.clone();
    let name = name.to_string();

    tokio::spawn(async move {
        let mut s = state_clone.write().await;

        // Remove from persistence
        if let Err(e) = crate::persistence::remove_custom_provider(&custom_id) {
            s.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("Failed to remove custom provider: {}", e),
                timestamp: now.clone(),
                status: MessageStatus::Error,
            });
            return;
        }

        // Remove from model registry
        s.models.remove_custom_provider(&custom_id);

        // Remove from configured providers and clients
        s.configured_providers.retain(|p| p != &provider_id && p != &custom_id);
        s.provider_clients.remove(&provider_id);

        // Remove API key
        let env_key = provider_env_key(&custom_id);
        s.api_keys.remove(&env_key);

        // Also clean up any selected models from this provider
        s.selected_models.retain(|sm| sm.provider_id != provider_id);

        s.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Custom provider \"{}\" removed", name),
            timestamp: now,
            status: MessageStatus::Completed,
        });
    });
}

pub fn list_custom_providers(state: &Arc<RwLock<AppState>>, now: &str) {
    let state_clone = state.clone();
    let now = now.to_string();

    tokio::spawn(async move {
        let s = state_clone.read().await;
        let custom: Vec<_> = s
            .models
            .providers()
            .iter()
            .filter(|p| p.id.starts_with("custom-"))
            .collect();

        if custom.is_empty() {
            let mut s = state_clone.write().await;
            s.messages.push(ChatMessage {
                role: MessageRole::System,
                content: "No custom providers configured.".to_string(),
                timestamp: now,
                status: MessageStatus::Completed,
            });
            return;
        }

        let mut lines = vec![format!("Custom Providers ({}):", custom.len())];
        for p in &custom {
            let model_count = p.models.len();
            let has_key = if let Some(env) = p.env.first() {
                if s.api_keys.contains_key(env) { "✓" } else { "⌁" }
            } else {
                ""
            };
            let api_url = p.api.as_deref().unwrap_or("-");
            lines.push(format!(
                "  {}  {} ({} model(s)) — {}",
                has_key, p.name, model_count, api_url
            ));
        }

        let mut s = state_clone.write().await;
        s.messages.push(ChatMessage {
            role: MessageRole::System,
            content: lines.join("\n"),
            timestamp: now,
            status: MessageStatus::Completed,
        });
    });
}

/// Load custom providers from persistence and merge them into the model registry.
pub fn merge_custom_providers(state: &mut AppState) {
    let custom_providers = crate::persistence::load_custom_providers();
    if custom_providers.is_empty() {
        return;
    }
    for cp in &custom_providers {
        // Restore API key if we have it
        let env_key = provider_env_key(&cp.id);
        if !cp.api_key.is_empty() {
            state.api_keys.insert(env_key.clone(), cp.api_key.clone());
            if !state.configured_providers.contains(&cp.id) {
                state.configured_providers.push(cp.id.clone());
            }
        }
        // Add to registry
        state.models.add_custom_provider(cp);
    }
    if !custom_providers.is_empty() {
        let count = custom_providers.len();
        state.messages.push(ChatMessage {
            role: MessageRole::System,
            content: format!("Loaded {} custom provider(s)", count),
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            status: MessageStatus::Completed,
        });
    }
}
