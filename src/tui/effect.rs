//! Effect system — explicit, traceable async operations.
//!
//! The handler never spawns tasks directly.  Instead it pushes
//! [`Effect`] values onto a queue in [`AppState`].  The event loop
//! drains the queue and delegates execution to [`execute_effect`],
//! which is a **stateless** async function — it takes an `Effect`,
//! produces zero or more [`AppEvent`] values, and never touches
//! [`AppState`] directly.
//!
//! Results arrive through an `mpsc` channel and are applied by
//! [`AppState::handle_event`].

use futures::{StreamExt, future::Abortable};
use tokio::sync::mpsc;

use crate::core::types::ExperienceEntry;
use crate::llm::{LlmProvider, ToolEvent};
use crate::models::ModelRegistry;
use crate::persistence;
use crate::runtime::AgentRuntime;
use crate::tools::ToolServerHandle;

// ═══════════════════════════════════════════════════════════════
//  Effect — an async operation requested by the UI
// ═══════════════════════════════════════════════════════════════

pub enum Effect {
    FetchModelRegistry,
    ExecuteShell {
        command: String,
    },
    PoolQuery {
        query_text: String,
        runtime: std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>,
        now: String,
    },
    StartChat {
        input: String,
        response_index: usize,
        request_id: u64,
        model_id: String,
        system_prompt: String,
        history: Vec<(String, String)>,
        tool_server: ToolServerHandle,
        provider: std::sync::Arc<LlmProvider>,
        runtime: Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
        abort_registration: futures::future::AbortRegistration,
    },
    /// Compute embeddings for all roles missing them.
    ComputeRoleEmbeddings,
    /// Run prompt optimization for a role.
    OptimizeRole {
        role_name: String,
        runtime: std::sync::Arc<tokio::sync::RwLock<crate::runtime::AgentRuntime>>,
    },
}

// ═══════════════════════════════════════════════════════════════
//  AppEvent — a completed async operation's result
// ═══════════════════════════════════════════════════════════════

pub enum AppEvent {
    ModelRegistryFetched {
        count: usize,
    },
    ModelRegistryFailed {
        error: String,
        is_empty: bool,
    },
    ShellOutput {
        content: String,
        timestamp: String,
    },
    ShellError {
        error: String,
        timestamp: String,
    },
    PoolQueryResult {
        content: String,
        timestamp: String,
        is_error: bool,
    },
    ChatToken {
        response_index: usize,
        text: String,
    },
    ChatToolCall {
        response_index: usize,
        name: String,
        args: String,
        timestamp: String,
    },
    ChatCompleted {
        response_index: usize,
        request_id: u64,
        full_response: String,
        input: String,
        runtime: Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
    },
    ChatError {
        response_index: usize,
        request_id: u64,
        error: String,
    },
    ChatCancelled {
        response_index: usize,
        request_id: u64,
    },
    /// Result of a role optimization.
    OptimizationResult {
        role_name: String,
        original: String,
        improved: String,
        summary: String,
        stats: crate::runtime::optimizer::OptimizationStats,
    },
    /// Error during role optimization.
    OptimizationError {
        role_name: String,
        error: String,
    },
}

// ═══════════════════════════════════════════════════════════════
//  Executor — stateless, never touches AppState
// ═══════════════════════════════════════════════════════════════

pub async fn execute_effect(effect: Effect, tx: &mpsc::UnboundedSender<AppEvent>) {
    match effect {
        Effect::FetchModelRegistry => {
            let mut registry = ModelRegistry::new();
            match registry.fetch().await {
                Ok(()) => {
                    let count = registry.providers().len();
                    let _ = persistence::save_provider_cache(&registry);
                    let _ = tx.send(AppEvent::ModelRegistryFetched { count });
                }
                Err(e) => {
                    // Preserve any cached data we might have
                    if let Some(cached) = persistence::load_provider_cache() {
                        let _ = persistence::save_provider_cache(&cached);
                    }
                    let _ = tx.send(AppEvent::ModelRegistryFailed {
                        error: e.to_string(),
                        is_empty: registry.providers().is_empty(),
                    });
                }
            }
        }

        Effect::ExecuteShell { command } => {
            let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
            match tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output()
                .await
            {
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
                    let _ = tx.send(AppEvent::ShellOutput { content, timestamp });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::ShellError {
                        error: e.to_string(),
                        timestamp,
                    });
                }
            }
        }

        Effect::PoolQuery {
            query_text,
            runtime,
            now,
        } => {
            let rt = runtime.read().await;
            let (content, is_error) = match rt.embed(&query_text).await {
                Ok(emb) => {
                    let results = rt.search_experience(&emb, 10);
                    if results.is_empty() {
                        ("No matching experiences found.".to_string(), false)
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
                            false,
                        )
                    }
                }
                Err(e) => (format!("Embedding failed: {}", e), true),
            };
            let _ = tx.send(AppEvent::PoolQueryResult {
                content,
                timestamp: now,
                is_error,
            });
        }

        Effect::StartChat {
            input,
            response_index,
            request_id,
            model_id,
            system_prompt,
            history,
            tool_server,
            provider,
            runtime,
            abort_registration,
        } => {
            let mut stream = match provider
                .chat_with_tools_stream_mcp(&model_id, &system_prompt, &input, &history, &tool_server)
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    let _ = tx.send(AppEvent::ChatError {
                        response_index,
                        request_id,
                        error: e.to_string(),
                    });
                    return;
                }
            };

            let mut full_response = String::new();

            let stream_result = Abortable::new(
                async {
                    while let Some(event) = stream.next().await {
                        match event {
                            ToolEvent::Text(text) => {
                                full_response.push_str(&text);
                                let _ = tx.send(AppEvent::ChatToken { response_index, text });
                            }
                            ToolEvent::ToolCall { name, args, .. } => {
                                let args_str = format_tool_args(&args);
                                let timestamp = chrono::Local::now().format("%H:%M:%S").to_string();
                                let _ = tx.send(AppEvent::ChatToolCall {
                                    response_index,
                                    name,
                                    args: args_str,
                                    timestamp,
                                });
                            }
                            ToolEvent::Done => break,
                        }
                    }
                },
                abort_registration,
            )
            .await;

            match stream_result {
                Ok(()) => {
                    let _ = tx.send(AppEvent::ChatCompleted {
                        response_index,
                        request_id,
                        full_response: full_response.clone(),
                        input: input.clone(),
                        runtime: runtime.clone(),
                    });
                }
                Err(_) => {
                    // Cancelled via Ctrl+X
                    let _ = tx.send(AppEvent::ChatCancelled {
                        response_index,
                        request_id,
                    });
                }
            }

            // Record experience in background (fire and forget)
            if !full_response.is_empty() {
                if let Some(rt) = &runtime {
                    if let Ok(rt) = rt.try_read() {
                        if let Ok(emb) = rt.embed(&input).await {
                            // TUI chat lacks agent context — role_template_id is None
                            // (agent-executed experiences set this in runtime.rs)
                            rt.record_experience(ExperienceEntry {
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

        Effect::ComputeRoleEmbeddings => {
            // ComputeRoleEmbeddings is handled directly in the TUI commands
            // via runtime.compute_role_embeddings_async() — no async effect needed.
            tracing::info!("Role embeddings computed via /role embed command");
        }

        Effect::OptimizeRole { role_name, runtime } => {
            // Read role and experiences from runtime
            let (role, experiences, provider, model_id) = {
                let rt = runtime.read().await;
                let role = match rt.get_role_template(&role_name) {
                    Some(r) => r,
                    None => {
                        let _ = tx.send(AppEvent::OptimizationError {
                            role_name: role_name.clone(),
                            error: format!("Role '{}' not found", role_name),
                        });
                        return;
                    }
                };
                let experiences = rt.get_experiences_by_role(role.template_id);
                let provider = match &rt.provider {
                    Some(p) => p.clone(),
                    None => {
                        let _ = tx.send(AppEvent::OptimizationError {
                            role_name: role_name.clone(),
                            error: "No LLM provider configured".to_string(),
                        });
                        return;
                    }
                };
                (role, experiences, provider, rt.model_id.clone())
            };

            match crate::runtime::optimizer::optimize_role(&role, &experiences, &provider, &model_id).await {
                Ok(result) => {
                    let _ = tx.send(AppEvent::OptimizationResult {
                        role_name: result.role_name.clone(),
                        original: result.original_prompt,
                        improved: result.improved_prompt,
                        summary: result.summary,
                        stats: result.stats,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::OptimizationError {
                        role_name: role_name.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Helpers
// ═══════════════════════════════════════════════════════════════

fn format_tool_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) if map.len() <= 3 => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| {
                    let val = match v {
                        serde_json::Value::String(s) => {
                            if s.len() > 60 {
                                // Safe char-boundary truncation at 57 chars.
                                let end = s.char_indices().nth(57).map(|(i, _)| i).unwrap_or(s.len());
                                format!("\"{}…\"", &s[..end])
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
    }
}
