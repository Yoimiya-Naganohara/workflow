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

        // ── Setup: get runtime, provider, model ──
        let (provider, model_id) = {
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
            (provider, model_id)
        };

        // ── Tool-enabled stream ──
        let system_prompt = concat!(
            "You are a helpful assistant with access to tools. ",
            "You can read/write files, execute shell commands, and list directories. ",
            "Always use the appropriate tool when asked. ",
            "Produce a concrete result."
        );
        let tool_server = {
            let s = state_clone.read().await;
            s.tool_server.clone()
        };
        let mut stream = match provider
            .chat_with_tools_stream_mcp(&model_id, system_prompt, &input_clone, &[], &tool_server)
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
                        llm::ToolEvent::ToolCall { name, args, result: _ } => {
                            let args_str =
                                serde_json::to_string_pretty(&args).unwrap_or_else(|_| format!("{:?}", args));
                            let tool_msg = format!("🔧 **{}**\n```json\n{}\n```", name, args_str);
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

        match stream_result {
            Ok(Ok(full)) => {
                if let Some(msg) = s.messages.get_mut(slot) {
                    if !full.is_empty() {
                        msg.content = full.clone();
                    }
                    msg.status = MessageStatus::Completed;
                }
                if let Some(mut plan) = crate::agent::plan::Plan::parse_from_response(&full) {
                    plan.goal = input_clone;
                    s.current_plan = Some(plan);
                    s.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Plan detected. Type /apply to approve and execute.".to_string(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
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
    });

    abort_handle
}

// ============================================================================
//  Plan execution
// ============================================================================

pub fn execute_plan(state: &Arc<RwLock<AppState>>) {
    let state_clone = state.clone();
    tokio::spawn(async move {
        // ── Gather tasks and prepare runtime ──
        let (tasks, runtime, agent_pool) = {
            let mut s = state_clone.write().await;

            let tasks: Vec<(usize, String)> = s
                .current_plan
                .as_ref()
                .map(|p| {
                    p.tasks
                        .iter()
                        .filter(|t| t.status == crate::agent::plan::TaskStatus::Pending)
                        .map(|t| (t.id, t.description.clone()))
                        .collect()
                })
                .unwrap_or_default();

            let runtime = match &s.runtime {
                Some(r) => r.clone(),
                None => {
                    s.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: "Runtime not initialized".to_string(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Error,
                    });
                    return;
                }
            };

            let agent_pool = s.agent_pool.clone();

            // Configure provider/model on runtime
            if let Some(ref sel) = s.selected_models.first().cloned() {
                let provider_id = sel.provider_id.clone();
                if s.configured_providers.iter().any(|id| id == &provider_id) {
                    if let Ok(client) = get_or_create_provider_client(&mut s, &provider_id) {
                        let mut rt = runtime.write().await;
                        rt.set_provider_from_state(client);
                        rt.set_default_model(&sel.model_id);
                    }
                }
            }

            (tasks, runtime, agent_pool)
        };

        let mut task_results: Vec<(usize, String)> = Vec::new();

        // ── Execute each task through the experience pipeline ──
        for (task_id, task_desc) in &tasks {
            // Mark running in UI
            {
                let mut s = state_clone.write().await;
                if let Some(p) = &mut s.current_plan {
                    p.mark_task_running(*task_id);
                }
                s.agents.push(AgentEntry {
                    id: format!("task-{:03}", task_id),
                    name: format!("Task {}: {}", task_id, task_desc.chars().take(24).collect::<String>()),
                    status: crate::tui::state::AgentStatus::Running,
                    budget: 0,
                });
            }

            // Step 1: spawn_root_agent through decision pipeline
            // Uses role="planner" template which has system prompt for decomposition
            let agent_id = match runtime
                .read()
                .await
                .spawn_root_agent(task_desc, "planner", &mut *agent_pool.write().await)
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    let mut s = state_clone.write().await;
                    if let Some(p) = &mut s.current_plan {
                        p.mark_task_failed(*task_id, e.to_string());
                    }
                    s.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Task {} rejected: {}", task_id, e),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Error,
                    });
                    continue;
                }
            };

            // Step 2: execute agent tree (recursive @role → spawn_child → aggregate)
            runtime.read().await.execute_agent(agent_id, &agent_pool).await;

            // Step 3: collect result
            let result = runtime.read().await.await_agent(agent_id, &agent_pool).await;

            let result_snippet = result.chars().take(200).collect::<String>();
            task_results.push((*task_id, result.clone()));

            // Mark completed in UI
            {
                let mut s = state_clone.write().await;
                if let Some(p) = &mut s.current_plan {
                    p.mark_task_completed(*task_id, result_snippet.clone());
                }
                if let Some(agent) = s.agents.iter_mut().find(|a| a.id == format!("task-{:03}", task_id)) {
                    agent.status = crate::tui::state::AgentStatus::Completed;
                }
                s.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: format!("✅ Task {} complete: {}", task_id, result_snippet),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }
        }

        // ── Final summary ──
        {
            let mut s = state_clone.write().await;
            let completed = task_results.len();
            let total = tasks.len();
            s.messages.push(ChatMessage {
                role: MessageRole::System,
                content: format!("Plan execution finished: {}/{} tasks completed.", completed, total),
                timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                status: MessageStatus::Completed,
            });
        }
    });
}
