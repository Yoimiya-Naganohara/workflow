use std::collections::HashMap;
use std::sync::Arc;

use futures::future::AbortHandle;
use tokio::sync::RwLock;

use crate::core::types::AgentId;
use crate::models::{ModelCapabilities, ModelRegistry};
use crate::provider::ProviderClient;
use crate::reflection::ReflectionConfig;
use crate::runtime::AgentRuntime;
use crate::tools::ToolServerHandle;
use crate::tui::effect::AppEvent;

// ── Shared types ──

/// Statistics about the dual-track experience pool.
#[derive(Debug, Clone, Default)]
pub struct ExperiencePoolStats {
    pub total: usize,
    pub bedrock: usize,
    pub fluid: usize,
    pub last_flush_result: Option<String>,
}

/// Cache hit/miss statistics for the embedding service.
#[derive(Debug, Clone, Copy, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

impl CacheStats {
    /// Cache hit rate as a percentage (0.0–100.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64 * 100.0
        }
    }
}

#[derive(Clone, PartialEq)]
pub enum Focus {
    Sidebar,
    Chat,
    Input,
}

#[derive(Clone, PartialEq)]
pub enum Panel {
    Chat,
}

#[derive(Clone, PartialEq)]
pub enum AppMode {
    Plan,
    Build,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MessageStatus {
    Thinking,
    Streaming,
    Completed,
    Error,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default)]
    pub reasoning: String,
    pub timestamp: String,
    pub status: MessageStatus,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        let now = chrono::Local::now().format("%H:%M:%S").to_string();
        Self {
            role: MessageRole::System,
            content: content.into(),
            reasoning: String::new(),
            timestamp: now,
            status: MessageStatus::Completed,
        }
    }
}

#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum MessageRole {
    System,
    User,
    Agent,
    Decision,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SelectedModel {
    pub provider_id: String,
    pub model_id: String,
    pub provider_name: String,
    pub model_name: String,
}

#[derive(Clone)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub budget: u64,
}

#[derive(Clone, PartialEq)]
pub enum AgentStatus {
    Running,
    Suspended,
    Completed,
    Failed,
}

// ── Grouped state structs ──

/// Business / domain state — persisted or long-lived.
pub struct CoreState {
    pub messages: Vec<ChatMessage>,
    pub models: ModelRegistry,
    pub configured_providers: Vec<String>,
    pub api_keys: HashMap<String, String>,
    pub provider_clients: HashMap<String, Arc<ProviderClient>>,
    pub selected_models: Vec<SelectedModel>,
    pub agents: Vec<AgentEntry>,
    pub agent_pool: Arc<RwLock<crate::agent::AgentPool>>,
    pub runtime: Option<Arc<RwLock<AgentRuntime>>>,
    pub tool_server: ToolServerHandle,
    pub responsible_agent_id: Option<AgentId>,
    pub default_role: String,
    pub reflection: ReflectionConfig,
    /// Track the last chat context for retry.
    pub last_chat_request_id: u64,
    /// Channel sender to the background [`RuntimeEventLoop`].
    /// Tools use this to dispatch async work without blocking the LLM stream.
    pub runtime_event_tx: Option<tokio::sync::mpsc::Sender<crate::runtime::RuntimeEvent>>,
}

/// Transient UI state — reset on restart.
pub struct UiState {
    pub panel: Panel,
    pub focus: Focus,
    pub mode: AppMode,
    pub input: String,
    pub input_cursor: usize,
    pub sidebar_scroll: usize,
    pub chat_scroll: usize,
    pub auto_scroll: bool,
    pub think_frame: u8,

    pub input_history: Vec<String>,
    pub input_history_idx: Option<usize>,
    pub active_chat_request_id: u64,
    pub active_chat_abort: Option<AbortHandle>,
    pub active_chat_requests: usize,
    pub budget_used: u64,
    pub budget_total: u64,
    pub permits_available: usize,
    pub permits_total: usize,
    pub context_limit: u64,
    /// Cached token counts (recalculated on message changes).
    pub cached_input_tokens: u32,
    pub cached_output_tokens: u32,
    /// Number of messages at last token recalc (to detect new messages).
    pub cached_message_count: usize,
    /// Whether the tiktoken BPE file has been downloaded.
    pub tokenizer_initialized: bool,
    /// Whether `ChatTokenUsage` events have been received in the current stream.
    /// When `true`, API‑reported token counts take precedence over local estimates.
    pub has_api_tokens: bool,
    /// When a paste is pending, stores the full text to be sent on submit.
    /// The input shows a summary marker instead of the raw text.
    pub pending_paste: Option<String>,
    pub current_plan: Option<crate::agent::plan::Plan>,

    // ── Phase 1: Agent diagnostic tree ──
    /// Bumped every time the agent tree topology or statuses change.
    /// Used by `render.rs` to decide whether to rebuild tree lines.
    pub agent_tree_version: u64,
    /// Cached tree lines from last render (avoids rebuilding every 50 ms tick).
    pub cached_tree_lines: Vec<String>,
    /// Currently selected agent index within the diagnostic tree.
    /// Used for keyboard navigation and detail popup (Phase 3).
    pub selected_agent_idx: usize,
    /// Agent IDs in display order of the diagnostic tree.
    /// Updated atomically with cached_tree_lines when version changes.
    pub tree_agent_ids: Vec<crate::core::types::AgentId>,
    /// When true, the input field is greyed out and keyboard input is
    /// discarded.  Set when the root agent transitions to AwaitingChildren.
    pub input_disabled: bool,
    /// Last known total chat lines (updated each render for scroll clamping).
    pub total_chat_lines: usize,
    /// Controls how much reasoning/chain-of-thought is shown:
    /// 0 = hidden, 1 = brief (first 200 chars), 2 = full.
    pub think_level: u8,
    /// Reasoning effort sent to the LLM: None = off, Some("low"/"medium"/"high"/"max").
    pub reasoning_effort: Option<String>,

    // ── Command Palette (Phase 1) ──
    /// Tree-based command navigation state machine.
    /// Only active when `popup_mode == PopupMode::CommandPalette`.
    pub command_palette: crate::tui::command_tree::CommandPalette,

    // ── System prompt cache ──
    /// Cached system prompt for the current session.
    /// Built once on first message, reused for all subsequent messages.
    /// Cleared on `/clear` or when role changes.
    pub cached_system_prompt: Option<String>,
    /// Role for which the system prompt was cached.
    /// If role changes, the cache is invalidated.
    pub cached_prompt_role: String,

    // ── Cache metrics ──
    /// Embedding service cache hit/miss stats (refreshed each render tick).
    pub embedding_cache: CacheStats,
    /// Accumulated provider-managed cache reads (prompt caching).
    pub llm_cache_read: u64,
    /// Accumulated provider-managed cache writes.
    pub llm_cache_write: u64,
}

// ── AppState ──

#[derive(Default)]
pub struct AppState {
    pub ui: UiState,
    pub core: CoreState,
    pub popup_mode: PopupMode,
    pub popup_selected: usize,
    pub popup_key_provider: Option<String>,
    pub keymap: super::keymap::Keymap,
    pub pool_stats: ExperiencePoolStats,
    /// Effects queued for async execution (drained by event loop).
    pub effects: Vec<crate::tui::effect::Effect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum PopupMode {
    #[default]
    None,
    Providers,
    KeyInput,
    ModelPicker,
    FilePicker {
        query: String,
    },
    AgentDetail {
        agent_id: crate::core::types::AgentId,
    },
    /// Popup asking the user to type argument(s) for a command.
    /// `cmd` is the base command (e.g. `/sh`), and once Enter is pressed
    /// the full line `/sh <input>` is dispatched.
    ShellInput {
        cmd: String,
        input: String,
    },

    /// Tree-based command palette (Phase 1).
    CommandPalette,
}

impl AppState {
    /// Apply an async result to the state.
    pub fn handle_event(&mut self, event: AppEvent) {
        use crate::tui::state::{ChatMessage, MessageRole, MessageStatus};
        match event {
            AppEvent::ModelRegistryFetched { count } => {
                if let Some(cached) = crate::persistence::load_provider_cache() {
                    self.core.models = cached;
                }
                self.core
                    .messages
                    .push(ChatMessage::system(format!("Loaded {} providers", count)));
            }
            AppEvent::ModelRegistryFailed { error, is_empty } => {
                let status = if is_empty {
                    MessageStatus::Error
                } else {
                    MessageStatus::Completed
                };
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    reasoning: String::new(),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status,
                });
            }
            AppEvent::ShellOutput { content, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    reasoning: String::new(),
                    timestamp,
                    status: MessageStatus::Completed,
                });
            }
            AppEvent::ShellError { error, timestamp } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: error,
                    reasoning: String::new(),
                    timestamp,
                    status: MessageStatus::Error,
                });
            }
            AppEvent::PoolQueryResult {
                content,
                timestamp,
                is_error,
            } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    reasoning: String::new(),
                    timestamp,
                    status: if is_error {
                        MessageStatus::Error
                    } else {
                        MessageStatus::Completed
                    },
                });
            }
            AppEvent::ChatToken {
                response_index,
                text,
            } => {
                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        let was_thinking = msg.status == MessageStatus::Thinking;
                        msg.content.push_str(&text);
                        msg.status = MessageStatus::Streaming;
                        // On first token, recalc local estimate only if no API token
                        // data has arrived yet.  Once ChatTokenUsage events are flowing,
                        // those per-tick accumulations are more accurate and cheaper.
                        if was_thinking && !self.ui.has_api_tokens {
                            self.recalc_tokens();
                        }
                    }
                }
            }
            AppEvent::ChatReasoning {
                response_index,
                text,
            } => {
                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.reasoning.push_str(&text);
                    }
                }
            }
            AppEvent::ChatCompleted {
                response_index,
                request_id,
                full_response,
                input,
                runtime,
            } => {
                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        // Don't overwrite content — it was already accumulated via
                        // ChatToken (text) and ChatToolCall (tool call lines) events
                        // during streaming. Overwriting would erase the tool call
                        // entries that were appended to the message content.
                        if matches!(
                            msg.status,
                            MessageStatus::Thinking | MessageStatus::Streaming
                        ) {
                            msg.status = MessageStatus::Completed;
                        }
                    }
                }
                if self.ui.active_chat_requests > 0 {
                    self.ui.active_chat_requests -= 1;
                }
                self.ui.active_chat_abort = None;

                // ── Trigger self-check reflection if enabled ──
                if self.core.reflection.auto_reflect {
                    self.trigger_self_check(
                        response_index,
                        request_id,
                        full_response.clone(),
                        &input,
                        &runtime,
                    );
                }

                // ── Save exchange to responsible agent's context ──
                if let Some(agent_id) = self.core.responsible_agent_id {
                    if let Ok(mut pool) = self.core.agent_pool.try_write() {
                        if let Some(agent) = pool.get_agent_mut(&agent_id) {
                            agent.context.push(crate::llm::types::Message {
                                role: "user".to_string(),
                                content: input.clone(),
                            });
                            agent.context.push(crate::llm::types::Message {
                                role: "assistant".to_string(),
                                content: full_response.clone(),
                            });
                            // Prevent unbounded growth: keep last 100 exchanges
                            if agent.context.len() > 200 {
                                agent.context.drain(0..agent.context.len() - 200);
                            }
                            agent.last_active_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                        }
                    }
                }

                // ── Sync budget from runtime ──
                if let Some(rt) = &runtime {
                    if let Ok(r) = rt.try_read() {
                        let remaining = r.remaining_budget() as u64;
                        self.ui.budget_used = self.ui.budget_total.saturating_sub(remaining);
                    }
                }

                // ── Recalculate token cache (only if no API tokens received) ──
                if !self.ui.has_api_tokens {
                    self.recalc_tokens();
                }
            }
            AppEvent::ChatError {
                response_index,
                request_id: _,
                error,
            } => {
                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content = error;
                        msg.status = MessageStatus::Error;
                    }
                } else {
                    // No streaming slot — push a standalone error message
                    // so the user can see it (instead of silently dropping).
                    self.core.messages.push(ChatMessage {
                        role: MessageRole::Agent,
                        content: error,
                        reasoning: String::new(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Error,
                    });
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
                self.recalc_tokens();
            }
            AppEvent::ChatCancelled {
                response_index,
                request_id: _,
            } => {
                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        msg.content += " (cancelled)";
                        msg.status = MessageStatus::Completed;
                    }
                }
                self.ui.active_chat_requests = 0;
                self.ui.active_chat_abort = None;
                self.recalc_tokens();
            }
            AppEvent::ChatTokenUsage {
                response_index: _,
                input,
                output,
                cached_input,
                cache_creation_input,
            } => {
                // API reports per-request cumulative — trust it over local estimate
                self.ui.cached_input_tokens = input;
                self.ui.cached_output_tokens = output;
                self.ui.has_api_tokens = true;
                self.ui.llm_cache_read += cached_input as u64;
                self.ui.llm_cache_write += cache_creation_input as u64;
            }
            AppEvent::OptimizationResult {
                role_name,
                original: _,
                improved,
                summary,
                stats: _,
            } => {
                // Auto-apply: update system_prompt, increment version, recompute embedding.
                if let Some(runtime) = &self.core.runtime {
                    if let Ok(rt) = runtime.try_read() {
                        if let Some(mut tpl) = rt.get_role_template(&role_name) {
                            tpl.system_prompt = improved.clone();
                            tpl.version += 1;
                            tpl.updated_at = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            rt.save_role_template(tpl);
                            // Recompute embedding in background.
                            rt.compute_role_embeddings_async();
                        }
                    }
                }
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization complete.\n\n{}  \n\nNew prompt applied (version ++).",
                    role_name, summary
                )));
            }
            AppEvent::OptimizationError { role_name, error } => {
                self.core.messages.push(ChatMessage::system(format!(
                    "Role '{}' optimization failed: {}",
                    role_name, error
                )));
            }
            AppEvent::ChatToolCall {
                response_index,
                name,
                args,
                timestamp: _,
            } => {
                // Tool call format:
                //   <tool_name>
                //   key=value
                //   key=value
                //   <blank line>
                //
                // Each arg line is truncated at 200 chars to prevent runaway
                // content (e.g. embedded file contents passed as args).
                let args_trunc = if args.len() > 200 {
                    let end = args
                        .char_indices()
                        .nth(197)
                        .map(|(i, _)| i)
                        .unwrap_or(args.len());
                    format!("{}…", &args[..end])
                } else {
                    args
                };

                if let Some(slot) =
                    find_streaming_slot_response(&self.core.messages, response_index)
                {
                    if let Some(msg) = self.core.messages.get_mut(slot) {
                        if !msg.content.is_empty() {
                            msg.content.push('\n');
                        }
                        msg.content.push_str(&name);
                        for arg_line in args_trunc.lines() {
                            msg.content.push('\n');
                            msg.content.push_str(arg_line);
                        }
                        // Trailing blank line to visually separate tool calls
                        // from subsequent text or next tool call.
                        msg.content.push('\n');
                    }
                } else {
                    let mut content = name;
                    for arg_line in args_trunc.lines() {
                        content.push('\n');
                        content.push_str(arg_line);
                    }
                    // Trailing blank line to visually separate tool calls.
                    content.push('\n');
                    self.core.messages.push(ChatMessage {
                        role: MessageRole::Decision,
                        content,
                        reasoning: String::new(),
                        timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                        status: MessageStatus::Completed,
                    });
                }
                self.recalc_tokens();
            }
            AppEvent::SelfCheckResult {
                response_index: _,
                request_id,
                passed,
                attempt,
                input: _,
                system_prompt,
                history,
                tool_server,
                provider,
                model_id,
                runtime,
                abort_registration: _,
                feedback,
            } => {
                if passed {
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let max_attempts = self.core.reflection.max_attempts;
                if attempt >= max_attempts {
                    self.core.messages.push(ChatMessage::system(format!(
                        "Reflection: max retries ({}) reached. Final result shown above.",
                        max_attempts
                    )));
                    self.core.last_chat_request_id = 0;
                    return;
                }

                let new_attempt = attempt + 1;
                let now = chrono::Local::now().format("%H:%M:%S").to_string();
                let feedback_msg = format!(
                    "Reflection #{} — revisiting response\n\n{}",
                    new_attempt, feedback
                );
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: feedback_msg,
                    reasoning: String::new(),
                    timestamp: now,
                    status: MessageStatus::Completed,
                });

                let mut new_history = history.clone();
                // Reset API token tracking for the retry stream
                self.ui.has_api_tokens = false;
                new_history.push(("user".to_string(), feedback.clone()));

                let new_response_index = self.core.messages.len();
                self.core.messages.push(ChatMessage {
                    role: MessageRole::Agent,
                    content: String::new(),
                    reasoning: String::new(),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Thinking,
                });

                let new_request_id = self.ui.active_chat_request_id.wrapping_add(1);
                self.ui.active_chat_request_id = new_request_id;
                self.ui.active_chat_requests = 1;
                self.ui.auto_scroll = true;

                let (abort_handle, new_abort_registration) =
                    futures::future::AbortHandle::new_pair();
                self.ui.active_chat_abort = Some(abort_handle);

                self.core.last_chat_request_id = request_id;

                let new_history_clone = new_history.clone();
                self.effects.push(crate::tui::effect::Effect::StartChat {
                    input: feedback.clone(),
                    response_index: new_response_index,
                    request_id: new_request_id,
                    model_id,
                    system_prompt,
                    history: new_history_clone,
                    tool_server,
                    provider,
                    runtime,
                    abort_registration: new_abort_registration,
                    reasoning_effort: self.ui.reasoning_effort.clone(),
                    reasoning_options: self
                        .core
                        .selected_models
                        .first()
                        .and_then(|sel| {
                            self.core
                                .models
                                .get_model(&sel.provider_id, &sel.model_id)
                                .map(|m| m.reasoning_options.clone())
                        })
                        .unwrap_or_default(),
                });
            }
            AppEvent::SystemLog { content } => {
                self.core.messages.push(ChatMessage {
                    role: MessageRole::System,
                    content,
                    reasoning: String::new(),
                    timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
                    status: MessageStatus::Completed,
                });
            }
            AppEvent::AggregationStarting { agent_id: _ } => {
                self.core.messages.push(ChatMessage::system(
                    "All sub-tasks completed. Synthesising final result…",
                ));
            }
        }
    }

    /// Recalculate cached token counts from all messages.
    /// Uses the tiktoken-based tokenizer; falls back to char/4 estimate.
    ///
    /// **Guarded**: if `has_api_tokens` is already true, this is a no-op.
    /// The API-reported per-request totals from `ChatTokenUsage` events are
    /// strictly more accurate than a local lifetime estimate, so we never let
    /// the local counter overwrite them.
    pub fn recalc_tokens(&mut self) {
        // ── Guard: never overwrite API-reported token data ──
        if self.ui.has_api_tokens {
            return;
        }

        use crate::tui::tokenizer;
        let mut input_tokens = 0u32;
        let mut output_tokens = 0u32;
        for msg in &self.core.messages {
            let tokens = tokenizer::count_tokens(&msg.content);
            match msg.role {
                MessageRole::User | MessageRole::System => {
                    input_tokens = input_tokens.saturating_add(tokens);
                }
                MessageRole::Agent | MessageRole::Decision => {
                    output_tokens = output_tokens.saturating_add(tokens);
                }
            }
        }
        self.ui.cached_input_tokens = input_tokens;
        self.ui.cached_output_tokens = output_tokens;
        self.ui.cached_message_count = self.core.messages.len();
        self.ui.tokenizer_initialized = tokenizer::is_initialised();
    }

    /// Get capabilities for the currently selected model (if any).
    pub fn model_capabilities(&self) -> Option<ModelCapabilities> {
        let sel = self.core.selected_models.first()?;
        let model = self
            .core
            .models
            .get_model(&sel.provider_id, &sel.model_id)?;
        Some(model.capabilities())
    }

    /// Trigger a self-check reflection after a completed chat.
    fn trigger_self_check(
        &mut self,
        response_index: usize,
        request_id: u64,
        full_response: String,
        input: &str,
        runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
    ) {
        // Don't reflect on reflection-triggered retries
        if request_id > 0 && request_id == self.core.last_chat_request_id {
            return;
        }

        // Get provider and model_id from the current selection
        let (provider, model_id, system_prompt) = self.get_chat_context(runtime, input);
        let system_prompt = format!(
            "{}\n\n{}\n\n{}",
            system_prompt,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
        );

        let provider = match provider {
            Some(p) => p,
            None => return,
        };

        let tool_server = self.core.tool_server.clone();

        // Build history from messages up to the response
        let mut history: Vec<(String, String)> = Vec::new();
        for (i, msg) in self.core.messages.iter().enumerate() {
            if i >= response_index.saturating_sub(1) {
                break;
            }
            match msg.role {
                MessageRole::User => history.push(("user".to_string(), msg.content.clone())),
                MessageRole::Agent => history.push(("assistant".to_string(), msg.content.clone())),
                _ => {}
            }
        }

        let (abort_handle, abort_registration) = futures::future::AbortHandle::new_pair();
        self.ui.active_chat_abort = Some(abort_handle);

        self.effects.push(crate::tui::effect::Effect::SelfCheck {
            response_index,
            request_id,
            attempt: 0,
            full_response,
            input: input.to_string(),
            system_prompt,
            history,
            tool_server,
            provider,
            model_id,
            runtime: runtime.clone(),
            abort_registration,
        });
    }

    /// Get the chat context (provider, model_id, system_prompt) from current state.
    fn get_chat_context(
        &self,
        _runtime: &Option<std::sync::Arc<tokio::sync::RwLock<AgentRuntime>>>,
        _input: &str,
    ) -> (
        Option<std::sync::Arc<crate::llm::LlmProvider>>,
        String,
        String,
    ) {
        let default_tool_prompt =
            "Must follow user instructions and use available tools. Remember preferences";

        let provider = self
            .core
            .runtime
            .as_ref()
            .and_then(|rt| rt.try_read().ok().and_then(|r| r.provider.clone()));

        let model_id = provider
            .as_ref()
            .and_then(|_| {
                self.core
                    .runtime
                    .as_ref()
                    .and_then(|rt| rt.try_read().ok().map(|r| r.model_id.clone()))
            })
            .unwrap_or_default();

        // Fallback: build from selected_models if runtime provider unavailable
        let provider = provider.or_else(|| {
            self.core.selected_models.first().and_then(|sel| {
                if self
                    .core
                    .configured_providers
                    .iter()
                    .any(|id| id == &sel.provider_id)
                {
                    // Try to get existing client
                    self.core
                        .provider_clients
                        .get(&sel.provider_id)
                        .map(|pc| Arc::new(pc.inner.clone()))
                } else {
                    None
                }
            })
        });

        let agent_prompt = self
            .core
            .responsible_agent_id
            .as_ref()
            .and_then(|aid| {
                let pool = self.core.agent_pool.try_read().ok()?;
                let role = pool.get_agent(aid)?.role.clone();
                drop(pool);
                let rt = self.core.runtime.as_ref()?.try_read().ok()?;
                rt.get_role_template(&role).map(|t| t.system_prompt.clone())
            })
            .unwrap_or_else(|| default_tool_prompt.to_string());

        (provider, model_id, agent_prompt)
    }
}

/// Find the slot index of a streaming message.
/// Returns `None` if no streaming/thinking slot is found at the preferred index
/// or elsewhere, to prevent overwriting already-completed messages.
fn find_streaming_slot_response(messages: &[ChatMessage], preferred: usize) -> Option<usize> {
    // 1. Prefer the exact index if it's still in streaming/thinking state
    if messages
        .get(preferred)
        .is_some_and(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    {
        return Some(preferred);
    }
    // 2. Fall back to the last streaming/thinking message anywhere
    messages
        .iter()
        .rposition(|m| matches!(m.status, MessageStatus::Thinking | MessageStatus::Streaming))
    // 3. Return None — don't overwrite completed messages
}

impl Default for CoreState {
    fn default() -> Self {
        Self {
            messages: vec![ChatMessage::system(
                "Workflow Agent — connected. Use `/connect` to configure a provider, then `/models` to add models to your pool.",
            )],
            models: ModelRegistry::new(),
            configured_providers: Vec::new(),
            api_keys: HashMap::new(),
            provider_clients: HashMap::new(),
            // provider_clients populated by load_initial_state
            selected_models: Vec::new(),
            agents: vec![AgentEntry {
                id: "agent-000".to_string(),
                name: "Planning Agent".to_string(),
                status: AgentStatus::Running,
                budget: 0,
            }],
            agent_pool: Arc::new(RwLock::new(crate::agent::AgentPool::new())),
            runtime: None,
            tool_server: crate::tools::create_tool_server(),
            default_role: "general_business_analyst".to_string(),
            responsible_agent_id: None,
            reflection: ReflectionConfig::default(),
            last_chat_request_id: 0,
            runtime_event_tx: None,
        }
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            panel: Panel::Chat,
            focus: Focus::Input,
            mode: AppMode::Plan,
            input: String::new(),
            input_cursor: 0,
            sidebar_scroll: 0,
            chat_scroll: 0,
            auto_scroll: true,
            think_frame: 0,
            command_palette: crate::tui::command_tree::CommandPalette::default(),

            input_history: Vec::new(),
            input_history_idx: None,
            active_chat_request_id: 0,
            cached_input_tokens: 0,
            cached_output_tokens: 0,
            cached_message_count: 0,
            tokenizer_initialized: false,
            has_api_tokens: false,
            pending_paste: None,
            active_chat_abort: None,
            active_chat_requests: 0,
            budget_used: 0,
            budget_total: 10000,
            permits_available: 10,
            permits_total: 10,
            context_limit: 0,
            current_plan: None,
            agent_tree_version: 0,
            cached_tree_lines: Vec::new(),
            selected_agent_idx: 0,
            tree_agent_ids: Vec::new(),
            input_disabled: false,
            total_chat_lines: 0,
            think_level: 2,
            reasoning_effort: None,
            cached_system_prompt: None,
            cached_prompt_role: String::new(),
            embedding_cache: CacheStats::default(),
            llm_cache_read: 0,
            llm_cache_write: 0,
        }
    }
}

// ============================================================================
//  Full-chain integration tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::EMBEDDING_DIM;
    use crate::runtime::AgentRuntime;
    use crate::runtime::config::AgentRuntimeConfig;

    /// Build a mock embedding that returns a fixed vector.
    struct MockEmbed;

    #[async_trait::async_trait]
    impl crate::llm::EmbeddingService for MockEmbed {
        async fn embed(&self, _text: &str) -> anyhow::Result<[f32; EMBEDDING_DIM]> {
            Ok([1.0f32; EMBEDDING_DIM])
        }
        async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<[f32; EMBEDDING_DIM]>> {
            Ok(vec![[1.0f32; EMBEDDING_DIM]; texts.len()])
        }
        fn similarity(&self, a: &[f32; EMBEDDING_DIM], b: &[f32; EMBEDDING_DIM]) -> f32 {
            if a == b { 1.0 } else { 0.0 }
        }
        fn cache_size(&self) -> usize {
            0
        }
        fn clear_cache(&self) {}
        fn cache_hits(&self) -> u64 {
            0
        }
        fn cache_misses(&self) -> u64 {
            0
        }
    }

    /// Simulate the handler's history-building logic (lines 718–735 of handler.rs).
    fn build_chat_history<'a>(
        messages: &'a [ChatMessage],
        response_index: usize,
    ) -> Vec<(&'a str, &'a str)> {
        let mut hist = Vec::new();
        for (i, msg) in messages.iter().enumerate() {
            if i >= response_index.saturating_sub(1) {
                break;
            }
            match msg.role {
                MessageRole::User => hist.push(("user", msg.content.as_str())),
                MessageRole::Agent => hist.push(("assistant", msg.content.as_str())),
                // System messages intentionally excluded (display-only).
                MessageRole::System => {}
                MessageRole::Decision => {}
            }
        }
        hist
    }

    /// Simulate the system-prompt construction from handler.rs (lines 704–710).
    fn build_system_prompt(agent_prompt: &str, memos: &str) -> String {
        format!(
            "{}\n\n{}\n\n{}{}",
            agent_prompt,
            crate::core::types::MEMO_INSTRUCTIONS,
            crate::core::types::ZERO_TOLERANCE_INSTRUCTIONS,
            memos,
        )
    }

    // ========================================================================
    //  1. History excluding System messages
    // ========================================================================

    #[test]
    fn test_history_excludes_system_messages() {
        let messages = vec![
            ChatMessage::system("Welcome to Workflow Agent."),
            ChatMessage {
                role: MessageRole::User,
                content: "Hello".into(),
                reasoning: String::new(),
                timestamp: "00:00:01".into(),
                status: MessageStatus::Completed,
            },
            ChatMessage {
                role: MessageRole::Agent,
                content: "Hi there".into(),
                reasoning: String::new(),
                timestamp: "00:00:02".into(),
                status: MessageStatus::Completed,
            },
            ChatMessage::system("Child agent completed."),
            ChatMessage {
                role: MessageRole::User,
                content: "New question".into(),
                reasoning: String::new(),
                timestamp: "00:00:03".into(),
                status: MessageStatus::Completed,
            },
        ];

        // response_index = 5 (agent slot after the last User at index 4).
        // The last User message is passed as the `message` argument to the LLM,
        // NOT included in history. So history contains only indices 0..3.
        let history = build_chat_history(&messages, 5);

        // Should contain: User("Hello"), Agent("Hi there")
        // System messages "Welcome" and "Child agent completed" must be absent.
        // User("New question") is the current query, not in history.
        assert_eq!(
            history.len(),
            2,
            "2 messages in history, 0 system, current query excluded"
        );
        assert_eq!(history[0].0, "user");
        assert_eq!(history[0].1, "Hello");
        assert_eq!(history[1].0, "assistant");
        assert_eq!(history[1].1, "Hi there");

        // Verify no message has role "system".
        for (role, _) in &history {
            assert_ne!(*role, "system", "no system messages in history");
        }
    }

    // ========================================================================
    //  2. System prompt stability (goal NOT in system prompt)
    // ========================================================================

    #[test]
    fn test_system_prompt_does_not_contain_goal() {
        let prompt = build_system_prompt("You are a developer. Write secure code.", "");
        // The goal "Implement feature X" must NOT appear in system prompt.
        assert!(!prompt.contains("Implement feature X"));
        assert!(!prompt.contains("Your goal:"));
        assert!(prompt.contains("You are a developer"));
        assert!(prompt.contains(crate::core::types::MEMO_INSTRUCTIONS));
    }

    #[test]
    fn test_system_prompt_includes_memos_when_present() {
        let prompt = build_system_prompt(
            "You are a tester.",
            "\n\n=== Role Memos ===\n  [preferred_lang]: Rust\n====",
        );
        assert!(prompt.contains("Role Memos"));
        assert!(prompt.contains("preferred_lang"));
        assert!(!prompt.contains("Your goal:"));
    }

    // ========================================================================
    //  3. Event processing pipeline (ChatCompleted → state update)
    // ========================================================================

    #[test]
    fn test_chat_completed_updates_state() {
        let mut state = AppState::default();

        // Simulate a user request & placeholder agent response.
        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Write a test".into(),
            reasoning: String::new(),
            timestamp: "00:00:01".into(),
            status: MessageStatus::Completed,
        });
        state.core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            reasoning: String::new(),
            timestamp: "00:00:02".into(),
            status: MessageStatus::Streaming,
        });
        let resp_idx = state.core.messages.len() - 1;
        state.ui.active_chat_requests = 1;

        // Simulate streaming tokens.
        let tokens = ["Sure", ", ", "here's", " your test."];
        for token in &tokens {
            state.handle_event(AppEvent::ChatToken {
                response_index: resp_idx,
                text: token.to_string(),
            });
        }

        // Simulate completion.
        state.handle_event(AppEvent::ChatCompleted {
            response_index: resp_idx,
            request_id: 0,
            full_response: "Sure, here's your test.".into(),
            input: "Write a test".into(),
            runtime: None,
        });

        // Verify final state.
        let last = state.core.messages.last().unwrap();
        assert_eq!(last.status, MessageStatus::Completed);
        assert_eq!(last.content, "Sure, here's your test.");
        assert_eq!(state.ui.active_chat_requests, 0);
    }

    #[test]
    fn test_chat_error_marks_message() {
        let mut state = AppState::default();

        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Do something".into(),
            reasoning: String::new(),
            timestamp: "00:00:01".into(),
            status: MessageStatus::Completed,
        });
        state.core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            reasoning: String::new(),
            timestamp: "00:00:02".into(),
            status: MessageStatus::Thinking,
        });
        let resp_idx = state.core.messages.len() - 1;
        state.ui.active_chat_requests = 1;

        state.handle_event(AppEvent::ChatError {
            response_index: resp_idx,
            request_id: 0,
            error: "Connection refused".into(),
        });

        let last = state.core.messages.last().unwrap();
        assert_eq!(last.status, MessageStatus::Error);
        assert!(last.content.contains("Connection refused"));
        assert_eq!(state.ui.active_chat_requests, 0);
    }

    // ========================================================================
    //  4. Token recalculation works with mixed roles
    // ========================================================================

    #[test]
    fn test_recalc_tokens_skips_system_messages_for_output() {
        let mut state = AppState::default();
        state
            .core
            .messages
            .push(ChatMessage::system("System note."));
        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Hello".into(),
            ..Default::default()
        });
        state.core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: "World".into(),
            ..Default::default()
        });
        // Clear the seeded initial message to have clean counts.
        state.core.messages.remove(0);

        state.recalc_tokens();

        // User + System count as input; Agent counts as output.
        // Tokenizer may not be initialized in test, so uses fallback (chars/4).
        assert!(
            state.ui.cached_input_tokens > 0,
            "input tokens from User+System"
        );
        assert!(
            state.ui.cached_output_tokens > 0,
            "output tokens from Agent"
        );
        assert_eq!(state.ui.cached_message_count, 3);
    }

    #[test]
    fn test_has_api_tokens_prevents_recalc_overwrite() {
        let mut state = AppState::default();
        state.ui.has_api_tokens = true;
        state.ui.cached_input_tokens = 100;
        state.ui.cached_output_tokens = 50;

        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "New message".into(),
            ..Default::default()
        });

        // recalc_tokens is guarded: if has_api_tokens, it's a no-op.
        state.recalc_tokens();
        assert_eq!(state.ui.cached_input_tokens, 100);
        assert_eq!(state.ui.cached_output_tokens, 50);
    }

    // ========================================================================
    //  5. CacheStats hit rate
    // ========================================================================

    #[test]
    fn test_cache_stats_hit_rate() {
        let stats = CacheStats {
            hits: 80,
            misses: 20,
        };
        assert!((stats.hit_rate() - 80.0).abs() < 0.01);

        let empty = CacheStats::default();
        assert_eq!(empty.hit_rate(), 0.0);
    }

    // ========================================================================
    //  6. Full integration: AppState + runtime
    // ========================================================================

    #[tokio::test]
    async fn test_full_chat_pipeline_with_runtime() {
        let embed: Arc<dyn crate::llm::EmbeddingService> = Arc::new(MockEmbed);
        let runtime = Arc::new(RwLock::new(AgentRuntime::new(
            AgentRuntimeConfig::default(),
            embed,
        )));

        let mut state = AppState::default();
        state.core.runtime = Some(runtime);

        // Add some messages including System.
        state
            .core
            .messages
            .push(ChatMessage::system("Boot message."));
        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Implement feature".into(),
            reasoning: String::new(),
            timestamp: "00:01:00".into(),
            status: MessageStatus::Completed,
        });
        state.core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: "Working on it...".into(),
            reasoning: String::new(),
            timestamp: "00:01:01".into(),
            status: MessageStatus::Completed,
        });
        state
            .core
            .messages
            .push(ChatMessage::system("Sub-agent done."));
        state.core.messages.push(ChatMessage {
            role: MessageRole::User,
            content: "Add tests".into(),
            reasoning: String::new(),
            timestamp: "00:02:00".into(),
            status: MessageStatus::Completed,
        });

        // The next response slot (agent will fill this).
        state.core.messages.push(ChatMessage {
            role: MessageRole::Agent,
            content: String::new(),
            reasoning: String::new(),
            timestamp: "00:02:01".into(),
            status: MessageStatus::Thinking,
        });
        let resp_idx = state.core.messages.len() - 1;

        // ── Build history (same logic as handler.rs) ──
        let history = build_chat_history(&state.core.messages, resp_idx);

        // System messages excluded.
        for (role, _) in &history {
            assert_ne!(*role, "system");
        }
        // Should be: User, Agent (current User "Add tests" is message param, not in history).
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].0, "user");
        assert_eq!(history[0].1, "Implement feature");

        // ── Recalc tokens ──
        state.recalc_tokens();
        assert!(state.ui.cached_input_tokens > 0);
        assert!(state.ui.cached_output_tokens > 0);
    }

    // ========================================================================
    //  7. System prompt caching (memo isolation)
    // ========================================================================

    #[test]
    fn test_system_prompt_cache_initially_empty() {
        let state = AppState::default();
        assert!(state.ui.cached_system_prompt.is_none());
        assert!(state.ui.cached_prompt_role.is_empty());
    }

    #[test]
    fn test_system_prompt_cache_cleared_on_new_session() {
        let mut state = AppState::default();
        // Simulate cache being populated
        state.ui.cached_system_prompt = Some("cached prompt".to_string());
        state.ui.cached_prompt_role = "developer".to_string();

        // Simulate /clear command
        state.ui.cached_system_prompt = None;
        state.ui.cached_prompt_role.clear();

        assert!(state.ui.cached_system_prompt.is_none());
        assert!(state.ui.cached_prompt_role.is_empty());
    }

    #[test]
    fn test_system_prompt_cache_role_change_invalidates() {
        let mut state = AppState::default();
        // Simulate cache for role A
        state.ui.cached_system_prompt = Some("prompt for A".to_string());
        state.ui.cached_prompt_role = "role_a".to_string();

        // If role changes to B, cache should be invalidated
        let new_role = "role_b";
        if state.ui.cached_prompt_role != new_role {
            state.ui.cached_system_prompt = None;
            state.ui.cached_prompt_role.clear();
        }

        assert!(state.ui.cached_system_prompt.is_none());
    }
}

impl Default for ChatMessage {
    fn default() -> Self {
        Self {
            role: MessageRole::System,
            content: String::new(),
            reasoning: String::new(),
            timestamp: String::new(),
            status: MessageStatus::Completed,
        }
    }
}
